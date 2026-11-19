//! Shared types for Horde: jobs, invites, capabilities, mesh messages, and admin limits.
//!
//! Invariants:
//! - Generative jobs are whole-unit; model fingerprints are required on every job.
//! - Tokenisation/embed jobs may chunk under one pinned fingerprint (Phase C).
//! - Invite tokens are opaque secrets hashed at rest; plaintext is shown once at mint.
//! - Mesh join is invite-bounded by default; public DHT is explicit opt-in only.
//! - Soft reputation / fair-share is local and non-transferable (Phase D).
//! - No cryptocurrency or spendable credits types exist in this crate.

mod capability;
mod dht;
mod error;
mod identity;
mod invite;
mod job;
mod limits;
mod mesh;
mod reputation;
mod support;
mod tokenize;

pub use capability::{BackendKind, CapabilityAdvert};
pub use dht::{
    DhtRateLimiter, PublicDhtConfig, PublicDhtStub, ResourceProofChallenge, ResourceProofResponse,
    DEFAULT_DHT_QUERIES_PER_MINUTE,
};
pub use error::{ProtocolError, ProtocolResult};
pub use identity::{NodeId, PublicKeyBytes};
pub use invite::{InvitePurpose, InviteRecord, InviteRedeemRequest, InviteTokenPlain};
pub use job::{
    validate_work_kind, JobCreateRequest, JobId, JobRecord, JobResult, JobRouting, JobStatus,
    JobStatusResponse, JobTimelineEvent, ModelFingerprint, ResultAttestation, ResultDecision,
    WorkKind,
};
pub use limits::{ServerLimits, TimeWindow};
pub use mesh::{BootstrapPeer, MeshMessage, MeshPeerInfo};
pub use reputation::{
    validate_priority_hint, FairShareConfig, LocalReputationLedger, PeerSoftScore,
    DEFAULT_CONSUMER_RATIO, DEFAULT_MIN_SAMPLES, DEFAULT_PRIORITY_HINT, MAX_PRIORITY_HINT,
};
pub use support::{
    check_resource_gate, support_for, support_matrix, WorkSupport, VIDEO_MIN_VRAM_MB,
};
pub use tokenize::{
    merge_chunk_outputs, plan_chunks, run_stub_tokenize_pipeline, stub_tokenize_chunk,
    TokenizeChunk, TokenizeChunkOutput, TokenizePipelineResult, TokenizeRequest,
    DEFAULT_CHUNK_SIZE, MAX_DOCUMENT_CHARS, MAX_TOKENIZE_DOCUMENTS,
};

/// Protocol semver advertised by nodes (decline on major mismatch in later phases).
pub const PROTOCOL_VERSION: &str = "0.4.0";

/// Default localhost bind for the HTTP admin/studio API (never the public overlay).
pub const DEFAULT_API_BIND: &str = "127.0.0.1:8787";

/// Default localhost bind for the invite-bounded mesh listener (Phase B).
pub const DEFAULT_MESH_BIND: &str = "127.0.0.1:9797";
