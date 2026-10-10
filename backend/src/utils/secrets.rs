//! Encrypt sensitive blobs at rest (talosconfig, kubeconfig, BMC) using AES-256-GCM.
//!
//! When the process has opened the database vault, encryption uses that data
//! key. Otherwise the key is derived from the JWT secret via SHA-256, which is
//! how blobs written before the vault were sealed. The first vault is seeded
//! with that same derived key so existing ciphertext still opens.

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::AppError;

/// Data key installed by [`crate::utils::vault`] after a successful open.
static VAULT_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Use the vault data key for later [`encrypt`] / [`decrypt`] calls.
/// A second call is ignored so a running process cannot swap keys.
pub fn set_vault_key(key: [u8; 32]) {
    let _ = VAULT_KEY.set(key);
}

fn active_key(jwt_secret: &str) -> [u8; 32] {
    VAULT_KEY
        .get()
        .copied()
        .unwrap_or_else(|| key_from_jwt_secret(jwt_secret))
}

const PREFIX: &str = "tcsenc:v1:";

/// Encrypt with a raw 32-byte key. Same `tcsenc:v1:` framing as [`encrypt`].
pub fn encrypt_with_key(key: &[u8; 32], plaintext: &str) -> Result<String, AppError> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    if plaintext.starts_with(PREFIX) {
        return Ok(plaintext.to_string());
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("AES key error: {}", e)))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|e| AppError::Internal(format!("Encrypt failed: {}", e)))?;
    let mut packed = Vec::with_capacity(12 + ciphertext.len());
    packed.extend_from_slice(&nonce_bytes);
    packed.extend_from_slice(&ciphertext);
    Ok(format!(
        "{}{}",
        PREFIX,
        base64::engine::general_purpose::STANDARD.encode(packed)
    ))
}

/// Decrypt a value produced by [`encrypt_with_key`].
pub fn decrypt_with_key(key: &[u8; 32], stored: &str) -> Result<String, AppError> {
    if stored.is_empty() {
        return Ok(String::new());
    }
    if !stored.starts_with(PREFIX) {
        return Err(AppError::Internal(
            "Vault ciphertext is not encrypted".into(),
        ));
    }
    let packed = base64::engine::general_purpose::STANDARD
        .decode(&stored[PREFIX.len()..])
        .map_err(|e| AppError::Internal(format!("Decrypt decode failed: {}", e)))?;
    if packed.len() < 13 {
        return Err(AppError::Internal("Corrupt encrypted blob".into()));
    }
    let (nonce_bytes, ciphertext) = packed.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("AES key error: {}", e)))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| AppError::Internal("Decrypt failed — wrong vault key".into()))?;
    String::from_utf8(plain)
        .map_err(|e| AppError::Internal(format!("Decrypted data is not UTF-8: {}", e)))
}

/// Derive a 256-bit AES key from the application JWT secret.
pub fn key_from_jwt_secret(jwt_secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"tcs-secrets-v1:");
    hasher.update(jwt_secret.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

/// Encrypt plaintext; returns a versioned base64 string safe for SQLite TEXT.
pub fn encrypt(jwt_secret: &str, plaintext: &str) -> Result<String, AppError> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    // Already encrypted?
    if plaintext.starts_with(PREFIX) {
        return Ok(plaintext.to_string());
    }

    encrypt_with_key(&active_key(jwt_secret), plaintext)
}

/// Decrypt a value produced by [`encrypt`]. Plaintext legacy values pass through.
pub fn decrypt(jwt_secret: &str, stored: &str) -> Result<String, AppError> {
    if stored.is_empty() {
        return Ok(String::new());
    }
    if !stored.starts_with(PREFIX) {
        // Legacy plaintext from earlier alpha builds
        return Ok(stored.to_string());
    }

    let using_vault = VAULT_KEY.get().is_some();
    decrypt_with_key(&active_key(jwt_secret), stored).map_err(|e| {
        if e.to_string().contains("wrong vault key") {
            if using_vault {
                AppError::Internal(
                    "Decrypt failed — the vault key does not match this ciphertext".into(),
                )
            } else {
                AppError::Internal(
                    "Decrypt failed — JWT secret may have changed since secrets were stored".into(),
                )
            }
        } else {
            e
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let secret = "test-secret-please-change";
        let enc = encrypt(secret, "hello talosconfig").unwrap();
        assert!(enc.starts_with(PREFIX));
        assert_eq!(decrypt(secret, &enc).unwrap(), "hello talosconfig");
    }

    #[test]
    fn legacy_plaintext() {
        assert_eq!(decrypt("x", "context: foo\n").unwrap(), "context: foo\n");
    }
}
