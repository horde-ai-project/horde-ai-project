use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// 32-byte Ed25519 verifying key, JSON-encoded as hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PublicKeyBytes(pub [u8; 32]);

impl PublicKeyBytes {
    pub fn from_verifying_key(key: &VerifyingKey) -> Self {
        Self(key.to_bytes())
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn short_id(&self) -> String {
        let full = self.to_hex();
        full[..12].to_string()
    }
}

impl Serialize for PublicKeyBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for PublicKeyBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("public key must be 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }
}

impl std::fmt::Display for PublicKeyBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Logical node / collective identity derived from the node public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub PublicKeyBytes);

impl NodeId {
    pub fn from_public_key(pk: PublicKeyBytes) -> Self {
        Self(pk)
    }

    /// Stable fingerprint used in logs and admin UI (SHA-256 of pubkey, truncated).
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0 .0);
        hex::encode(&digest[..16])
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.fingerprint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn node_id_is_stable() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk = PublicKeyBytes::from_verifying_key(&sk.verifying_key());
        let a = NodeId::from_public_key(pk);
        let b = NodeId::from_public_key(pk);
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    #[test]
    fn public_key_json_is_hex() {
        let pk = PublicKeyBytes([0xab; 32]);
        let json = serde_json::to_string(&pk).unwrap();
        assert!(json.contains("ababab"));
        let back: PublicKeyBytes = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pk);
    }
}
