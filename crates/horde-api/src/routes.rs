use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use horde_collective::{MeshJoinRequest, MintInviteRequest};
use horde_node::ServerSnapshot;
use horde_protocol::{
    support_matrix, BootstrapPeer, InviteRecord, InviteRedeemRequest, InviteTokenPlain,
    JobCreateRequest, JobId, JobResult, JobStatusResponse, MeshPeerInfo, PublicDhtConfig,
    ResourceProofResponse, ResultDecision, ServerLimits, PROTOCOL_VERSION,
};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::{AdminAuth, StudioAuth};
use crate::error::ApiResult;
use crate::AppState;

pub fn api_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/meta", get(meta))
        // Admin
        .route("/v1/admin/status", get(admin_status))
        .route("/v1/admin/servers", get(list_servers))
        .route("/v1/admin/servers/:server_id/limits", post(set_limits))
        .route("/v1/admin/invites", get(list_invites).post(mint_invite))
        .route("/v1/admin/invites/redeem", post(redeem_invite))
        .route("/v1/admin/api-keys", post(create_api_key))
        // Mesh admin (localhost only — not on public overlay)
        .route("/v1/admin/mesh/status", get(mesh_status))
        .route("/v1/admin/mesh/bootstrap", post(set_bootstrap))
        .route("/v1/admin/mesh/join", post(mesh_join))
        .route("/v1/admin/mesh/blocklist", get(get_blocklist).post(block_peer))
        .route(
            "/v1/admin/mesh/blocklist/:node_id",
            delete(unblock_peer),
        )
        .route("/v1/admin/mesh/dht", get(dht_status).post(set_dht))
        .route("/v1/admin/mesh/dht/challenge", post(dht_challenge))
        .route("/v1/admin/mesh/dht/lookup", post(dht_lookup))
        .route("/v1/admin/support-matrix", get(get_support_matrix))
        // Studio
        .route("/v1/jobs", post(create_job))
        .route("/v1/jobs/:job_id", get(get_job))
        .route("/v1/jobs/:job_id/result", get(get_result))
        .route("/v1/jobs/:job_id/cancel", post(cancel_job))
        .route("/v1/jobs/:job_id/accept", post(accept_result))
        .route("/v1/jobs/:job_id/reject", post(reject_result))
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    protocol_version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        protocol_version: PROTOCOL_VERSION,
    })
}

async fn meta(State(state): State<AppState>) -> Json<Value> {
    let mesh = state.head.mesh_status().await;
    let mode = if mesh.is_some() { "network" } else { "local" };
    let public_dht_enabled = mesh
        .as_ref()
        .map(|m| m.public_dht.enabled)
        .unwrap_or(false);
    Json(json!({
        "name": "horde",
        "phase": "C",
        "mode": mode,
        "protocol_version": PROTOCOL_VERSION,
        "node_id": state.head.node_id(),
        "mesh": mesh,
        "public_dht_enabled": public_dht_enabled,
        "product": "P2P GPU compute substrate (invite-bounded by default; public DHT opt-in stub); no built-in AI studio; no cryptocurrency",
    }))
}

async fn admin_status(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<horde_collective::CollectiveStatus> {
    Json(state.head.status().await)
}

async fn list_servers(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<Vec<ServerSnapshot>> {
    Json(state.head.list_servers().await)
}

async fn set_limits(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(server_id): Path<String>,
    Json(limits): Json<ServerLimits>,
) -> ApiResult<Json<ServerSnapshot>> {
    let snap = state.head.set_server_limits(&server_id, limits).await?;
    Ok(Json(snap))
}

async fn mint_invite(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(req): Json<MintInviteRequest>,
) -> Json<InviteTokenPlain> {
    Json(state.head.mint_invite(req).await)
}

async fn list_invites(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<Vec<InviteRecord>> {
    Json(state.head.list_invites().await)
}

async fn redeem_invite(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(req): Json<InviteRedeemRequest>,
) -> ApiResult<Json<ServerSnapshot>> {
    Ok(Json(state.head.redeem_invite(req).await?))
}

#[derive(Serialize)]
struct ApiKeyCreated {
    api_key: String,
    label: String,
}

#[derive(serde::Deserialize)]
struct CreateKeyBody {
    label: String,
}

async fn create_api_key(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<CreateKeyBody>,
) -> Json<ApiKeyCreated> {
    let api_key = state.head.create_api_key(&body.label).await;
    Json(ApiKeyCreated {
        api_key,
        label: body.label,
    })
}

async fn mesh_status(State(state): State<AppState>, _auth: AdminAuth) -> ApiResult<Json<Value>> {
    match state.head.mesh_status().await {
        Some(status) => Ok(Json(serde_json::to_value(status).unwrap_or(json!({})))),
        None => Err(crate::error::ApiError::new(
            StatusCode::BAD_REQUEST,
            "mesh not enabled; set HORDE_MESH_BIND",
        )),
    }
}

#[derive(serde::Deserialize)]
struct BootstrapBody {
    peers: Vec<BootstrapPeer>,
}

async fn set_bootstrap(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<BootstrapBody>,
) -> ApiResult<Json<Value>> {
    state.head.set_bootstrap_peers(body.peers).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn mesh_join(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(req): Json<MeshJoinRequest>,
) -> ApiResult<Json<MeshPeerInfo>> {
    Ok(Json(state.head.mesh_join(req).await?))
}

#[derive(serde::Deserialize)]
struct BlockBody {
    node_id: String,
}

async fn get_blocklist(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> ApiResult<Json<Value>> {
    let blocked = state.head.blocklist().await?;
    Ok(Json(json!({ "blocked_node_ids": blocked })))
}

async fn block_peer(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<BlockBody>,
) -> ApiResult<Json<Value>> {
    state.head.block_peer(&body.node_id).await?;
    Ok(Json(json!({ "blocked": body.node_id })))
}

async fn unblock_peer(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(node_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let removed = state.head.unblock_peer(&node_id).await?;
    Ok(Json(json!({ "unblocked": removed, "node_id": node_id })))
}

async fn get_support_matrix(
    State(_state): State<AppState>,
    _auth: AdminAuth,
) -> Json<Value> {
    Json(json!({ "workloads": support_matrix() }))
}

async fn dht_status(State(state): State<AppState>, _auth: AdminAuth) -> ApiResult<Json<Value>> {
    let cfg = state.head.dht_config().await?;
    Ok(Json(json!({
        "public_dht": cfg,
        "default_join": "invite_only",
        "note": "Public DHT is an explicit opt-in stub; invite-bounded mesh remains the default"
    })))
}

async fn set_dht(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(cfg): Json<PublicDhtConfig>,
) -> ApiResult<Json<Value>> {
    let updated = state.head.set_dht_config(cfg).await?;
    Ok(Json(json!({ "public_dht": updated })))
}

async fn dht_challenge(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> ApiResult<Json<Value>> {
    let challenge = state.head.dht_challenge().await?;
    Ok(Json(serde_json::to_value(challenge).unwrap_or(json!({}))))
}

#[derive(serde::Deserialize)]
struct DhtLookupBody {
    key: String,
    #[serde(default)]
    proof: Option<ResourceProofResponse>,
}

async fn dht_lookup(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<DhtLookupBody>,
) -> ApiResult<Json<Value>> {
    let peers = state.head.dht_lookup(&body.key, body.proof).await?;
    Ok(Json(json!({ "peers": peers })))
}

async fn create_job(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Json(req): Json<JobCreateRequest>,
) -> ApiResult<(StatusCode, Json<JobStatusResponse>)> {
    let job = state.head.submit_and_schedule(req).await?;
    Ok((StatusCode::ACCEPTED, Json(JobStatusResponse::from(&job))))
}

async fn get_job(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<JobStatusResponse>> {
    let job = state.head.get_job(&JobId(job_id)).await?;
    Ok(Json(JobStatusResponse::from(&job)))
}

async fn get_result(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<JobResult>> {
    Ok(Json(state.head.get_result(&JobId(job_id)).await?))
}

async fn cancel_job(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<JobStatusResponse>> {
    let job = state.head.cancel_job(&JobId(job_id)).await?;
    Ok(Json(JobStatusResponse::from(&job)))
}

async fn accept_result(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<JobStatusResponse>> {
    let job = state
        .head
        .decide_result(&JobId(job_id), ResultDecision::Accepted)
        .await?;
    Ok(Json(JobStatusResponse::from(&job)))
}

async fn reject_result(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<JobStatusResponse>> {
    let job = state
        .head
        .decide_result(&JobId(job_id), ResultDecision::Rejected)
        .await?;
    Ok(Json(JobStatusResponse::from(&job)))
}
