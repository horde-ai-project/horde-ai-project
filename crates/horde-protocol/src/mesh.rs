//! Invite-bounded mesh wire types (Phase B/C).
//!
//! Discovery defaults to invite-graph + static bootstrap peers.
//! Public DHT join is Phase C+ **explicit opt-in** only (see `PublicDhtConfig`).

use serde::{Deserialize, Serialize};

use crate::capability::CapabilityAdvert;
use crate::identity::{NodeId, PublicKeyBytes};
use crate::job::{JobCreateRequest, JobId, JobResult, JobStatus};
use crate::PROTOCOL_VERSION;

/// Publish-only bootstrap peer entry (config / static list — not a job authority).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootstrapPeer {
    pub addr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Snapshot of a trusted mesh peer after invite-gated Noise handshake.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MeshPeerInfo {
    pub node_id: String,
    pub public_key: PublicKeyBytes,
    pub addr: String,
    pub capability: Option<CapabilityAdvert>,
    pub connected: bool,
}

/// Application messages framed over the Noise channel (JSON).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeshMessage {
    Hello {
        protocol_version: String,
        node_id: NodeId,
        public_key: PublicKeyBytes,
        /// Hex Ed25519 signature over `hello|{protocol}|{fingerprint}`.
        identity_sig_hex: String,
    },
    InvitePresent {
        token: String,
    },
    InviteOk,
    InviteDenied {
        reason: String,
    },
    Capability {
        advert: CapabilityAdvert,
    },
    CapabilityQuery,
    JobOffer {
        job_id: JobId,
        request: JobCreateRequest,
    },
    JobDecline {
        job_id: JobId,
        reason: String,
    },
    JobProgress {
        job_id: JobId,
        status: JobStatus,
    },
    JobComplete {
        job_id: JobId,
        result: JobResult,
    },
    JobFailed {
        job_id: JobId,
        error: String,
    },
    Ping,
    Pong,
}

impl MeshMessage {
    pub fn hello(node_id: NodeId, public_key: PublicKeyBytes, identity_sig_hex: String) -> Self {
        Self::Hello {
            protocol_version: PROTOCOL_VERSION.to_string(),
            node_id,
            public_key,
            identity_sig_hex,
        }
    }

    pub fn hello_sign_payload(protocol_version: &str, fingerprint: &str) -> Vec<u8> {
        format!("horde-mesh-hello|{protocol_version}|{fingerprint}").into_bytes()
    }
}
