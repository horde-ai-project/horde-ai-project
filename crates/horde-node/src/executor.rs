use std::time::Instant;

use horde_protocol::{
    run_stub_tokenize_pipeline, JobRecord, JobResult, TokenizeRequest, WorkKind,
};
use tracing::info;

use crate::keystore::IdentityKeys;

/// GPU / workload executor surface. Phase A ships the stub implementation.
pub trait GpuExecutor: Send + Sync {
    fn execute(
        &self,
        job: &JobRecord,
        keys: &IdentityKeys,
    ) -> impl std::future::Future<Output = anyhow::Result<JobResult>> + Send;
}

/// Deterministic echo / tokenise executor used when no GPU is available (CI / local smoke).
#[derive(Debug, Default, Clone, Copy)]
pub struct StubExecutor;

impl GpuExecutor for StubExecutor {
    async fn execute(&self, job: &JobRecord, keys: &IdentityKeys) -> anyhow::Result<JobResult> {
        let started = Instant::now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let output = match job.kind {
            WorkKind::StubEcho | WorkKind::TextGenerate => {
                format!(
                    "stub-result fingerprint={} payload_len={} echo={}",
                    job.fingerprint.compact(),
                    job.payload.len(),
                    truncate(&job.payload, 256)
                )
            }
            WorkKind::Tokenize => {
                let req = TokenizeRequest::from_job_payload(&job.payload)?;
                let pipeline = run_stub_tokenize_pipeline(&req, &job.fingerprint)?;
                serde_json::to_string(&pipeline)?
            }
            WorkKind::Embed => {
                // Stub embed: reuse tokenisation pipeline checksum as a deterministic digest.
                let req = TokenizeRequest::from_job_payload(&job.payload).unwrap_or_else(|_| {
                    TokenizeRequest {
                        documents: vec![job.payload.clone()],
                        chunk_size: horde_protocol::DEFAULT_CHUNK_SIZE,
                        seed: 0,
                    }
                });
                let pipeline = run_stub_tokenize_pipeline(&req, &job.fingerprint)?;
                serde_json::to_string(&serde_json::json!({
                    "kind": "embed",
                    "document_count": pipeline.document_count,
                    "chunk_count": pipeline.chunk_count,
                    "embedding_checksum_hex": pipeline.pipeline_checksum_hex,
                    "fingerprint_compact": pipeline.fingerprint_compact,
                    "seed": pipeline.seed,
                }))?
            }
            WorkKind::VideoGenerate => {
                anyhow::bail!(
                    "video_generate rejected by stub executor; requires CUDA/ROCm resource gate"
                );
            }
        };

        let wall_time_ms = started.elapsed().as_millis() as u64;
        let runtime = match job.kind {
            WorkKind::Tokenize => "stub-tokenizer/0.3",
            WorkKind::Embed => "stub-embed/0.3",
            _ => "stub-executor/0.3",
        };
        let attestation =
            keys.sign_attestation(&job.fingerprint, runtime, wall_time_ms, &output);
        info!(job_id = %job.id, wall_time_ms, kind = ?job.kind, "stub executor finished");
        Ok(JobResult {
            output,
            attestation,
        })
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.chars().count() <= max {
        s
    } else {
        let end = s
            .char_indices()
            .nth(max)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horde_protocol::{JobCreateRequest, JobRouting, ModelFingerprint};

    #[tokio::test]
    async fn tokenize_pipeline_deterministic() {
        let keys = IdentityKeys::generate();
        let exec = StubExecutor;
        let payload = serde_json::json!({
            "documents": ["hello world", "foo bar baz", "third"],
            "chunk_size": 2,
            "seed": 9
        })
        .to_string();
        let job = JobRecord::new(JobCreateRequest {
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
        })
        .unwrap();
        let a = exec.execute(&job, &keys).await.unwrap();
        let b = exec.execute(&job, &keys).await.unwrap();
        let ra: serde_json::Value = serde_json::from_str(&a.output).unwrap();
        let rb: serde_json::Value = serde_json::from_str(&b.output).unwrap();
        assert_eq!(ra["pipeline_checksum_hex"], rb["pipeline_checksum_hex"]);
        assert_eq!(ra["chunk_count"], 2);
    }
}
