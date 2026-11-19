use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Soft per-peer pause: decline inbound jobs without kicking or blocklisting.
/// Invite mesh and reconnect trust stay intact; operator policy only.
#[derive(Debug, Default)]
pub struct LocalMuteList {
    inner: RwLock<HashSet<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MuteListSnapshot {
    pub muted_node_ids: Vec<String>,
}

impl LocalMuteList {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn mute(&self, node_id: impl Into<String>) {
        self.inner.write().await.insert(node_id.into());
    }

    pub async fn unmute(&self, node_id: &str) -> bool {
        self.inner.write().await.remove(node_id)
    }

    pub async fn is_muted(&self, node_id: &str) -> bool {
        self.inner.read().await.contains(node_id)
    }

    pub async fn snapshot(&self) -> MuteListSnapshot {
        let mut muted_node_ids: Vec<_> = self.inner.read().await.iter().cloned().collect();
        muted_node_ids.sort();
        MuteListSnapshot { muted_node_ids }
    }
}
