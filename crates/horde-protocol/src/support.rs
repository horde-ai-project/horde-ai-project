//! Hardened support matrix and resource gates for Phase C workloads.

use serde::{Deserialize, Serialize};

use crate::capability::{BackendKind, CapabilityAdvert};
use crate::job::{ModelFingerprint, WorkKind};
use crate::ProtocolError;

/// Minimum free VRAM (MiB) required before a video job may be accepted.
pub const VIDEO_MIN_VRAM_MB: u32 = 12_288;

/// Resource / support entry for one work kind.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkSupport {
    pub kind: WorkKind,
    pub backends: Vec<BackendKind>,
    pub min_vram_mb: u32,
    pub requires_tokenizer_revision: bool,
    /// When true, work may be sharded under one pinned fingerprint.
    pub shardable: bool,
    pub notes: String,
}

/// Catalogue of supported workloads (stub GPU included for CI).
pub fn support_matrix() -> Vec<WorkSupport> {
    vec![
        WorkSupport {
            kind: WorkKind::StubEcho,
            backends: vec![BackendKind::Stub, BackendKind::Cpu],
            min_vram_mb: 0,
            requires_tokenizer_revision: false,
            shardable: false,
            notes: "CI / smoke whole-unit echo".into(),
        },
        WorkSupport {
            kind: WorkKind::TextGenerate,
            backends: vec![
                BackendKind::Stub,
                BackendKind::Cuda,
                BackendKind::Rocm,
                BackendKind::Cpu,
            ],
            min_vram_mb: 0,
            requires_tokenizer_revision: false,
            shardable: false,
            notes: "Whole-unit generative text; stub until real backends wire in".into(),
        },
        WorkSupport {
            kind: WorkKind::Tokenize,
            backends: vec![
                BackendKind::Stub,
                BackendKind::Cpu,
                BackendKind::Cuda,
                BackendKind::Rocm,
            ],
            min_vram_mb: 0,
            requires_tokenizer_revision: true,
            shardable: true,
            notes: "Dataset tokenisation with deterministic chunking under pinned fingerprint".into(),
        },
        WorkSupport {
            kind: WorkKind::Embed,
            backends: vec![
                BackendKind::Stub,
                BackendKind::Cpu,
                BackendKind::Cuda,
                BackendKind::Rocm,
            ],
            min_vram_mb: 0,
            requires_tokenizer_revision: true,
            shardable: true,
            notes: "Batch embedding under pinned fingerprint (stub checksum path)".into(),
        },
        WorkSupport {
            kind: WorkKind::VideoGenerate,
            backends: vec![BackendKind::Cuda, BackendKind::Rocm],
            min_vram_mb: VIDEO_MIN_VRAM_MB,
            requires_tokenizer_revision: false,
            shardable: false,
            notes: "Video only behind high VRAM gate; stub backend never accepts video".into(),
        },
    ]
}

pub fn support_for(kind: &WorkKind) -> WorkSupport {
    support_matrix()
        .into_iter()
        .find(|s| &s.kind == kind)
        .expect("every WorkKind has a support row")
}

/// Validate fingerprint + capability against the support matrix for `kind`.
pub fn check_resource_gate(
    kind: &WorkKind,
    fingerprint: &ModelFingerprint,
    capability: &CapabilityAdvert,
    requested_vram_mb: Option<u32>,
) -> Result<(), ProtocolError> {
    let gate = support_for(kind);

    if gate.requires_tokenizer_revision
        && fingerprint
            .tokenizer_revision
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        return Err(ProtocolError::InvalidFingerprint(
            "tokenizer_revision required for this work kind".into(),
        ));
    }

    if !gate.backends.contains(&capability.backend) {
        return Err(ProtocolError::ResourceGate(format!(
            "{kind:?} not supported on backend {:?}",
            capability.backend
        )));
    }

    let need = requested_vram_mb.unwrap_or(gate.min_vram_mb).max(gate.min_vram_mb);
    if need > 0 && capability.backend != BackendKind::Stub {
        if capability.vram_free_mb < need {
            return Err(ProtocolError::ResourceGate(format!(
                "need {need} MiB VRAM, free {}",
                capability.vram_free_mb
            )));
        }
    }

    if matches!(kind, WorkKind::VideoGenerate) {
        if capability.backend == BackendKind::Stub {
            return Err(ProtocolError::ResourceGate(
                "video_generate rejected on stub backend; requires CUDA/ROCm and >= 12288 MiB VRAM"
                    .into(),
            ));
        }
        if capability.vram_free_mb < VIDEO_MIN_VRAM_MB {
            return Err(ProtocolError::ResourceGate(format!(
                "video_generate requires >= {VIDEO_MIN_VRAM_MB} MiB free VRAM"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_requires_tokenizer_rev() {
        let cap = CapabilityAdvert::stub_local("s", "h");
        let fp = ModelFingerprint {
            model_id: "stub/tokenizer".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        };
        assert!(check_resource_gate(&WorkKind::Tokenize, &fp, &cap, None).is_err());
    }

    #[test]
    fn video_rejected_on_stub() {
        let cap = CapabilityAdvert::stub_local("s", "h");
        let fp = ModelFingerprint {
            model_id: "stub/video".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        };
        assert!(check_resource_gate(&WorkKind::VideoGenerate, &fp, &cap, Some(16_000)).is_err());
    }

    #[test]
    fn stub_echo_ok() {
        let cap = CapabilityAdvert::stub_local("s", "h");
        let fp = ModelFingerprint {
            model_id: "stub/echo".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: None,
            precision: "fp16".into(),
        };
        assert!(check_resource_gate(&WorkKind::StubEcho, &fp, &cap, None).is_ok());
    }
}
