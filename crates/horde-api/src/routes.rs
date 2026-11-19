use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use horde_collective::{MeshJoinRequest, MintInviteRequest};
use horde_node::ServerSnapshot;
use horde_protocol::{
    support_matrix, BootstrapPeer, FairShareConfig, InviteRecord, InviteRedeemRequest,
    InviteTokenPlain, JobCreateRequest, JobId, JobResult, JobStatusResponse, MeshPeerInfo,
    PeerSoftScore, PublicDhtConfig, ResourceProofResponse, ResultDecision,
    ServerLimits, PROTOCOL_VERSION,
};
use serde::Deserialize;
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
        .route("/v1/admin/config", get(admin_config))
        .route("/v1/admin/servers", get(list_servers))
        .route("/v1/admin/servers/:server_id/limits", post(set_limits))
        .route("/v1/admin/invites", get(list_invites).post(mint_invite))
        .route("/v1/admin/invites/redeem", post(redeem_invite))
        .route("/v1/admin/api-keys", post(create_api_key))
        .route("/v1/admin/drain", get(drain_status).post(begin_drain))
        .route("/v1/admin/diagnostics/join", post(join_diagnostics))
        .route("/v1/admin/export", get(export_snapshot))
        // Mesh admin (localhost only — not on public overlay)
        .route("/v1/admin/mesh/status", get(mesh_status))
        .route("/v1/admin/mesh/health", get(mesh_health))
        .route("/v1/admin/mesh/bind", post(set_mesh_bind))
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
        // Soft reputation / fair-share (Phase D; non-transferable, no credits)
        .route(
            "/v1/admin/reputation",
            get(get_reputation).post(set_fair_share),
        )
        .route(
            "/v1/admin/reputation/:node_id/priority",
            post(set_peer_priority),
        )
        // Studio
        .route("/v1/jobs", get(list_jobs).post(create_job))
        .route("/v1/jobs/:job_id", get(get_job))
        .route("/v1/jobs/:job_id/timeline", get(get_timeline))
        .route("/v1/jobs/:job_id/result", get(get_result))
        .route("/v1/jobs/:job_id/cancel", post(cancel_job))
        .route("/v1/jobs/:job_id/accept", post(accept_result))
        .route("/v1/jobs/:job_id/reject", post(reject_result))
        .with_state(state)
}

fn encode_join_uri(mesh_addr: &str, token: &str, expires_at: DateTime<Utc>) -> String {
    let addr = urlencoding_minimal(mesh_addr);
    let tok = urlencoding_minimal(token);
    let exp = urlencoding_minimal(&expires_at.to_rfc3339());
    format!("horde://join?addr={addr}&token={tok}&expires={exp}")
}

fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
        "phase": "D",
        "mode": mode,
        "protocol_version": PROTOCOL_VERSION,
        "node_id": state.head.node_id(),
        "mesh": mesh,
        "public_dht_enabled": public_dht_enabled,
        "soft_reputation": true,
        "product": "P2P GPU compute substrate (invite-bounded by default; soft reputation/fair-share; public DHT opt-in stub); no built-in AI studio; no cryptocurrency",
    }))
}

async fn admin_status(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<horde_collective::CollectiveStatus> {
    Json(state.head.status().await)
}

/// Redacted bind / data-dir / mesh flags for the Host panel (never passphrase).
async fn admin_config(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<crate::PublicRuntimeConfig> {
    let mut cfg = state.public_config.read().await.clone();
    // Reflect live mesh listener (runtime rebind / ephemeral ports).
    if let Some(addr) = state.head.mesh_bind_addr().await {
        cfg.mesh_bind = Some(addr.to_string());
        cfg.mesh_enabled = true;
    } else {
        cfg.mesh_enabled = false;
    }
    Json(cfg)
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
) -> Json<Value> {
    let plain: InviteTokenPlain = state.head.mint_invite(req).await;
    let mesh_addr = state.head.mesh_bind_addr().await.map(|a| a.to_string());
    let join_uri = mesh_addr
        .as_ref()
        .map(|addr| encode_join_uri(addr, &plain.token, plain.expires_at));
    Json(json!({
        "token_id": plain.token_id,
        "token": plain.token,
        "purpose": plain.purpose,
        "expires_at": plain.expires_at,
        "join_uri": join_uri,
        "mesh_addr": mesh_addr,
        "note": "Plaintext token shown once. Paste join_uri / scan QR in the operator UI. No protocol crypto change."
    }))
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
            "mesh not enabled; set HORDE_MESH_BIND or POST /v1/admin/mesh/bind",
        )),
    }
}

#[derive(serde::Deserialize)]
struct MeshBindBody {
    /// Socket address for the mesh listener, e.g. `127.0.0.1:0` or `0.0.0.0:9100`.
    bind: String,
}

/// Enable or rebind the mesh listener at runtime (no process restart).
///
/// Updates redacted `GET /v1/admin/config` mesh fields. Never accepts a passphrase.
async fn set_mesh_bind(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(body): Json<MeshBindBody>,
) -> ApiResult<Json<Value>> {
    let bind: std::net::SocketAddr = body.bind.parse().map_err(|e| {
        crate::error::ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid mesh bind address: {e}"),
        )
    })?;
    let actual = state.head.set_mesh_bind(bind).await.map_err(|e| {
        crate::error::ApiError::new(StatusCode::BAD_REQUEST, e.to_string())
    })?;
    {
        let mut cfg = state.public_config.write().await;
        cfg.mesh_bind = Some(actual.to_string());
        cfg.mesh_enabled = true;
    }
    Ok(Json(json!({
        "ok": true,
        "mesh_bind": actual.to_string(),
        "mesh_enabled": true,
        "rebind": "runtime",
        "note": "Mesh accept socket rebound in-process; existing peer sessions are retained"
    })))
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

async fn get_reputation(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<Value> {
    let peers = state.head.reputation_snapshot().await;
    let config = state.head.fair_share_config().await;
    Json(json!({
        "peers": peers,
        "fair_share": config,
        "note": "Soft, non-transferable local reputation only; not credits or currency"
    }))
}

async fn set_fair_share(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Json(config): Json<FairShareConfig>,
) -> Json<Value> {
    let updated = state.head.set_fair_share_config(config).await;
    Json(json!({ "fair_share": updated }))
}

#[derive(serde::Deserialize)]
struct PriorityBody {
    priority_hint: u8,
}

async fn set_peer_priority(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(node_id): Path<String>,
    Json(body): Json<PriorityBody>,
) -> ApiResult<Json<PeerSoftScore>> {
    Ok(Json(
        state
            .head
            .set_peer_priority_hint(&node_id, body.priority_hint)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ListJobsQuery {
    #[serde(default = "default_job_limit")]
    limit: usize,
}

fn default_job_limit() -> usize {
    50
}

async fn list_jobs(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Query(q): Query<ListJobsQuery>,
) -> Json<Vec<JobStatusResponse>> {
    let jobs = state.head.list_jobs(q.limit).await;
    Json(jobs.iter().map(JobStatusResponse::summary).collect())
}

async fn get_timeline(
    State(state): State<AppState>,
    _auth: StudioAuth,
    Path(job_id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let events = state.head.job_timeline(&JobId(job_id)).await?;
    Ok(Json(json!({
        "job_id": job_id,
        "timeline": events,
    })))
}

async fn drain_status(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<horde_collective::DrainStatus> {
    Json(state.head.drain_status().await)
}

async fn begin_drain(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<horde_collective::DrainStatus> {
    Json(state.head.begin_drain().await)
}

#[derive(Deserialize)]
struct JoinDiagBody {
    #[serde(default)]
    peer_addr: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    client_time: Option<DateTime<Utc>>,
}

async fn join_diagnostics(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<JoinDiagBody>,
) -> Json<horde_collective::JoinDiagnostics> {
    let _ = auth;
    Json(
        state
            .head
            .join_diagnostics(
                true, // AdminAuth already validated the key
                body.peer_addr.as_deref(),
                body.token.as_deref(),
                body.client_time,
            )
            .await,
    )
}

async fn export_snapshot(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> Json<Value> {
    Json(state.head.export_snapshot().await)
}

async fn mesh_health(
    State(state): State<AppState>,
    _auth: AdminAuth,
) -> ApiResult<Json<Value>> {
    let mesh = state.head.mesh_status().await.ok_or_else(|| {
        crate::error::ApiError::new(
            StatusCode::BAD_REQUEST,
            "mesh not enabled; set HORDE_MESH_BIND or POST /v1/admin/mesh/bind",
        )
    })?;
    let reputation = state.head.reputation_snapshot().await;
    let rep_by_id: std::collections::HashMap<_, _> = reputation
        .into_iter()
        .map(|p| (p.node_id.clone(), p))
        .collect();
    let peers: Vec<Value> = mesh
        .peers
        .iter()
        .map(|p| {
            let score = rep_by_id.get(&p.node_id);
            let accepted = score.map(|s| s.accepted_results).unwrap_or(0);
            let rejected = score.map(|s| s.rejected_results).unwrap_or(0);
            let total = accepted + rejected;
            let accept_rate = if total == 0 {
                None
            } else {
                Some((accepted as f64) / (total as f64))
            };
            let reject_rate = if total == 0 {
                None
            } else {
                Some((rejected as f64) / (total as f64))
            };
            json!({
                "node_id": p.node_id,
                "addr": p.addr,
                "connected": p.connected,
                "last_seen": p.last_seen,
                "rtt_ms": p.rtt_ms,
                "accepted_results": accepted,
                "rejected_results": rejected,
                "accept_rate": accept_rate,
                "reject_rate": reject_rate,
                "fair_share_weight": score.map(|s| s.fair_share_weight),
                "ignored_as_consumer": score.map(|s| s.ignored_as_consumer).unwrap_or(false),
            })
        })
        .collect();
    Ok(Json(json!({
        "bind": mesh.bind,
        "peer_count": mesh.peer_count,
        "peers": peers,
        "note": "Read-only mesh health strip from peer sessions + soft reputation"
    })))
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
