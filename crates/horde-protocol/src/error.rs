use thiserror::Error;

pub type ProtocolResult<T> = Result<T, ProtocolError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid model fingerprint: {0}")]
    InvalidFingerprint(String),
    #[error("invite expired or already used")]
    InviteInvalid,
    #[error("job not found")]
    JobNotFound,
    #[error("job is not cancellable in status {0}")]
    JobNotCancellable(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("server not found")]
    ServerNotFound,
    #[error("limit rejected: {0}")]
    LimitRejected(String),
    #[error("capability mismatch: {0}")]
    CapabilityMismatch(String),
    #[error("peer is blocked locally")]
    PeerBlocked,
    #[error("mesh invite required")]
    MeshInviteRequired,
    #[error("peer not found")]
    PeerNotFound,
    #[error("attestation invalid: {0}")]
    AttestationInvalid(String),
    #[error("no capable mesh peer for job")]
    NoMeshPeer,
    #[error("invalid tokenisation request: {0}")]
    InvalidTokenize(String),
    #[error("resource gate: {0}")]
    ResourceGate(String),
    #[error("public DHT is disabled (invite-only default); enable only via explicit opt-in")]
    DhtDisabled,
    #[error("resource proof invalid: {0}")]
    ResourceProofInvalid(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("keystore error: {0}")]
    Keystore(String),
    #[error("{0}")]
    Other(String),
}
