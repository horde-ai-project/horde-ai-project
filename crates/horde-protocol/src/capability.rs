use serde::{Deserialize, Serialize};

use crate::job::ModelFingerprint;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Stub,
    Cuda,
    Rocm,
    Cpu,
}

/// What a physical server advertises before accepting work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityAdvert {
    pub server_id: String,
    pub hostname: String,
    pub backend: BackendKind,
    pub vram_total_mb: u32,
    pub vram_free_mb: u32,
    pub agent_version: String,
    pub models: Vec<ModelFingerprint>,
    pub online: bool,
}

impl CapabilityAdvert {
    pub fn stub_local(server_id: impl Into<String>, hostname: impl Into<String>) -> Self {
        Self {
            server_id: server_id.into(),
            hostname: hostname.into(),
            backend: BackendKind::Stub,
            vram_total_mb: 0,
            vram_free_mb: 0,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            models: vec![
                ModelFingerprint {
                    model_id: "stub/echo".into(),
                    revision_hash: "deadbeefcafebabe".into(),
                    tokenizer_revision: None,
                    precision: "fp16".into(),
                },
                ModelFingerprint {
                    model_id: "stub/tokenizer".into(),
                    revision_hash: "deadbeefcafebabe".into(),
                    tokenizer_revision: Some("tok-rev-1".into()),
                    precision: "fp16".into(),
                },
            ],
            online: true,
        }
    }

    pub fn supports_fingerprint(&self, fp: &ModelFingerprint) -> bool {
        self.models.iter().any(|m| {
            m.model_id == fp.model_id
                && m.revision_hash == fp.revision_hash
                && match (&fp.tokenizer_revision, &m.tokenizer_revision) {
                    (None, _) => true,
                    (Some(need), Some(have)) => need == have,
                    (Some(_), None) => false,
                }
        })
    }
}
