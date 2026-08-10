//! Live TLS control: reload HTTPS certificates without process restart.
//!
//! Uses a custom `ResolvesServerCert` backed by `Arc<tokio::sync::RwLock>` so the
//! running listener picks up new cert + key on the next TLS handshake — zero
//! downtime, zero rebind.

use std::fmt;
use std::sync::Arc;

use axum_server::tls_rustls::RustlsConfig;
use dashmap::DashMap;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign;
use tokio::sync::RwLock;

use crate::config::tls::{TlsConfig, TlsMode};
use crate::AppError;

pub type AcmeStore = Arc<DashMap<String, String>>;

/// Shared handle for the running HTTPS listener's cert material.
#[derive(Clone)]
pub struct TlsRuntime {
    /// The axum-server TLS config (wraps ServerConfig with our reloadable resolver).
    rustls: RustlsConfig,
    /// The reloadable cert resolver — write to change the cert live.
    resolver: Arc<ReloadableCertResolver>,
    /// HTTP-01 challenge tokens (must already be served on :80).
    pub acme_store: AcmeStore,
    /// Effective TLS settings (updated when Settings saves a new mode).
    pub tls: Arc<RwLock<TlsConfig>>,
    /// Last loaded PEMs (for status / disk write).
    pub certs: Arc<RwLock<(String, String)>>,
    /// Base data dir for cert persistence.
    pub data_dir: String,
}

/// A `ResolvesServerCert` impl backed by an async `RwLock`.
#[derive(Clone)]
struct ReloadableCertResolver {
    key: Arc<RwLock<Arc<sign::CertifiedKey>>>,
}

impl fmt::Debug for ReloadableCertResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReloadableCertResolver")
    }
}

impl ResolvesServerCert for ReloadableCertResolver {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<sign::CertifiedKey>> {
        let guard = self.key.try_read().ok()?;
        Some((*guard).clone())
    }
}

impl ReloadableCertResolver {
    fn new(ck: Arc<sign::CertifiedKey>) -> Self {
        Self {
            key: Arc::new(RwLock::new(ck)),
        }
    }

    async fn update(&self, ck: Arc<sign::CertifiedKey>) {
        let mut guard = self.key.write().await;
        *guard = ck;
    }
}

/// Parse PEM bytes into a rustls `sign::CertifiedKey`.
fn pem_to_certified_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<sign::CertifiedKey>, AppError> {
    let chain: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut &cert_pem[..])
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::InvalidInput(format!("Invalid certificate PEM: {}", e)))?;

    if chain.is_empty() {
        return Err(AppError::InvalidInput("No certificate found in PEM".to_string()));
    }

    let mut key_reader = std::io::Cursor::new(key_pem);
    let key_der = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| AppError::InvalidInput(format!("Invalid private key PEM: {}", e)))?
        .ok_or_else(|| AppError::InvalidInput("No private key found in PEM".to_string()))?;

    let provider = CryptoProvider::get_default().ok_or_else(|| {
        AppError::Internal("No default CryptoProvider configured".to_string())
    })?;

    let signing_key = provider
        .key_provider
        .load_private_key(key_der)
        .map_err(|e| AppError::InvalidInput(format!("Failed to load private key for signing: {}", e)))?;

    Ok(Arc::new(sign::CertifiedKey::new(chain, signing_key)))
}

impl TlsRuntime {
    pub fn new(
        cert_pem: String,
        key_pem: String,
        acme_store: AcmeStore,
        tls: TlsConfig,
        data_dir: String,
    ) -> Result<Self, AppError> {
        let ck = pem_to_certified_key(cert_pem.as_bytes(), key_pem.as_bytes())?;
        let resolver = Arc::new(ReloadableCertResolver::new(ck));

        let server_config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_cert_resolver(resolver.clone()),
        );

        let rustls = RustlsConfig::from_config(server_config);

        Ok(Self {
            rustls,
            resolver,
            acme_store,
            tls: Arc::new(RwLock::new(tls)),
            certs: Arc::new(RwLock::new((cert_pem, key_pem))),
            data_dir,
        })
    }

    /// Get the `RustlsConfig` to pass to `axum_server::bind_rustls`.
    pub fn rustls_config(&self) -> RustlsConfig {
        self.rustls.clone()
    }

    /// Apply a new TLS mode and reload the listener's certificate live.
    pub async fn apply_mode(&self, tls: &TlsConfig) -> Result<String, AppError> {
        let (cert_pem, key_pem, note) = match tls.mode {
            TlsMode::LetsEncrypt => {
                let le = tls.letsencrypt.as_ref().ok_or_else(|| {
                    AppError::InvalidInput("letsencrypt config missing".into())
                })?;
                if le.domains.is_empty() {
                    return Err(AppError::InvalidInput(
                        "letsencrypt domains required".into(),
                    ));
                }
                tracing::info!(domains = ?le.domains, "Live ACME issuance starting");
                match crate::cert::acme::obtain_http01_certificate(
                    &le.domains,
                    &le.email,
                    &self.acme_store,
                )
                .await
                {
                    Ok((c, k)) => (
                        c,
                        k,
                        "Let's Encrypt certificate obtained and applied live".to_string(),
                    ),
                    Err(e) => {
                        tracing::warn!(error = %e, "ACME failed during live apply; keeping current cert");
                        return Err(AppError::Internal(format!(
                            "Let's Encrypt issuance failed: {}. Config was saved; fix ACME (port 80/DNS) and try Apply again — no restart required while HTTPS is running.",
                            e
                        )));
                    }
                }
            }
            TlsMode::SelfSigned => {
                let domains = tls
                    .self_signed
                    .as_ref()
                    .map(|s| s.domains.clone())
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| vec!["localhost".to_string()]);
                let (c, k) = crate::cert::self_signed::generate_self_signed(&domains)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                (c, k, "Self-signed certificate applied live".to_string())
            }
            TlsMode::Provided => {
                let p = tls.provided.as_ref().ok_or_else(|| {
                    AppError::InvalidInput("provided cert paths required".into())
                })?;
                let (c, k) = crate::cert::provided::load_provided_certs(&p.cert_path, &p.key_path)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                (c, k, "Provided certificate loaded and applied live".to_string())
            }
            TlsMode::Disabled => {
                return Err(AppError::InvalidInput(
                    "Cannot disable TLS live while HTTPS is bound; edit config and restart to go HTTP-only".into(),
                ));
            }
        };

        // Update the reloadable cert resolver — next handshake gets the new cert
        let new_ck = pem_to_certified_key(cert_pem.as_bytes(), key_pem.as_bytes())
            .map_err(|e| AppError::Internal(format!("Failed to parse new cert/key: {}", e)))?;
        self.resolver.update(new_ck).await;

        *self.certs.write().await = (cert_pem.clone(), key_pem.clone());
        *self.tls.write().await = tls.clone();

        // Persist PEMs for renewal/status
        let certs_dir = format!("{}/certs", self.data_dir);
        let _ = std::fs::create_dir_all(&certs_dir);
        let _ = std::fs::write(format!("{}/cert.pem", certs_dir), &cert_pem);
        let _ = std::fs::write(format!("{}/key.pem", certs_dir), &key_pem);

        tracing::info!(%note, "TLS configuration applied without process restart");
        Ok(note)
    }
}
