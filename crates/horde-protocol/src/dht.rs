//! Optional public DHT discovery (Phase C+): **explicit opt-in**, default off.
//!
//! Invite-bounded mesh remains the default join path. This module is a stub with
//! rate-limit and resource-proof hooks so a real DHT can be wired later without
//! changing the safety-locked defaults.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ProtocolError;

/// Default DHT query budget when public discovery is enabled.
pub const DEFAULT_DHT_QUERIES_PER_MINUTE: u32 = 30;

/// Configuration for optional public DHT discovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicDhtConfig {
    /// Must be set `true` explicitly — default is invite-only (`false`).
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_qpm")]
    pub max_queries_per_minute: u32,
    /// When enabled, callers must present a resource proof (stub verified).
    #[serde(default = "default_require_proof")]
    pub require_resource_proof: bool,
}

fn default_qpm() -> u32 {
    DEFAULT_DHT_QUERIES_PER_MINUTE
}

fn default_require_proof() -> bool {
    true
}

impl Default for PublicDhtConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_queries_per_minute: DEFAULT_DHT_QUERIES_PER_MINUTE,
            require_resource_proof: true,
        }
    }
}

impl PublicDhtConfig {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.max_queries_per_minute == 0 {
            return Err(ProtocolError::LimitRejected(
                "max_queries_per_minute must be >= 1 when configuring DHT".into(),
            ));
        }
        if self.max_queries_per_minute > 1_000 {
            return Err(ProtocolError::LimitRejected(
                "max_queries_per_minute too high (max 1000)".into(),
            ));
        }
        Ok(())
    }

    /// Env helper: `HORDE_PUBLIC_DHT=1` enables the stub; still rate-limited + proof-gated.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if matches!(
            std::env::var("HORDE_PUBLIC_DHT").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes")
        ) {
            cfg.enabled = true;
        }
        if let Ok(qpm) = std::env::var("HORDE_DHT_QPM") {
            if let Ok(n) = qpm.parse::<u32>() {
                cfg.max_queries_per_minute = n;
            }
        }
        cfg
    }
}

/// Challenge issued before a public DHT lookup is allowed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceProofChallenge {
    pub nonce_hex: String,
    /// Difficulty: leading zero nibbles required in the response digest (stub scale 0–4).
    pub difficulty: u8,
}

/// Caller response to a resource-proof challenge.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceProofResponse {
    pub nonce_hex: String,
    pub counter: u64,
    pub digest_hex: String,
}

impl ResourceProofChallenge {
    pub fn new(nonce_hex: impl Into<String>, difficulty: u8) -> Self {
        Self {
            nonce_hex: nonce_hex.into(),
            difficulty: difficulty.min(4),
        }
    }

    pub fn digest_for(nonce_hex: &str, counter: u64) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"horde-dht-rproof|");
        hasher.update(nonce_hex.as_bytes());
        hasher.update(counter.to_le_bytes());
        hasher.finalize().into()
    }

    /// Brute-force a valid counter for tests / stub clients.
    pub fn solve(&self) -> ResourceProofResponse {
        let mut counter = 0u64;
        loop {
            let digest = Self::digest_for(&self.nonce_hex, counter);
            if leading_zero_nibbles(&digest) >= self.difficulty {
                return ResourceProofResponse {
                    nonce_hex: self.nonce_hex.clone(),
                    counter,
                    digest_hex: hex::encode(digest),
                };
            }
            counter += 1;
        }
    }

    pub fn verify(&self, response: &ResourceProofResponse) -> Result<(), ProtocolError> {
        if response.nonce_hex != self.nonce_hex {
            return Err(ProtocolError::ResourceProofInvalid(
                "nonce mismatch".into(),
            ));
        }
        let expected = Self::digest_for(&self.nonce_hex, response.counter);
        let got = hex::decode(&response.digest_hex)
            .map_err(|e| ProtocolError::ResourceProofInvalid(e.to_string()))?;
        if got.as_slice() != expected.as_slice() {
            return Err(ProtocolError::ResourceProofInvalid(
                "digest mismatch".into(),
            ));
        }
        if leading_zero_nibbles(&expected) < self.difficulty {
            return Err(ProtocolError::ResourceProofInvalid(
                "insufficient difficulty".into(),
            ));
        }
        Ok(())
    }
}

fn leading_zero_nibbles(digest: &[u8; 32]) -> u8 {
    let mut n = 0u8;
    for byte in digest {
        if *byte == 0 {
            n += 2;
            continue;
        }
        if byte & 0xf0 == 0 {
            n += 1;
        }
        break;
    }
    n
}

/// Sliding-window rate limiter for DHT queries.
#[derive(Debug, Default)]
pub struct DhtRateLimiter {
    window: Duration,
    events: VecDeque<Instant>,
}

impl DhtRateLimiter {
    pub fn per_minute() -> Self {
        Self {
            window: Duration::from_secs(60),
            events: VecDeque::new(),
        }
    }

    pub fn try_acquire(&mut self, max_per_window: u32) -> Result<(), ProtocolError> {
        let now = Instant::now();
        while let Some(front) = self.events.front() {
            if now.duration_since(*front) > self.window {
                self.events.pop_front();
            } else {
                break;
            }
        }
        if self.events.len() as u32 >= max_per_window {
            return Err(ProtocolError::RateLimited(
                "public DHT query rate limit exceeded".into(),
            ));
        }
        self.events.push_back(now);
        Ok(())
    }
}

/// In-process public DHT stub (no real overlay). Opt-in only.
#[derive(Debug)]
pub struct PublicDhtStub {
    config: PublicDhtConfig,
    limiter: DhtRateLimiter,
    /// Last challenge issued (single-slot stub).
    pending_challenge: Option<ResourceProofChallenge>,
}

impl Default for PublicDhtStub {
    fn default() -> Self {
        Self::new(PublicDhtConfig::default())
    }
}

impl PublicDhtStub {
    pub fn new(config: PublicDhtConfig) -> Self {
        Self {
            config,
            limiter: DhtRateLimiter::per_minute(),
            pending_challenge: None,
        }
    }

    pub fn config(&self) -> &PublicDhtConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: PublicDhtConfig) -> Result<(), ProtocolError> {
        config.validate()?;
        self.config = config;
        Ok(())
    }

    pub fn issue_challenge(&mut self) -> Result<ResourceProofChallenge, ProtocolError> {
        if !self.config.enabled {
            return Err(ProtocolError::DhtDisabled);
        }
        let nonce = hex::encode(Sha256::digest(
            format!("chal|{:?}", Instant::now()).as_bytes(),
        ));
        let challenge = ResourceProofChallenge::new(&nonce[..16], 1);
        self.pending_challenge = Some(challenge.clone());
        Ok(challenge)
    }

    /// Stub lookup: returns empty peer list when enabled + rate limit + proof OK.
    pub fn lookup(
        &mut self,
        key: &str,
        proof: Option<&ResourceProofResponse>,
    ) -> Result<Vec<String>, ProtocolError> {
        if !self.config.enabled {
            return Err(ProtocolError::DhtDisabled);
        }
        if key.trim().is_empty() {
            return Err(ProtocolError::Other("DHT lookup key empty".into()));
        }
        self.limiter
            .try_acquire(self.config.max_queries_per_minute)?;

        if self.config.require_resource_proof {
            let challenge = self
                .pending_challenge
                .as_ref()
                .ok_or(ProtocolError::ResourceProofInvalid(
                    "no challenge issued; call challenge first".into(),
               ))?;
            let proof = proof.ok_or(ProtocolError::ResourceProofInvalid(
                "resource proof required".into(),
            ))?;
            challenge.verify(proof)?;
            self.pending_challenge = None;
        }

        // Stub: no public peers discovered — invite graph remains the real path.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_disabled() {
        let mut dht = PublicDhtStub::default();
        assert!(!dht.config().enabled);
        assert!(matches!(dht.lookup("k", None), Err(ProtocolError::DhtDisabled)));
    }

    #[test]
    fn opt_in_with_proof() {
        let mut dht = PublicDhtStub::new(PublicDhtConfig {
            enabled: true,
            max_queries_per_minute: 10,
            require_resource_proof: true,
        });
        let chal = dht.issue_challenge().unwrap();
        let proof = chal.solve();
        let peers = dht.lookup("bootstrap", Some(&proof)).unwrap();
        assert!(peers.is_empty());
    }

    #[test]
    fn rate_limit_trips() {
        let mut dht = PublicDhtStub::new(PublicDhtConfig {
            enabled: true,
            max_queries_per_minute: 2,
            require_resource_proof: false,
        });
        assert!(dht.lookup("a", None).is_ok());
        assert!(dht.lookup("b", None).is_ok());
        assert!(matches!(
            dht.lookup("c", None),
            Err(ProtocolError::RateLimited(_))
        ));
    }
}
