use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Operator-local peer deny list. Soft gossip is never an authority over this.
#[derive(Debug, Default)]
pub struct LocalBlocklist {
    inner: RwLock<HashSet<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlocklistSnapshot {
    pub blocked_node_ids: Vec<String>,
}

impl LocalBlocklist {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn block(&self, node_id: impl Into<String>) {
        self.inner.write().await.insert(node_id.into());
    }

    pub async fn unblock(&self, node_id: &str) -> bool {
        self.inner.write().await.remove(node_id)
    }

    pub async fn is_blocked(&self, node_id: &str) -> bool {
        self.inner.read().await.contains(node_id)
    }

    pub async fn snapshot(&self) -> BlocklistSnapshot {
        let mut blocked_node_ids: Vec<_> = self.inner.read().await.iter().cloned().collect();
        blocked_node_ids.sort();
        BlocklistSnapshot { blocked_node_ids }
    }
}
