use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use horde_mesh::{MeshHost, MeshService, MeshStatus};
use horde_node::{EncryptedKeystore, IdentityKeys, PhysicalServer, ServerSnapshot};
use horde_protocol::{
    BootstrapPeer, CapabilityAdvert, FairShareConfig, InvitePurpose, InviteRecord,
    InviteRedeemRequest, InviteTokenPlain, JobCreateRequest, JobId, JobRecord, JobResult,
    JobRouting, JobStatus, LocalReputationLedger, MeshPeerInfo, PeerSoftScore, ProtocolError,
    ProtocolResult, PublicDhtConfig, ResourceProofChallenge, ResourceProofResponse,
    ResultDecision, ServerLimits, PROTOCOL_VERSION,
};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::scheduler::pick_server;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MintInviteRequest {
    pub purpose: InvitePurpose,
    /// Time-to-live in seconds (minimum 60).
    #[serde(default = "default_ttl")]
    pub ttl_secs: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_ttl() -> i64 {
    3600
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectiveStatus {
    pub protocol_version: String,
    pub mode: String,
    pub node_id: String,
    pub head_server_id: String,
    pub member_count: usize,
    pub job_count: usize,
    pub members: Vec<ServerSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshStatus>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MeshJoinRequest {
    pub peer_addr: String,
    pub token: String,
}

/// In-memory collective head (local + optional invite-bounded mesh).
pub struct CollectiveHead {
    identity: IdentityKeys,
    head_server_id: String,
    members: RwLock<HashMap<String, Arc<PhysicalServer>>>,
    invites: RwLock<HashMap<uuid::Uuid, InviteRecord>>,
    jobs: RwLock<HashMap<JobId, JobRecord>>,
    /// Studio API keys: raw key → label.
    api_keys: RwLock<HashMap<String, String>>,
    data_dir: PathBuf,
    mesh: RwLock<Option<Arc<MeshService>>>,
    /// Soft, non-transferable local reputation (Phase D). Local mode does not update it.
    reputation: RwLock<LocalReputationLedger>,
}

impl CollectiveHead {
    pub fn bootstrap(
        data_dir: PathBuf,
        passphrase: &str,
        hostname: impl Into<String>,
    ) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&data_dir)?;
        let keystore_path = data_dir.join("node.keystore.json");
        let identity = EncryptedKeystore::load_or_create(&keystore_path, passphrase)?;
        let hostname = hostname.into();
        let head_server_id = format!("srv-{}", identity.node_id.fingerprint());
        let worker_keys = EncryptedKeystore::unlock(&keystore_path, passphrase)?;
        let head_server = Arc::new(PhysicalServer::new(
            head_server_id.clone(),
            hostname,
            worker_keys,
        ));

        Ok(Arc::new(Self {
            identity,
            head_server_id: head_server_id.clone(),
            members: RwLock::new(HashMap::from([(head_server_id, head_server)])),
            invites: RwLock::new(HashMap::new()),
            jobs: RwLock::new(HashMap::new()),
            api_keys: RwLock::new(HashMap::new()),
            data_dir,
            mesh: RwLock::new(None),
            reputation: RwLock::new(LocalReputationLedger::new()),
        }))
    }

    /// Enable invite-bounded Noise mesh listener (admin API stays on its own localhost bind).
    pub async fn enable_mesh(self: &Arc<Self>, bind: SocketAddr) -> anyhow::Result<()> {
        // Network mode: head accepts mesh jobs by default (operator may tighten via limits API).
        if let Some(head) = self.members.read().await.get(&self.head_server_id).cloned() {
            let mut limits = head.limits().await;
            limits.accept_network_jobs = true;
            head.set_limits(limits).await?;
        }
        let host: Arc<dyn MeshHost> = Arc::clone(self) as Arc<dyn MeshHost>;
        let mesh = MeshService::new(self.identity.clone(), bind, host)?;
        mesh.start().await?;
        *self.mesh.write().await = Some(mesh);
        info!(%bind, "collective mesh enabled");
        Ok(())
    }

    pub async fn mesh_status(&self) -> Option<MeshStatus> {
        match self.mesh.read().await.as_ref() {
            Some(m) => Some(m.status().await),
            None => None,
        }
    }

    pub fn node_id(&self) -> String {
        self.identity.node_id.fingerprint()
    }

    pub fn identity_keys(&self) -> &IdentityKeys {
        &self.identity
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub async fn ensure_dev_api_key(&self) -> String {
        let mut keys = self.api_keys.write().await;
        if let Some((k, _)) = keys.iter().next() {
            return k.clone();
        }
        let key = format!("hk_dev_{}", random_token(24));
        keys.insert(key.clone(), "local-dev".into());
        key
    }

    pub async fn create_api_key(&self, label: impl Into<String>) -> String {
        let key = format!("hk_{}", random_token(32));
        self.api_keys
            .write()
            .await
            .insert(key.clone(), label.into());
        key
    }

    pub async fn auth_api_key(&self, key: &str) -> ProtocolResult<()> {
        if self.api_keys.read().await.contains_key(key) {
            Ok(())
        } else {
            Err(ProtocolError::AuthFailed)
        }
    }

    pub async fn status(&self) -> CollectiveStatus {
        let members = self.members.read().await;
        let mut snaps = Vec::with_capacity(members.len());
        for m in members.values() {
            snaps.push(m.snapshot().await);
        }
        snaps.sort_by(|a, b| a.server_id.cmp(&b.server_id));
        let mesh = self.mesh_status().await;
        let mode = if mesh.is_some() {
            "network".into()
        } else {
            "local".into()
        };
        CollectiveStatus {
            protocol_version: PROTOCOL_VERSION.to_string(),
            mode,
            node_id: self.node_id(),
            head_server_id: self.head_server_id.clone(),
            member_count: snaps.len(),
            job_count: self.jobs.read().await.len(),
            members: snaps,
            mesh,
        }
    }

    pub async fn list_servers(&self) -> Vec<ServerSnapshot> {
        self.status().await.members
    }

    pub async fn set_server_limits(
        &self,
        server_id: &str,
        limits: ServerLimits,
    ) -> ProtocolResult<ServerSnapshot> {
        let members = self.members.read().await;
        let server = members
            .get(server_id)
            .ok_or(ProtocolError::ServerNotFound)?;
        server.set_limits(limits).await?;
        Ok(server.snapshot().await)
    }

    pub async fn mint_invite(&self, req: MintInviteRequest) -> InviteTokenPlain {
        let plaintext = format!("horde_inv_{}", random_token(40));
        let (plain, record) = InviteRecord::mint(req.purpose, req.ttl_secs, plaintext, req.note);
        self.invites.write().await.insert(record.token_id, record);
        info!(token_id = %plain.token_id, "minted invite");
        plain
    }

    pub async fn redeem_invite(
        &self,
        req: InviteRedeemRequest,
    ) -> ProtocolResult<ServerSnapshot> {
        let mut invites = self.invites.write().await;
        let record = invites
            .values_mut()
            .find(|r| r.token_hash == InviteRecord::hash_token(&req.token))
            .ok_or(ProtocolError::InviteInvalid)?;
        if let Some(expected) = &req.purpose {
            if expected != &record.purpose {
                return Err(ProtocolError::InviteInvalid);
            }
        }
        record.verify_plaintext(&req.token)?;
        record.redeemed = true;
        record.redeemed_by = Some(req.claimant.clone());
        let purpose = record.purpose.clone();
        drop(invites);

        match purpose {
            InvitePurpose::CollectiveJoin => {
                let server_id = format!("srv-{}", random_token(8));
                let keys = IdentityKeys::generate();
                let server = Arc::new(PhysicalServer::new(server_id.clone(), req.claimant, keys));
                self.members
                    .write()
                    .await
                    .insert(server_id, Arc::clone(&server));
                info!(server = %server.server_id, "collective member joined via invite");
                Ok(server.snapshot().await)
            }
            InvitePurpose::MeshJoin => {
                // Prefer POST /v1/admin/mesh/join which dials with the token.
                // HTTP redeem still marks the invite used for operator bookkeeping.
                info!(claimant = %req.claimant, "mesh invite marked redeemed via HTTP");
                let head = self
                    .members
                    .read()
                    .await
                    .get(&self.head_server_id)
                    .cloned()
                    .ok_or_else(|| ProtocolError::Other("head missing".into()))?;
                Ok(head.snapshot().await)
            }
        }
    }

    pub async fn list_invites(&self) -> Vec<InviteRecord> {
        let mut v: Vec<_> = self.invites.read().await.values().cloned().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub async fn set_bootstrap_peers(&self, peers: Vec<BootstrapPeer>) -> ProtocolResult<()> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.set_bootstrap(peers).await;
        Ok(())
    }

    /// Dial a peer presenting a mesh invite (invite-bounded join).
    pub async fn mesh_join(&self, req: MeshJoinRequest) -> ProtocolResult<MeshPeerInfo> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.dial_with_invite(&req.peer_addr, Some(&req.token))
            .await
    }

    pub async fn block_peer(&self, node_id: &str) -> ProtocolResult<()> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.block_peer(node_id).await;
        Ok(())
    }

    pub async fn unblock_peer(&self, node_id: &str) -> ProtocolResult<bool> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        Ok(mesh.blocklist.unblock(node_id).await)
    }

    pub async fn blocklist(&self) -> ProtocolResult<Vec<String>> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        Ok(mesh.blocklist.snapshot().await.blocked_node_ids)
    }

    pub async fn dht_config(&self) -> ProtocolResult<PublicDhtConfig> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        Ok(mesh.dht_config().await)
    }

    pub async fn set_dht_config(&self, config: PublicDhtConfig) -> ProtocolResult<PublicDhtConfig> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.set_dht_config(config).await?;
        Ok(mesh.dht_config().await)
    }

    pub async fn dht_challenge(&self) -> ProtocolResult<ResourceProofChallenge> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.dht_challenge().await
    }

    pub async fn dht_lookup(
        &self,
        key: &str,
        proof: Option<ResourceProofResponse>,
    ) -> ProtocolResult<Vec<String>> {
        let mesh = self.mesh.read().await;
        let mesh = mesh.as_ref().ok_or_else(|| {
            ProtocolError::Other("mesh not enabled; set HORDE_MESH_BIND".into())
        })?;
        mesh.dht_lookup(key, proof).await
    }

    /// Snapshot of soft reputation scores (non-transferable; advisory only).
    pub async fn reputation_snapshot(&self) -> Vec<PeerSoftScore> {
        self.reputation.read().await.snapshot()
    }

    pub async fn fair_share_config(&self) -> FairShareConfig {
        self.reputation.read().await.config().clone()
    }

    pub async fn set_fair_share_config(&self, config: FairShareConfig) -> FairShareConfig {
        let mut ledger = self.reputation.write().await;
        ledger.set_config(config);
        ledger.config().clone()
    }

    /// Operator priority hint for an invite-trusted peer (0–100).
    pub async fn set_peer_priority_hint(
        &self,
        node_id: &str,
        hint: u8,
    ) -> ProtocolResult<PeerSoftScore> {
        let mut ledger = self.reputation.write().await;
        ledger.set_priority_hint(node_id, hint)?;
        Ok(ledger
            .get(node_id)
            .cloned()
            .unwrap_or_else(|| PeerSoftScore::new(node_id)))
    }

    pub async fn submit_and_schedule(
        self: &Arc<Self>,
        req: JobCreateRequest,
    ) -> ProtocolResult<JobRecord> {
        let routing = req.routing.clone();
        let job = JobRecord::new(req.clone())?;

        match routing {
            JobRouting::Local => self.schedule_local(job).await,
            JobRouting::Mesh => self.schedule_mesh(job, req).await,
            JobRouting::Auto => {
                let members: Vec<_> = self.members.read().await.values().cloned().collect();
                if pick_server(&members, &job).await.is_ok() {
                    self.schedule_local(job).await
                } else {
                    self.schedule_mesh(job, req).await
                }
            }
        }
    }

    async fn schedule_local(self: &Arc<Self>, mut job: JobRecord) -> ProtocolResult<JobRecord> {
        let members: Vec<_> = self.members.read().await.values().cloned().collect();
        let server = pick_server(&members, &job).await?;
        job.status = JobStatus::Scheduled;
        job.assigned_server_id = Some(server.server_id.clone());
        job.updated_at = chrono::Utc::now();
        self.jobs.write().await.insert(job.id, job.clone());

        let head = Arc::clone(self);
        let job_id = job.id;
        tokio::spawn(async move {
            let current = {
                let jobs = head.jobs.read().await;
                jobs.get(&job_id).cloned()
            };
            let Some(current) = current else {
                return;
            };
            if matches!(current.status, JobStatus::Cancelled) {
                return;
            }
            let finished = server.run_job(current).await;
            let mut jobs = head.jobs.write().await;
            if let Some(existing) = jobs.get(&job_id) {
                if matches!(existing.status, JobStatus::Cancelled) {
                    return;
                }
            }
            jobs.insert(job_id, finished);
        });

        Ok(job)
    }

    async fn schedule_mesh(
        self: &Arc<Self>,
        mut job: JobRecord,
        req: JobCreateRequest,
    ) -> ProtocolResult<JobRecord> {
        let mesh = self.mesh.read().await;
        let mesh = mesh
            .as_ref()
            .cloned()
            .ok_or_else(|| ProtocolError::Other("mesh not enabled".into()))?;
        let peer_id = mesh.find_capable_peer(&req).await?;
        job.status = JobStatus::Scheduled;
        job.assigned_peer_id = Some(peer_id.clone());
        job.updated_at = chrono::Utc::now();
        self.jobs.write().await.insert(job.id, job.clone());

        let head = Arc::clone(self);
        let job_id = job.id;
        tokio::spawn(async move {
            let current = {
                let jobs = head.jobs.read().await;
                jobs.get(&job_id).cloned()
            };
            let Some(current) = current else {
                return;
            };
            if matches!(current.status, JobStatus::Cancelled) {
                return;
            }
            {
                let mut jobs = head.jobs.write().await;
                if let Some(j) = jobs.get_mut(&job_id) {
                    j.status = JobStatus::Running;
                    j.updated_at = chrono::Utc::now();
                }
            }
            let offer_req = JobCreateRequest {
                fingerprint: current.fingerprint.clone(),
                kind: current.kind.clone(),
                payload: current.payload.clone(),
                requested_vram_mb: current.requested_vram_mb,
                routing: JobRouting::Mesh,
                priority_hint: current.priority_hint,
            };
            let outcome = mesh.offer_job(&peer_id, job_id, offer_req).await;
            // Soft reputation before re-taking the jobs lock (avoid lock-order deadlock).
            if let Ok(result) = &outcome {
                if result.attestation.model_fingerprint == current.fingerprint
                    && result.attestation.verify(&result.output).is_ok()
                {
                    head.reputation
                        .write()
                        .await
                        .record_inbound_success(&peer_id);
                }
            }
            let mut jobs = head.jobs.write().await;
            if let Some(existing) = jobs.get(&job_id) {
                if matches!(existing.status, JobStatus::Cancelled) {
                    return;
                }
            }
            match outcome {
                Ok(result) => {
                    // Enforce fingerprint pin on attested result.
                    if result.attestation.model_fingerprint != current.fingerprint {
                        if let Some(j) = jobs.get_mut(&job_id) {
                            j.status = JobStatus::Failed;
                            j.error = Some("attestation fingerprint mismatch".into());
                            j.updated_at = chrono::Utc::now();
                        }
                        return;
                    }
                    if let Err(err) = result.attestation.verify(&result.output) {
                        if let Some(j) = jobs.get_mut(&job_id) {
                            j.status = JobStatus::Failed;
                            j.error = Some(err.to_string());
                            j.updated_at = chrono::Utc::now();
                        }
                        return;
                    }
                    if let Some(j) = jobs.get_mut(&job_id) {
                        j.status = JobStatus::Succeeded;
                        j.result = Some(result);
                        j.error = None;
                        j.result_decision = ResultDecision::Pending;
                        j.updated_at = chrono::Utc::now();
                    }
                }
                Err(err) => {
                    if let Some(j) = jobs.get_mut(&job_id) {
                        j.status = JobStatus::Failed;
                        j.error = Some(err.to_string());
                        j.updated_at = chrono::Utc::now();
                    }
                }
            }
        });

        Ok(job)
    }

    pub async fn get_job(&self, id: &JobId) -> ProtocolResult<JobRecord> {
        self.jobs
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or(ProtocolError::JobNotFound)
    }

    pub async fn get_result(&self, id: &JobId) -> ProtocolResult<JobResult> {
        let job = self.get_job(id).await?;
        match job.status {
            JobStatus::Succeeded => job
                .result
                .ok_or_else(|| ProtocolError::Other("missing result".into())),
            other => Err(ProtocolError::Other(format!(
                "result not ready; status={other}"
            ))),
        }
    }

    pub async fn decide_result(
        &self,
        id: &JobId,
        decision: ResultDecision,
    ) -> ProtocolResult<JobRecord> {
        if matches!(decision, ResultDecision::Pending) {
            return Err(ProtocolError::Other(
                "decision must be accepted or rejected".into(),
            ));
        }
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(id).ok_or(ProtocolError::JobNotFound)?;
        if job.status != JobStatus::Succeeded {
            return Err(ProtocolError::Other(
                "can only accept/reject succeeded jobs".into(),
            ));
        }
        if let Some(result) = &job.result {
            result.attestation.verify(&result.output)?;
        }
        // Soft reputation updates only for mesh-assigned peers (local mode: no effect).
        let peer_for_rep = job.assigned_peer_id.clone();
        job.result_decision = decision.clone();
        job.updated_at = chrono::Utc::now();
        let updated = job.clone();
        drop(jobs);
        if let Some(peer_id) = peer_for_rep {
            let mut ledger = self.reputation.write().await;
            match decision {
                ResultDecision::Accepted => ledger.record_accept(&peer_id),
                ResultDecision::Rejected => ledger.record_reject(&peer_id),
                ResultDecision::Pending => {}
            }
        }
        info!(job_id = %id, ?decision, "requester result decision");
        Ok(updated)
    }

    pub async fn cancel_job(&self, id: &JobId) -> ProtocolResult<JobRecord> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(id).ok_or(ProtocolError::JobNotFound)?;
        if !job.status.is_cancellable() {
            return Err(ProtocolError::JobNotCancellable(job.status.to_string()));
        }
        job.status = JobStatus::Cancelled;
        job.updated_at = chrono::Utc::now();
        warn!(job_id = %id, "job cancelled");
        Ok(job.clone())
    }
}

#[async_trait]
impl MeshHost for CollectiveHead {
    async fn validate_mesh_invite(&self, token: &str) -> ProtocolResult<()> {
        let mut invites = self.invites.write().await;
        let record = invites
            .values_mut()
            .find(|r| r.token_hash == InviteRecord::hash_token(token))
            .ok_or(ProtocolError::InviteInvalid)?;
        if record.purpose != InvitePurpose::MeshJoin {
            return Err(ProtocolError::InviteInvalid);
        }
        record.verify_plaintext(token)?;
        record.redeemed = true;
        record.redeemed_by = Some("mesh-peer".into());
        Ok(())
    }

    async fn local_capability(&self) -> CapabilityAdvert {
        let members = self.members.read().await;
        if let Some(head) = members.get(&self.head_server_id) {
            return head.snapshot().await.capability;
        }
        CapabilityAdvert::stub_local(&self.head_server_id, "unknown")
    }

    async fn execute_mesh_job(&self, request: JobCreateRequest) -> ProtocolResult<JobResult> {
        let members: Vec<_> = self.members.read().await.values().cloned().collect();
        let mut eligible = Vec::new();
        for m in &members {
            let limits = m.limits().await;
            if limits.accept_network_jobs && limits.enabled {
                eligible.push(Arc::clone(m));
            }
        }
        if eligible.is_empty() {
            return Err(ProtocolError::LimitRejected(
                "no member accepts network jobs".into(),
            ));
        }
        let job = JobRecord::new(request)?;
        let server = pick_server(&eligible, &job).await?;
        let finished = server.run_job(job).await;
        match finished.status {
            JobStatus::Succeeded => finished
                .result
                .ok_or_else(|| ProtocolError::Other("missing result".into())),
            _ => Err(ProtocolError::Other(
                finished
                    .error
                    .unwrap_or_else(|| "mesh job failed".into()),
            )),
        }
    }

    async fn note_outbound_work(&self, peer_id: &str) {
        self.reputation
            .write()
            .await
            .record_outbound_success(peer_id);
    }

    async fn should_decline_consumer(&self, peer_id: &str) -> bool {
        self.reputation.read().await.should_ignore_consumer(peer_id)
    }

    async fn rank_mesh_peers(
        &self,
        peer_ids: &[String],
        job_priority_hint: Option<u8>,
    ) -> Vec<String> {
        self.reputation
            .read()
            .await
            .rank_peers(peer_ids, job_priority_hint)
    }
}

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horde_protocol::{ModelFingerprint, WorkKind};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("horde-collective-{nanos}"))
    }

    #[tokio::test]
    async fn local_job_round_trip() {
        let dir = tmp_dir();
        let head = CollectiveHead::bootstrap(dir.clone(), "pass", "test-host").unwrap();
        let _ = head.ensure_dev_api_key().await;
        let job = head
            .submit_and_schedule(JobCreateRequest {
                fingerprint: ModelFingerprint {
                    model_id: "stub/echo".into(),
                    revision_hash: "deadbeefcafebabe".into(),
                    tokenizer_revision: None,
                    precision: "fp16".into(),
                },
                kind: WorkKind::StubEcho,
                payload: "ping".into(),
                requested_vram_mb: None,
                routing: JobRouting::Local,
                priority_hint: None,
            })
            .await
            .unwrap();

        for _ in 0..50 {
            let j = head.get_job(&job.id).await.unwrap();
            if j.status.is_terminal() {
                assert_eq!(j.status, JobStatus::Succeeded);
                let result = head.get_result(&job.id).await.unwrap();
                assert!(result.output.contains("ping"));
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("job did not finish");
    }

    #[tokio::test]
    async fn invite_join_adds_member() {
        let dir = tmp_dir();
        let head = CollectiveHead::bootstrap(dir.clone(), "pass", "test-host").unwrap();
        let plain = head
            .mint_invite(MintInviteRequest {
                purpose: InvitePurpose::CollectiveJoin,
                ttl_secs: 600,
                note: Some("lab pc".into()),
            })
            .await;
        let snap = head
            .redeem_invite(InviteRedeemRequest {
                token: plain.token,
                claimant: "other-pc".into(),
                purpose: Some(InvitePurpose::CollectiveJoin),
            })
            .await
            .unwrap();
        assert_eq!(snap.hostname, "other-pc");
        assert_eq!(head.list_servers().await.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn tokenize_job_chunks_under_fingerprint() {
        let dir = tmp_dir();
        let head = CollectiveHead::bootstrap(dir.clone(), "pass", "test-host").unwrap();
        let payload = serde_json::json!({
            "documents": ["alpha one", "beta two", "gamma three", "delta four"],
            "chunk_size": 2,
            "seed": 3
        })
        .to_string();
        let job = head
            .submit_and_schedule(JobCreateRequest {
                fingerprint: ModelFingerprint {
                    model_id: "stub/tokenizer".into(),
                    revision_hash: "deadbeefcafebabe".into(),
                    tokenizer_revision: Some("tok-rev-1".into()),
                    precision: "fp16".into(),
                },
                kind: WorkKind::Tokenize,
                payload,
                requested_vram_mb: None,
                routing: JobRouting::Local,
                priority_hint: None,
            })
            .await
            .unwrap();

        for _ in 0..50 {
            let j = head.get_job(&job.id).await.unwrap();
            if j.status.is_terminal() {
                assert_eq!(j.status, JobStatus::Succeeded);
                let result = head.get_result(&job.id).await.unwrap();
                let parsed: serde_json::Value =
                    serde_json::from_str(&result.output).expect("tokenize json");
                assert_eq!(parsed["chunk_count"], 2);
                assert_eq!(parsed["document_count"], 4);
                assert!(parsed["pipeline_checksum_hex"].as_str().unwrap().len() == 64);
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("tokenize job did not finish");
    }

    #[tokio::test]
    async fn soft_reputation_accept_updates_ledger() {
        let dir = tmp_dir();
        let head = CollectiveHead::bootstrap(dir.clone(), "pass", "test-host").unwrap();
        // Simulate a mesh peer completion without a live mesh: record + decide via ledger API.
        head.reputation
            .write()
            .await
            .record_inbound_success("peer-abc");
        head.set_peer_priority_hint("peer-abc", 80).await.unwrap();
        head.reputation.write().await.record_accept("peer-abc");
        let snap = head.reputation_snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].node_id, "peer-abc");
        assert_eq!(snap[0].accepted_results, 1);
        assert_eq!(snap[0].priority_hint, 80);
        assert!(snap[0].fair_share_weight > 0.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
