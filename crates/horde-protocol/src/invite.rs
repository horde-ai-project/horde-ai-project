use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ProtocolError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitePurpose {
    /// Join a collective as a physical server member (LAN/mTLS).
    CollectiveJoin,
    /// Soft-trust mesh invite (Phase B: required on first Noise join).
    MeshJoin,
}

/// Plaintext invite shown once at mint time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteTokenPlain {
    pub token_id: Uuid,
    /// Opaque secret; treat as a password.
    pub token: String,
    pub purpose: InvitePurpose,
    pub expires_at: DateTime<Utc>,
}

/// Stored invite record (secret is hashed).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteRecord {
    pub token_id: Uuid,
    pub token_hash: String,
    pub purpose: InvitePurpose,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub redeemed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redeemed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl InviteRecord {
    pub fn hash_token(token: &str) -> String {
        let digest = Sha256::digest(token.as_bytes());
        hex::encode(digest)
    }

    pub fn mint(
        purpose: InvitePurpose,
        ttl_secs: i64,
        plaintext: String,
        note: Option<String>,
    ) -> (InviteTokenPlain, Self) {
        let token_id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = now + Duration::seconds(ttl_secs.max(60));
        let plain = InviteTokenPlain {
            token_id,
            token: plaintext.clone(),
            purpose: purpose.clone(),
            expires_at,
        };
        let record = Self {
            token_id,
            token_hash: Self::hash_token(&plaintext),
            purpose,
            created_at: now,
            expires_at,
            redeemed: false,
            redeemed_by: None,
            note,
        };
        (plain, record)
    }

    pub fn is_valid_now(&self) -> bool {
        !self.redeemed && Utc::now() < self.expires_at
    }

    pub fn verify_plaintext(&self, token: &str) -> Result<(), ProtocolError> {
        if !self.is_valid_now() {
            return Err(ProtocolError::InviteInvalid);
        }
        if Self::hash_token(token) != self.token_hash {
            return Err(ProtocolError::InviteInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteRedeemRequest {
    pub token: String,
    /// Hostname or label of the joining physical server / peer.
    pub claimant: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<InvitePurpose>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_and_verify() {
        let (plain, record) =
            InviteRecord::mint(InvitePurpose::CollectiveJoin, 3600, "secret-token".into(), None);
        assert!(record.verify_plaintext(&plain.token).is_ok());
        assert!(record.verify_plaintext("wrong").is_err());
    }
}
