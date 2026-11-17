//! Invite-bounded P2P mesh (Phase B/C).
//!
//! - Transport: Noise_XX_25519_ChaChaPoly_BLAKE2s over TCP
//! - Join: invite token required on first connection (default — no open DHT)
//! - Optional public DHT: explicit opt-in stub with rate limits + resource proofs
//! - Jobs: whole-unit or fingerprint-pinned shardable tokenisation
//! - Trust: local blocklists are first-class; bootstrap peers are publish-only

mod blocklist;
mod channel;
mod service;

pub use blocklist::{BlocklistSnapshot, LocalBlocklist};
pub use service::{MeshHost, MeshService, MeshStatus};
