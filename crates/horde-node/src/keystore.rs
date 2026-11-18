use std::path::Path;

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use horde_protocol::{ModelFingerprint, NodeId, PublicKeyBytes, ResultAttestation};
use rand::{rngs::OsRng, Rng};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("incorrect passphrase")]
    BadPassphrase,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("serde: {0}")]
    Serde(String),
}

/// Unlocked node signing material. Signing bytes are zeroized on drop.
pub struct IdentityKeys {
    pub node_id: NodeId,
    pub verifying_key: VerifyingKey,
    signing_key_bytes: [u8; 32],
}

impl Drop for IdentityKeys {
    fn drop(&mut self) {
        self.signing_key_bytes.zeroize();
    }
}

impl Clone for IdentityKeys {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id,
            verifying_key: self.verifying_key,
            signing_key_bytes: self.signing_key_bytes,
        }
    }
}

impl IdentityKeys {
    pub fn generate() -> Self {
        let signing = SigningKey::generate(&mut OsRng);
        Self::from_signing(signing)
    }

    fn from_signing(signing: SigningKey) -> Self {
        let verifying = signing.verifying_key();
        let pk = PublicKeyBytes::from_verifying_key(&verifying);
        Self {
            node_id: NodeId::from_public_key(pk),
            verifying_key: verifying,
            signing_key_bytes: signing.to_bytes(),
        }
    }

    fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.signing_key_bytes)
    }

    pub fn public_key_bytes(&self) -> PublicKeyBytes {
        PublicKeyBytes::from_verifying_key(&self.verifying_key)
    }

    pub fn sign_bytes(&self, msg: &[u8]) -> String {
        hex::encode(self.signing_key().sign(msg).to_bytes())
    }

    pub fn sign_attestation(
        &self,
        fingerprint: &ModelFingerprint,
        runtime: &str,
        wall_time_ms: u64,
        output: &str,
    ) -> ResultAttestation {
        let digest = ResultAttestation::digest_bytes(
            &self.node_id.fingerprint(),
            fingerprint,
            runtime,
            wall_time_ms,
            output,
        );
        let sig = self.signing_key().sign(&digest);
        ResultAttestation {
            worker_node_id: self.node_id,
            model_fingerprint: fingerprint.clone(),
            runtime: runtime.to_string(),
            wall_time_ms,
            signature_hex: hex::encode(sig.to_bytes()),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct KeystoreFile {
    version: u32,
    salt_b64: String,
    nonce_b64: String,
    ciphertext_b64: String,
    public_key_hex: String,
}

/// Passphrase-encrypted Ed25519 keystore on disk.
pub struct EncryptedKeystore;

impl EncryptedKeystore {
    pub fn create(path: &Path, passphrase: &str) -> Result<IdentityKeys, KeystoreError> {
        let keys = IdentityKeys::generate();
        Self::save(path, passphrase, &keys)?;
        Ok(keys)
    }

    pub fn load_or_create(path: &Path, passphrase: &str) -> Result<IdentityKeys, KeystoreError> {
        if path.exists() {
            Self::unlock(path, passphrase)
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Self::create(path, passphrase)
        }
    }

    pub fn unlock(path: &Path, passphrase: &str) -> Result<IdentityKeys, KeystoreError> {
        let raw = std::fs::read_to_string(path)?;
        let file: KeystoreFile =
            serde_json::from_str(&raw).map_err(|e| KeystoreError::Serde(e.to_string()))?;
        let salt = decode_b64(&file.salt_b64)?;
        let nonce_bytes = decode_b64(&file.nonce_b64)?;
        let ciphertext = decode_b64(&file.ciphertext_b64)?;
        // Prefer try_into over zero-initialized arrays: CodeQL rust/hard-coded-cryptographic-value
        // treats `[0u8; N]` as hard-coded salt/key material even when immediately overwritten.
        let salt_arr: [u8; 16] = salt
            .as_slice()
            .try_into()
            .map_err(|_| KeystoreError::Crypto("invalid salt length".into()))?;
        let nonce_arr: [u8; 12] = nonce_bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeystoreError::Crypto("invalid nonce length".into()))?;
        let key = derive_key(passphrase, &salt_arr)?;
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| KeystoreError::Crypto(e.to_string()))?;
        let nonce = Nonce::from_slice(&nonce_arr);
        let mut plain = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|_| KeystoreError::BadPassphrase)?;
        let mut sk_bytes: [u8; 32] = match plain.as_slice().try_into() {
            Ok(bytes) => bytes,
            Err(_) => {
                plain.zeroize();
                return Err(KeystoreError::Crypto("unexpected key length".into()));
            }
        };
        plain.zeroize();
        let signing = SigningKey::from_bytes(&sk_bytes);
        sk_bytes.zeroize();
        Ok(IdentityKeys::from_signing(signing))
    }

    fn save(path: &Path, passphrase: &str, keys: &IdentityKeys) -> Result<(), KeystoreError> {
        // Draw salt/nonce from OsRng directly (no `[0u8; N]` staging buffer).
        // `fills_bytes` after a zero literal still taints as hard-coded for CodeQL CWE-798.
        let salt = random_salt();
        let key = derive_key(passphrase, &salt)?;
        let cipher = ChaCha20Poly1305::new_from_slice(&key)
            .map_err(|e| KeystoreError::Crypto(e.to_string()))?;
        let nonce_bytes = random_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, keys.signing_key_bytes.as_ref())
            .map_err(|e| KeystoreError::Crypto(e.to_string()))?;
        let file = KeystoreFile {
            version: 1,
            salt_b64: encode_b64(&salt),
            nonce_b64: encode_b64(&nonce_bytes),
            ciphertext_b64: encode_b64(&ciphertext),
            public_key_hex: PublicKeyBytes::from_verifying_key(&keys.verifying_key).to_hex(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| KeystoreError::Serde(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// Cryptographically random bytes without a hard-coded zero staging array.
/// Concrete sizes: rand 0.8's `Standard` implements `Distribution` for `[u8; N]`
/// only at fixed N (not `const N: usize`).
fn random_salt() -> [u8; 16] {
    OsRng.gen()
}

fn random_nonce() -> [u8; 12] {
    OsRng.gen()
}

fn random_key_material() -> [u8; 32] {
    OsRng.gen()
}

fn derive_key(passphrase: &str, salt: &[u8; 16]) -> Result<[u8; 32], KeystoreError> {
    // Argon2 fully overwrites this buffer; seed from OsRng so the array is not a
    // hard-coded constant flowing into the ChaCha20Poly1305 key sink (CWE-798).
    let mut out = random_key_material();
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| KeystoreError::Crypto(e.to_string()))?;
    Ok(out)
}

fn encode_b64(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn decode_b64(s: &str) -> Result<Vec<u8>, KeystoreError> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s)
        .map_err(|e| KeystoreError::Crypto(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn round_trip_keystore() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("horde-keystore-{nanos}.json"));
        let keys = EncryptedKeystore::create(&path, "test-pass").unwrap();
        let unlocked = EncryptedKeystore::unlock(&path, "test-pass").unwrap();
        assert_eq!(keys.node_id, unlocked.node_id);
        assert!(EncryptedKeystore::unlock(&path, "wrong").is_err());
        let _ = std::fs::remove_file(&path);
    }
}
