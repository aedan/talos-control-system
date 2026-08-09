//! Encrypt sensitive blobs at rest (talosconfig, kubeconfig) using AES-256-GCM.
//! Key material is derived from the JWT secret via SHA-256.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::AppError;

const PREFIX: &str = "tcsenc:v1:";

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

    let key = key_from_jwt_secret(jwt_secret);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::Internal(format!("AES key error: {}", e)))?;

    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
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

/// Decrypt a value produced by [`encrypt`]. Plaintext legacy values pass through.
pub fn decrypt(jwt_secret: &str, stored: &str) -> Result<String, AppError> {
    if stored.is_empty() {
        return Ok(String::new());
    }
    if !stored.starts_with(PREFIX) {
        // Legacy plaintext from earlier alpha builds
        return Ok(stored.to_string());
    }

    let b64 = &stored[PREFIX.len()..];
    let packed = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| AppError::Internal(format!("Decrypt decode failed: {}", e)))?;
    if packed.len() < 13 {
        return Err(AppError::Internal("Corrupt encrypted blob".to_string()));
    }

    let (nonce_bytes, ciphertext) = packed.split_at(12);
    let key = key_from_jwt_secret(jwt_secret);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::Internal(format!("AES key error: {}", e)))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    let plain = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AppError::Internal(
            "Decrypt failed — JWT secret may have changed since secrets were stored".to_string(),
        ))?;

    String::from_utf8(plain)
        .map_err(|e| AppError::Internal(format!("Decrypted data is not UTF-8: {}", e)))
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
        assert_eq!(
            decrypt("x", "context: foo\n").unwrap(),
            "context: foo\n"
        );
    }
}
