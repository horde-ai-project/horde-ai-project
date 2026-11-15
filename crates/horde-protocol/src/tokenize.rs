//! Deterministic tokenisation chunking under a pinned model fingerprint (Phase C).
//!
//! Generative jobs stay whole-unit. Tokenisation batches are explicitly shardable:
//! same input + seed + fingerprint → same chunks and checksum.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::job::ModelFingerprint;
use crate::ProtocolError;

/// Default lines (or documents) per shard.
pub const DEFAULT_CHUNK_SIZE: usize = 32;

/// Maximum documents accepted in one tokenisation pipeline request.
pub const MAX_TOKENIZE_DOCUMENTS: usize = 10_000;

/// Maximum characters per document line.
pub const MAX_DOCUMENT_CHARS: usize = 1_048_576;

/// Studio / worker payload for a shardable tokenisation job.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenizeRequest {
    /// One string per document (or line). Order is preserved across shards.
    pub documents: Vec<String>,
    /// Documents per chunk (minimum 1). Default [`DEFAULT_CHUNK_SIZE`].
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    /// Determinism seed mixed into token ids / checksum.
    #[serde(default)]
    pub seed: u64,
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

impl TokenizeRequest {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.documents.is_empty() {
            return Err(ProtocolError::InvalidTokenize(
                "documents must not be empty".into(),
            ));
        }
        if self.documents.len() > MAX_TOKENIZE_DOCUMENTS {
            return Err(ProtocolError::InvalidTokenize(format!(
                "too many documents (max {MAX_TOKENIZE_DOCUMENTS})"
            )));
        }
        if self.chunk_size == 0 {
            return Err(ProtocolError::InvalidTokenize(
                "chunk_size must be >= 1".into(),
            ));
        }
        for (i, doc) in self.documents.iter().enumerate() {
            if doc.len() > MAX_DOCUMENT_CHARS {
                return Err(ProtocolError::InvalidTokenize(format!(
                    "document {i} exceeds {MAX_DOCUMENT_CHARS} bytes"
                )));
            }
        }
        Ok(())
    }

    /// Parse JSON payload, or treat raw text as a single-document request.
    pub fn from_job_payload(payload: &str) -> Result<Self, ProtocolError> {
        let trimmed = payload.trim();
        if trimmed.starts_with('{') {
            let req: Self = serde_json::from_str(trimmed)
                .map_err(|e| ProtocolError::InvalidTokenize(e.to_string()))?;
            req.validate()?;
            Ok(req)
        } else if trimmed.is_empty() {
            Err(ProtocolError::InvalidTokenize(
                "empty tokenisation payload".into(),
            ))
        } else {
            let req = Self {
                documents: vec![trimmed.to_string()],
                chunk_size: DEFAULT_CHUNK_SIZE,
                seed: 0,
            };
            req.validate()?;
            Ok(req)
        }
    }
}

/// One deterministic shard under a pinned fingerprint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenizeChunk {
    pub index: u32,
    pub total: u32,
    pub documents: Vec<String>,
    /// Compact fingerprint string mixed into stub token ids.
    pub fingerprint_compact: String,
    pub seed: u64,
}

/// Per-chunk stub tokenisation output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenizeChunkOutput {
    pub index: u32,
    pub total: u32,
    /// Flattened token ids for documents in this chunk (space-joined doc boundaries as empty token 0 separators omitted — one vec per doc).
    pub token_ids: Vec<Vec<u32>>,
    pub checksum_hex: String,
}

/// Merged pipeline result (JSON-serialised into job output).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenizePipelineResult {
    pub chunk_count: u32,
    pub document_count: u32,
    pub chunks: Vec<TokenizeChunkOutput>,
    /// SHA-256 over ordered chunk checksums + fingerprint + seed.
    pub pipeline_checksum_hex: String,
    pub fingerprint_compact: String,
    pub seed: u64,
}

/// Plan deterministic shards. Same inputs always yield the same chunk boundaries.
pub fn plan_chunks(
    req: &TokenizeRequest,
    fingerprint: &ModelFingerprint,
) -> Result<Vec<TokenizeChunk>, ProtocolError> {
    req.validate()?;
    fingerprint.validate()?;
    if fingerprint.tokenizer_revision.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Err(ProtocolError::InvalidFingerprint(
            "tokenizer_revision required for tokenisation jobs".into(),
        ));
    }

    let total = req.documents.len().div_ceil(req.chunk_size).max(1) as u32;
    let compact = fingerprint.compact();
    let mut chunks = Vec::with_capacity(total as usize);
    for (i, piece) in req.documents.chunks(req.chunk_size).enumerate() {
        chunks.push(TokenizeChunk {
            index: i as u32,
            total,
            documents: piece.to_vec(),
            fingerprint_compact: compact.clone(),
            seed: req.seed,
        });
    }
    Ok(chunks)
}

/// Stub whitespace tokeniser: deterministic u32 ids from fingerprint + seed + token text.
pub fn stub_tokenize_chunk(chunk: &TokenizeChunk) -> TokenizeChunkOutput {
    let mut token_ids = Vec::with_capacity(chunk.documents.len());
    let mut hasher = Sha256::new();
    hasher.update(chunk.fingerprint_compact.as_bytes());
    hasher.update(chunk.seed.to_le_bytes());
    hasher.update(chunk.index.to_le_bytes());

    for doc in &chunk.documents {
        let mut doc_tokens = Vec::new();
        for token in doc.split_whitespace() {
            let id = stub_token_id(&chunk.fingerprint_compact, chunk.seed, token);
            doc_tokens.push(id);
            hasher.update(id.to_le_bytes());
            hasher.update(token.as_bytes());
        }
        token_ids.push(doc_tokens);
    }

    TokenizeChunkOutput {
        index: chunk.index,
        total: chunk.total,
        token_ids,
        checksum_hex: hex::encode(hasher.finalize()),
    }
}

fn stub_token_id(fingerprint_compact: &str, seed: u64, token: &str) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(b"horde-stub-tok|");
    hasher.update(fingerprint_compact.as_bytes());
    hasher.update(seed.to_le_bytes());
    hasher.update(token.as_bytes());
    let digest = hasher.finalize();
    u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]])
}

/// Merge ordered chunk outputs; rejects missing/duplicate/out-of-order indexes.
pub fn merge_chunk_outputs(
    fingerprint: &ModelFingerprint,
    seed: u64,
    mut chunks: Vec<TokenizeChunkOutput>,
) -> Result<TokenizePipelineResult, ProtocolError> {
    if chunks.is_empty() {
        return Err(ProtocolError::InvalidTokenize(
            "no chunk outputs to merge".into(),
        ));
    }
    chunks.sort_by_key(|c| c.index);
    let total = chunks[0].total;
    if total == 0 || chunks.len() as u32 != total {
        return Err(ProtocolError::InvalidTokenize(format!(
            "expected {total} chunks, got {}",
            chunks.len()
        )));
    }
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.index != i as u32 || chunk.total != total {
            return Err(ProtocolError::InvalidTokenize(format!(
                "chunk order/total mismatch at index {i}"
            )));
        }
    }

    let document_count: u32 = chunks
        .iter()
        .map(|c| c.token_ids.len() as u32)
        .sum();

    let mut hasher = Sha256::new();
    hasher.update(fingerprint.compact().as_bytes());
    hasher.update(seed.to_le_bytes());
    for chunk in &chunks {
        hasher.update(chunk.checksum_hex.as_bytes());
    }

    Ok(TokenizePipelineResult {
        chunk_count: total,
        document_count,
        chunks,
        pipeline_checksum_hex: hex::encode(hasher.finalize()),
        fingerprint_compact: fingerprint.compact(),
        seed,
    })
}

/// Run the full stub pipeline (plan → tokenize each shard → merge).
pub fn run_stub_tokenize_pipeline(
    req: &TokenizeRequest,
    fingerprint: &ModelFingerprint,
) -> Result<TokenizePipelineResult, ProtocolError> {
    let plan = plan_chunks(req, fingerprint)?;
    let outputs: Vec<_> = plan.iter().map(stub_tokenize_chunk).collect();
    merge_chunk_outputs(fingerprint, req.seed, outputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp() -> ModelFingerprint {
        ModelFingerprint {
            model_id: "stub/tokenizer".into(),
            revision_hash: "deadbeefcafebabe".into(),
            tokenizer_revision: Some("tok-rev-1".into()),
            precision: "fp16".into(),
        }
    }

    #[test]
    fn plan_is_deterministic() {
        let req = TokenizeRequest {
            documents: (0..70).map(|i| format!("doc {i} alpha")).collect(),
            chunk_size: 32,
            seed: 7,
        };
        let a = plan_chunks(&req, &fp()).unwrap();
        let b = plan_chunks(&req, &fp()).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].documents.len(), 32);
        assert_eq!(a[2].documents.len(), 6);
    }

    #[test]
    fn pipeline_checksum_stable() {
        let req = TokenizeRequest {
            documents: vec!["hello world".into(), "foo bar".into(), "baz".into()],
            chunk_size: 2,
            seed: 42,
        };
        let r1 = run_stub_tokenize_pipeline(&req, &fp()).unwrap();
        let r2 = run_stub_tokenize_pipeline(&req, &fp()).unwrap();
        assert_eq!(r1.pipeline_checksum_hex, r2.pipeline_checksum_hex);
        assert_eq!(r1.chunk_count, 2);
        assert_eq!(r1.document_count, 3);
        assert!(!r1.chunks[0].token_ids[0].is_empty());
    }

    #[test]
    fn requires_tokenizer_revision() {
        let mut bad = fp();
        bad.tokenizer_revision = None;
        let req = TokenizeRequest {
            documents: vec!["x".into()],
            chunk_size: 1,
            seed: 0,
        };
        assert!(plan_chunks(&req, &bad).is_err());
    }

    #[test]
    fn from_raw_payload() {
        let req = TokenizeRequest::from_job_payload("plain text line").unwrap();
        assert_eq!(req.documents.len(), 1);
    }
}
