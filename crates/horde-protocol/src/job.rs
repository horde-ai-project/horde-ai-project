use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::NodeId;
use crate::ProtocolError;

/// Opaque job identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Pinned model identity required on every job (safety lock §1.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFingerprint {
    pub model_id: String,
    pub revision_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_revision: Option<String>,
    #[serde(default = "default_precision")]
    pub precision: String,
}

fn default_precision() -> String {
    "fp16".to_string()
}

impl ModelFingerprint {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.model_id.trim().is_empty() {
            return Err(ProtocolError::InvalidFingerprint(
                "model_id must not be empty".into(),
            ));
        }
        if self.revision_hash.trim().is_empty() {
            return Err(ProtocolError::InvalidFingerprint(
                "revision_hash must not be empty".into(),
            ));
        }
        if self.revision_hash.len() < 8 {
            return Err(ProtocolError::InvalidFingerprint(
                "revision_hash too short".into(),
            ));
        }
        Ok(())
    }

    pub fn compact(&self) -> String {
        format!("{}@{}", self.model_id, &self.revision_hash[..8.min(self.revision_hash.len())])
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    /// Whole-unit generative text (Phase A default).
    TextGenerate,
    /// Stub echo workload for CI / no-GPU environments.
    StubEcho,
    /// Shardable dataset tokenisation under one pinned fingerprint (Phase C).
    Tokenize,
    /// Shardable batch embedding under one pinned fingerprint (Phase C).
    Embed,
    /// Whole-unit video generation — hard resource gates only (Phase C).
    VideoGenerate,
}

impl Default for WorkKind {
    fn default() -> Self {
        Self::StubEcho
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Scheduled,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Scheduled => "scheduled",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        };
        write!(f, "{s}")
    }
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    pub fn is_cancellable(&self) -> bool {
        matches!(self, Self::Queued | Self::Scheduled | Self::Running)
    }
}

/// Where the collective should try to run a job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobRouting {
    /// Only members of this collective.
    Local,
    /// Only invited mesh peers (cross-node).
    Mesh,
    /// Prefer local; fall back to mesh if no local capacity.
    Auto,
}

impl Default for JobRouting {
    fn default() -> Self {
        Self::Local
    }
}

/// Requester decision on a succeeded network/local result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultDecision {
    Pending,
    Accepted,
    Rejected,
}

impl Default for ResultDecision {
    fn default() -> Self {
        Self::Pending
    }
}

/// Studio-facing create-job body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobCreateRequest {
    pub fingerprint: ModelFingerprint,
    #[serde(default)]
    pub kind: WorkKind,
    /// Opaque prompt / payload JSON string (Phase A keeps this simple).
    pub payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_vram_mb: Option<u32>,
    #[serde(default)]
    pub routing: JobRouting,
}

/// Worker-signed attestation over a result (audit, not payment).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultAttestation {
    pub worker_node_id: NodeId,
    pub model_fingerprint: ModelFingerprint,
    pub runtime: String,
    pub wall_time_ms: u64,
    /// Hex-encoded Ed25519 signature over canonical attestation bytes.
    pub signature_hex: String,
}

impl ResultAttestation {
    pub fn digest_bytes(
        worker_fingerprint: &str,
        fingerprint: &ModelFingerprint,
        runtime: &str,
        wall_time_ms: u64,
        output: &str,
    ) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(worker_fingerprint.as_bytes());
        hasher.update(fingerprint.compact().as_bytes());
        hasher.update(runtime.as_bytes());
        hasher.update(wall_time_ms.to_le_bytes());
        hasher.update(output.as_bytes());
        hasher.finalize().into()
    }

    /// Verify Ed25519 signature using the worker public key embedded in `worker_node_id`.
    pub fn verify(&self, output: &str) -> Result<(), ProtocolError> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&self.worker_node_id.0 .0)
            .map_err(|e| ProtocolError::AttestationInvalid(e.to_string()))?;
        let sig_bytes = hex::decode(&self.signature_hex)
            .map_err(|e| ProtocolError::AttestationInvalid(e.to_string()))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|e| ProtocolError::AttestationInvalid(e.to_string()))?;
        let digest = Self::digest_bytes(
            &self.worker_node_id.fingerprint(),
            &self.model_fingerprint,
            &self.runtime,
            self.wall_time_ms,
            output,
        );
        vk.verify(&digest, &sig)
            .map_err(|_| ProtocolError::AttestationInvalid("bad signature".into()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobResult {
    pub output: String,
    pub attestation: ResultAttestation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: JobId,
    pub status: JobStatus,
    pub fingerprint: ModelFingerprint,
    pub kind: WorkKind,
    pub payload: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_peer_id: Option<String>,
    #[serde(default)]
    pub routing: JobRouting,
    #[serde(default)]
    pub result_decision: ResultDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<JobResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_vram_mb: Option<u32>,
}

impl JobRecord {
    pub fn new(req: JobCreateRequest) -> Result<Self, ProtocolError> {
        req.fingerprint.validate()?;
        validate_work_kind(&req.kind, &req.fingerprint, &req.payload)?;
        let now = Utc::now();
        Ok(Self {
            id: JobId::new(),
            status: JobStatus::Queued,
            fingerprint: req.fingerprint,
            kind: req.kind,
            payload: req.payload,
            created_at: now,
            updated_at: now,
            assigned_server_id: None,
            assigned_peer_id: None,
            routing: req.routing,
            result_decision: ResultDecision::Pending,
            error: None,
            result: None,
            requested_vram_mb: req.requested_vram_mb,
        })
    }
}

/// Fingerprint / payload rules that apply before scheduling.
pub fn validate_work_kind(
    kind: &WorkKind,
    fingerprint: &ModelFingerprint,
    payload: &str,
) -> Result<(), ProtocolError> {
    match kind {
        WorkKind::Tokenize | WorkKind::Embed => {
            if fingerprint
                .tokenizer_revision
                .as_ref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                return Err(ProtocolError::InvalidFingerprint(
                    "tokenizer_revision required for tokenize/embed jobs".into(),
                ));
            }
            // Ensure payload parses when kind is tokenize; embed accepts same JSON shape or raw.
            if matches!(kind, WorkKind::Tokenize) {
                let _ = crate::tokenize::TokenizeRequest::from_job_payload(payload)?;
            }
        }
        WorkKind::VideoGenerate => {
            // Video is whole-unit and must declare VRAM need via requested_vram_mb at schedule time;
            // payload non-empty check only here.
            if payload.trim().is_empty() {
                return Err(ProtocolError::Other(
                    "video_generate payload must not be empty".into(),
                ));
            }
        }
        WorkKind::StubEcho | WorkKind::TextGenerate => {}
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobStatusResponse {
    pub id: JobId,
    pub status: JobStatus,
    pub fingerprint: ModelFingerprint,
    pub kind: WorkKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_peer_id: Option<String>,
    #[serde(default)]
    pub routing: JobRouting,
    #[serde(default)]
    pub result_decision: ResultDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<&JobRecord> for JobStatusResponse {
    fn from(job: &JobRecord) -> Self {
        Self {
            id: job.id,
            status: job.status.clone(),
            fingerprint: job.fingerprint.clone(),
            kind: job.kind.clone(),
            created_at: job.created_at,
            updated_at: job.updated_at,
            assigned_server_id: job.assigned_server_id.clone(),
            assigned_peer_id: job.assigned_peer_id.clone(),
            routing: job.routing.clone(),
            result_decision: job.result_decision.clone(),
            error: job.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_rejects_empty() {
        let fp = ModelFingerprint {
            model_id: "".into(),
            revision_hash: "abcd1234".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        };
        assert!(fp.validate().is_err());
    }

    #[test]
    fn job_starts_queued() {
        let job = JobRecord::new(JobCreateRequest {
            fingerprint: ModelFingerprint {
                model_id: "stub/echo".into(),
                revision_hash: "deadbeefcafe".into(),
                tokenizer_revision: None,
                precision: "fp16".into(),
            },
            kind: WorkKind::StubEcho,
            payload: "hello".into(),
            requested_vram_mb: None,
            routing: JobRouting::Local,
        })
        .unwrap();
        assert_eq!(job.status, JobStatus::Queued);
    }
}
