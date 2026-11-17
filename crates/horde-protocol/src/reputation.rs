//! Soft, non-transferable local reputation and fair-share weighting (Phase D).
//!
//! Invariants:
//! - Scores are local and advisory; they never form a spendable credits ledger.
//! - No cryptocurrency, convert, or transfer between peers.
//! - Local blocklists remain first-class and override soft reputation.
//! - Priority hints apply only among invite-trusted peers (operator / job soft hints).

use serde::{Deserialize, Serialize};

use crate::ProtocolError;

/// Default priority hint for a peer with no operator override (neutral).
pub const DEFAULT_PRIORITY_HINT: u8 = 50;

/// Maximum allowed priority hint value.
pub const MAX_PRIORITY_HINT: u8 = 100;

/// After this many jobs a peer has taken from us relative to work given, treat as consumer.
pub const DEFAULT_CONSUMER_RATIO: f64 = 5.0;

/// Minimum outbound jobs we completed for a peer before consumer ignore can apply.
pub const DEFAULT_MIN_SAMPLES: u64 = 3;

/// Soft configuration for reciprocal fair-share (local policy, not a currency).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FairShareConfig {
    /// Ignore peers whose (work_we_did_for_them) / max(work_they_did_for_us, 1) exceeds this.
    pub consumer_ratio_threshold: f64,
    /// Require at least this many jobs we ran for them before applying consumer ignore.
    pub min_samples_before_ignore: u64,
}

impl Default for FairShareConfig {
    fn default() -> Self {
        Self {
            consumer_ratio_threshold: DEFAULT_CONSUMER_RATIO,
            min_samples_before_ignore: DEFAULT_MIN_SAMPLES,
        }
    }
}

/// Per-peer soft score kept on this node only (non-transferable).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PeerSoftScore {
    pub node_id: String,
    /// Successful mesh completions they delivered to us (before accept/reject).
    pub work_completed_for_us: u64,
    /// Successful mesh completions we delivered to them.
    pub work_we_completed_for_them: u64,
    /// Results we accepted from them.
    pub accepted_results: u64,
    /// Results we rejected from them.
    pub rejected_results: u64,
    /// Operator priority hint (0–100) among invite-trusted peers.
    pub priority_hint: u8,
    /// True when fair-share policy currently treats this peer as a pure consumer.
    pub ignored_as_consumer: bool,
    /// Composite fair-share weight used for ranking (higher is better).
    pub fair_share_weight: f64,
}

impl PeerSoftScore {
    pub fn new(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            work_completed_for_us: 0,
            work_we_completed_for_them: 0,
            accepted_results: 0,
            rejected_results: 0,
            priority_hint: DEFAULT_PRIORITY_HINT,
            ignored_as_consumer: false,
            fair_share_weight: 0.0,
        }
    }
}

/// Local soft-reputation ledger (never a credits or transfer system).
#[derive(Clone, Debug, Default)]
pub struct LocalReputationLedger {
    peers: std::collections::HashMap<String, PeerSoftScore>,
    config: FairShareConfig,
}

impl LocalReputationLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(config: FairShareConfig) -> Self {
        Self {
            peers: std::collections::HashMap::new(),
            config,
        }
    }

    pub fn config(&self) -> &FairShareConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: FairShareConfig) {
        self.config = config;
        self.recompute_all();
    }

    fn entry_mut(&mut self, peer_id: &str) -> &mut PeerSoftScore {
        self.peers
            .entry(peer_id.to_string())
            .or_insert_with(|| PeerSoftScore::new(peer_id))
    }

    /// They completed a successful mesh job for us.
    pub fn record_inbound_success(&mut self, peer_id: &str) {
        {
            let e = self.entry_mut(peer_id);
            e.work_completed_for_us = e.work_completed_for_us.saturating_add(1);
        }
        self.recompute(peer_id);
    }

    /// We completed a successful mesh job for them.
    pub fn record_outbound_success(&mut self, peer_id: &str) {
        {
            let e = self.entry_mut(peer_id);
            e.work_we_completed_for_them = e.work_we_completed_for_them.saturating_add(1);
        }
        self.recompute(peer_id);
    }

    /// Requester accepted their attested result.
    pub fn record_accept(&mut self, peer_id: &str) {
        {
            let e = self.entry_mut(peer_id);
            e.accepted_results = e.accepted_results.saturating_add(1);
        }
        self.recompute(peer_id);
    }

    /// Requester rejected their attested result.
    pub fn record_reject(&mut self, peer_id: &str) {
        {
            let e = self.entry_mut(peer_id);
            e.rejected_results = e.rejected_results.saturating_add(1);
        }
        self.recompute(peer_id);
    }

    /// Operator sets a soft priority hint for an invite-trusted peer (0–100).
    pub fn set_priority_hint(&mut self, peer_id: &str, hint: u8) -> Result<(), ProtocolError> {
        if hint > MAX_PRIORITY_HINT {
            return Err(ProtocolError::Other(format!(
                "priority_hint must be 0..={MAX_PRIORITY_HINT}"
            )));
        }
        self.entry_mut(peer_id).priority_hint = hint;
        self.recompute(peer_id);
        Ok(())
    }

    pub fn get(&self, peer_id: &str) -> Option<&PeerSoftScore> {
        self.peers.get(peer_id)
    }

    pub fn should_ignore_consumer(&self, peer_id: &str) -> bool {
        self.peers
            .get(peer_id)
            .map(|p| p.ignored_as_consumer)
            .unwrap_or(false)
    }

    /// Fair-share weight for ranking (higher prefers the peer). Unknown peers score neutral.
    pub fn fair_share_weight(&self, peer_id: &str) -> f64 {
        self.peers
            .get(peer_id)
            .map(|p| p.fair_share_weight)
            .unwrap_or_else(|| compute_weight(&PeerSoftScore::new(peer_id), &self.config))
    }

    /// Rank capable peer ids: non-consumers first, then by fair-share + priority (+ job hint).
    ///
    /// `job_priority_hint` is an optional soft boost (0–100) on the job request; it slightly
    /// amplifies preference for already reciprocal peers — never a paid queue jump.
    pub fn rank_peers(&self, peer_ids: &[String], job_priority_hint: Option<u8>) -> Vec<String> {
        let job_boost = job_priority_hint
            .map(|h| (h.min(MAX_PRIORITY_HINT) as f64) / 100.0)
            .unwrap_or(0.0);

        let mut ranked: Vec<(String, f64, bool)> = peer_ids
            .iter()
            .map(|id| {
                let ignored = self.should_ignore_consumer(id);
                let mut weight = self.fair_share_weight(id);
                // Soft job hint: only amplifies peers that already have positive reciprocity.
                if job_boost > 0.0 && weight > 0.0 {
                    weight += job_boost * weight.min(10.0);
                }
                (id.clone(), weight, ignored)
            })
            .collect();

        ranked.sort_by(|a, b| {
            // Prefer non-ignored consumers.
            match (a.2, b.2) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => b
                    .1
                    .partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.cmp(&b.0)),
            }
        });

        ranked.into_iter().map(|(id, _, _)| id).collect()
    }

    pub fn snapshot(&self) -> Vec<PeerSoftScore> {
        let mut v: Vec<_> = self.peers.values().cloned().collect();
        v.sort_by(|a, b| {
            b.fair_share_weight
                .partial_cmp(&a.fair_share_weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        v
    }

    fn recompute(&mut self, peer_id: &str) {
        let config = self.config.clone();
        if let Some(score) = self.peers.get_mut(peer_id) {
            score.ignored_as_consumer = is_pure_consumer(score, &config);
            score.fair_share_weight = compute_weight(score, &config);
        }
    }

    fn recompute_all(&mut self) {
        let ids: Vec<_> = self.peers.keys().cloned().collect();
        for id in ids {
            self.recompute(&id);
        }
    }
}

fn is_pure_consumer(score: &PeerSoftScore, config: &FairShareConfig) -> bool {
    if score.work_we_completed_for_them < config.min_samples_before_ignore {
        return false;
    }
    let given = score.work_completed_for_us.max(1) as f64;
    let taken = score.work_we_completed_for_them as f64;
    (taken / given) > config.consumer_ratio_threshold
}

fn compute_weight(score: &PeerSoftScore, _config: &FairShareConfig) -> f64 {
    if score.ignored_as_consumer {
        return -1000.0;
    }
    let accepted = score.accepted_results as f64;
    let rejected = score.rejected_results as f64;
    let inbound = score.work_completed_for_us as f64;
    let outbound = score.work_we_completed_for_them as f64;
    let hint = score.priority_hint as f64 / 100.0;
    // Prefer peers who recently completed accepted work for us; mild credit for reciprocity.
    accepted * 2.0 - rejected * 3.0 + inbound * 1.0 + outbound * 0.25 + hint
}

/// Validate an optional job-level priority hint.
pub fn validate_priority_hint(hint: Option<u8>) -> Result<(), ProtocolError> {
    if let Some(h) = hint {
        if h > MAX_PRIORITY_HINT {
            return Err(ProtocolError::Other(format!(
                "priority_hint must be 0..={MAX_PRIORITY_HINT}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefer_accepted_workers() {
        let mut ledger = LocalReputationLedger::new();
        ledger.record_inbound_success("alice");
        ledger.record_accept("alice");
        ledger.record_inbound_success("bob");
        ledger.record_reject("bob");
        let ranked = ledger.rank_peers(&["bob".into(), "alice".into()], None);
        assert_eq!(ranked[0], "alice");
    }

    #[test]
    fn ignore_pure_consumer_after_threshold() {
        let mut ledger = LocalReputationLedger::with_config(FairShareConfig {
            consumer_ratio_threshold: 3.0,
            min_samples_before_ignore: 3,
        });
        for _ in 0..6 {
            ledger.record_outbound_success("taker");
        }
        // Almost no reciprocal work from them.
        ledger.record_inbound_success("taker");
        assert!(ledger.should_ignore_consumer("taker"));
        let ranked = ledger.rank_peers(&["taker".into(), "fresh".into()], None);
        assert_eq!(ranked[0], "fresh");
    }

    #[test]
    fn priority_hint_boosts_trusted_peer() {
        let mut ledger = LocalReputationLedger::new();
        ledger.set_priority_hint("vip", 90).unwrap();
        ledger.set_priority_hint("normal", 10).unwrap();
        let ranked = ledger.rank_peers(&["normal".into(), "vip".into()], None);
        assert_eq!(ranked[0], "vip");
    }

    #[test]
    fn job_priority_hint_amplifies_reciprocal_peers() {
        let mut ledger = LocalReputationLedger::new();
        ledger.record_inbound_success("good");
        ledger.record_accept("good");
        ledger.record_inbound_success("ok");
        let without = ledger.rank_peers(&["ok".into(), "good".into()], None);
        let with = ledger.rank_peers(&["ok".into(), "good".into()], Some(80));
        assert_eq!(without[0], "good");
        assert_eq!(with[0], "good");
        assert!(ledger.fair_share_weight("good") > ledger.fair_share_weight("ok"));
    }

    #[test]
    fn priority_hint_rejects_over_max() {
        let mut ledger = LocalReputationLedger::new();
        assert!(ledger.set_priority_hint("x", 101).is_err());
        assert!(validate_priority_hint(Some(101)).is_err());
        assert!(validate_priority_hint(Some(100)).is_ok());
    }

    #[test]
    fn snapshot_sorted_by_weight() {
        let mut ledger = LocalReputationLedger::new();
        ledger.record_accept("a");
        ledger.record_reject("b");
        let snap = ledger.snapshot();
        assert_eq!(snap[0].node_id, "a");
    }
}
