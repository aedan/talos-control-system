use std::fs;
use crate::cert::CertError;

pub async fn load_provided_certs(cert_path: &str, key_path: &str) -> Result<(String, String), CertError> {
    tracing::info!(cert_path, key_path, "Loading provided TLS certificates");

    let cert = fs::read_to_string(cert_path)
        .map_err(|e| CertError::Io(e))?;
    let key = fs::read_to_string(key_path)
        .map_err(|e| CertError::Io(e))?;

    if cert.trim().is_empty() || key.trim().is_empty() {
        return Err(CertError::Config("Cert and key files must not be empty".to_string()));
    }

    tracing::info!("Provided TLS certificates loaded successfully");

    Ok((cert, key))
}

pub fn parse_expiry_from_cert_pem(pem: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use rustls_pemfile::certs;
    use std::io::Cursor;

    let mut cursor = Cursor::new(pem.as_bytes());
    let cert_items: Vec<_> = certs(&mut cursor).collect();

    if cert_items.is_empty() {
        tracing::warn!("No certificates found in PEM data");
        return None;
    }

    for item in cert_items {
        let der = match item {
            Ok(der) => der,
            Err(e) => {
                tracing::warn!("Failed to read certificate from PEM: {}", e);
                return None;
            }
        };
        match parse_x509_not_after(&der) {
            Some(expiry) => {
                tracing::info!(expiry = ?expiry, "Certificate expiry parsed from PEM");
                return Some(expiry);
            }
            None => {
                tracing::warn!("Failed to parse x509 certificate to extract expiry");
                return None;
            }
        }
    }

    None
}

fn parse_x509_not_after(der: &[u8]) -> Option<chrono::DateTime<chrono::Utc>> {
    match x509_parser::parse_x509_certificate(der) {
        Ok((_, cert)) => {
            let not_after = cert.validity().not_after;
            let dt = not_after.to_datetime();
            Some(chrono::DateTime::<chrono::Utc>::from_timestamp(dt.unix_timestamp(), 0)?)
        }
        Err(e) => {
            tracing::warn!("Failed to parse DER certificate with x509-parser: {}", e);
            None
        }
    }
}
