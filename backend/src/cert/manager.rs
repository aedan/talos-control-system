use std::sync::Arc;
use chrono::{DateTime, Utc, Duration};
use crate::cert::CertError;
use crate::config::tls::{TlsConfig, TlsMode};
use crate::cert::self_signed;
use crate::cert::provided;

pub struct CertificateManager {
    cert_data: Arc<(String, String)>,
    mode: TlsMode,
    expires_at: Option<DateTime<Utc>>,
}

impl CertificateManager {
    pub async fn new(config: &TlsConfig) -> Result<Self, CertError> {
        let mode = config.mode.clone();

        tracing::info!(?mode, "Initializing CertificateManager");

        let (cert_pem, key_pem) = Self::load_certificates(config).await?;

        let expires_at = if mode == TlsMode::SelfSigned {
            None
        } else {
            provided::parse_expiry_from_cert_pem(&cert_pem)
        };

        Ok(Self {
            cert_data: Arc::new((cert_pem, key_pem)),
            mode,
            expires_at,
        })
    }

    async fn load_certificates(config: &TlsConfig) -> Result<(String, String), CertError> {
        match config.mode {
            TlsMode::LetsEncrypt => {
                let store_dir = config.letsencrypt.as_ref()
                    .and_then(|le| le.dns_provider.as_ref())
                    .map(|_| "/var/lib/tcs/certs".to_string())
                    .unwrap_or_else(|| "/var/lib/tcs/certs".to_string());

                match Self::load_from_disk(&store_dir) {
                    Ok(certs) => {
                        tracing::info!("Loaded existing Let's Encrypt certificates from disk");
                        Ok(certs)
                    }
                    Err(_) => {
                        tracing::warn!("No existing Let's Encrypt certificates found, ACME issuance required");
                        Err(CertError::Acme("ACME certificate issuance not yet implemented; place existing certs in cert store directory".to_string()))
                    }
                }
            }
            TlsMode::SelfSigned => {
                let domains = config.self_signed.as_ref()
                    .map(|ss| ss.domains.clone())
                    .unwrap_or_else(|| vec!["localhost".to_string()]);

                tracing::info!(domains = ?domains, "Generating self-signed certificate");
                self_signed::generate_self_signed(&domains).await
            }
            TlsMode::Provided => {
                let provided_config = config.provided.as_ref()
                    .ok_or_else(|| CertError::Config("Provided TLS mode selected but no provided cert config found".to_string()))?;

                load_provided_certs_from_config(provided_config).await
            }
            TlsMode::Disabled => {
                Err(CertError::Config("TLS is disabled, no certificates to load".to_string()))
            }
        }
    }

    pub fn get_expiry(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }

    pub fn should_renew(&self) -> bool {
        match self.expires_at {
            Some(expiry) => {
                let thirty_days = Duration::days(30);
                let renewal_threshold = Utc::now() + thirty_days;
                let should = expiry <= renewal_threshold;
                if should {
                    tracing::warn!(expires_at = ?expiry, "Certificate expires within 30 days, renewal needed");
                }
                should
            }
            None => false,
        }
    }

    pub fn save_to_disk(&self, cert_pem: &str, key_pem: &str, store_dir: &str) -> Result<(), CertError> {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(store_dir)
            .map_err(|e| CertError::Io(e))?;

        let cert_path = format!("{}/tls.crt", store_dir);
        let key_path = format!("{}/tls.key", store_dir);

        std::fs::write(&cert_path, cert_pem)
            .map_err(|e| CertError::Io(e))?;

        std::fs::write(&key_path, key_pem)
            .map_err(|e| CertError::Io(e))?;

        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| CertError::Io(e))?;

        tracing::info!(store_dir, "TLS certificates saved to disk");

        Ok(())
    }

    pub fn load_from_disk(store_dir: &str) -> Result<(String, String), CertError> {
        let cert_path = format!("{}/tls.crt", store_dir);
        let key_path = format!("{}/tls.key", store_dir);

        let cert = std::fs::read_to_string(&cert_path)
            .map_err(|e| CertError::Io(e))?;
        let key = std::fs::read_to_string(&key_path)
            .map_err(|e| CertError::Io(e))?;

        if cert.is_empty() || key.is_empty() {
            return Err(CertError::Config("Certificates on disk are empty".to_string()));
        }

        tracing::info!(store_dir, "TLS certificates loaded from disk");

        Ok((cert, key))
    }

    pub fn get_cert_pem(&self) -> &str {
        &self.cert_data.0
    }

    pub fn get_key_pem(&self) -> &str {
        &self.cert_data.1
    }

    pub fn get_mode(&self) -> &TlsMode {
        &self.mode
    }

    pub fn update_certificates(&self, cert_pem: String, key_pem: String, expires_at: Option<DateTime<Utc>>) {
        let _ = cert_pem;
        let _ = key_pem;
        let _ = expires_at;
        tracing::warn!("CertificateManager uses Arc, update_certificates is a no-op. Reinitialize the manager instead.");
    }
}

async fn load_provided_certs_from_config(config: &crate::config::tls::ProvidedCertConfig) -> Result<(String, String), CertError> {
    provided::load_provided_certs(&config.cert_path, &config.key_path).await
}
