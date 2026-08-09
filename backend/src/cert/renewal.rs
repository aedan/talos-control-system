use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::cert::acme::AcmeClient;
use crate::cert::provided::{load_provided_certs, parse_expiry_from_cert_pem};
use crate::cert::self_signed::generate_self_signed;
use crate::cert::CertError;
use crate::config::{Config, TlsMode};

pub const RENEWAL_CHECK_INTERVAL: Duration = Duration::from_secs(86400);
pub const RENEWAL_THRESHOLD_DAYS: i64 = 30;

fn needs_renewal(cert_pem: &str) -> bool {
    match parse_expiry_from_cert_pem(cert_pem) {
        Some(expiry) => {
            let now = chrono::Utc::now();
            let remaining = expiry - now;
            remaining.num_days() <= RENEWAL_THRESHOLD_DAYS
        }
        None => {
            warn!("Could not parse certificate expiry, scheduling renewal");
            true
        }
    }
}

pub async fn start_cert_renewal_task(
    config: Config,
    acme_store: Option<Arc<DashMap<String, String>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Certificate renewal monitoring task started");

    let mut interval = interval(RENEWAL_CHECK_INTERVAL);
    interval.tick().await;

    loop {
        interval.tick().await;
        info!("Checking certificate renewal status...");

        let tls = &config.tls;
        let cert_path = "/var/lib/tcs/certs/cert.pem";

        let should_renew = match std::fs::read_to_string(cert_path) {
            Ok(pem) => needs_renewal(&pem),
            Err(_) => {
                warn!(path = %cert_path, "Certificate file not found, will attempt renewal");
                true
            }
        };

        if !should_renew {
            info!("Certificate renewal not needed");
            continue;
        }

        info!("Certificate renewal needed, proceeding...");

        match &tls.mode {
            TlsMode::LetsEncrypt => {
                if let Some(le) = &tls.letsencrypt {
                    let acme = AcmeClient::new(
                        &le.email,
                        le.dns_provider
                            .as_ref()
                            .map(|d| crate::config::tls::DnsProviderConfig {
                                provider: d.provider.clone(),
                                api_key: d.api_key.clone(),
                                api_secret: d.api_secret.clone(),
                                api_token: d.api_token.clone(),
                                zone_id: d.zone_id.clone(),
                            }),
                        le.challenge_type.clone(),
                    );

                    match acme {
                        Ok(acme_client) => {
                            let result = if let Some(store) = &acme_store {
                                acme_client.renew_certificate_with_store(&le.domains, store).await
                            } else {
                                acme_client.renew_certificate(&le.domains).await
                            };

                            match result {
                                Ok((new_cert, new_key)) => {
                                    if let Err(e) = write_cert_to_disk(&new_cert, &new_key) {
                                        error!(error = %e, "Failed to write renewed cert to disk");
                                    } else {
                                        info!("Let's Encrypt certificate renewed successfully");
                                    }
                                }
                                Err(e) => error!(error = %e, "Let's Encrypt renewal failed"),
                            }
                        }
                        Err(e) => error!(error = %e, "Failed to create ACME client for renewal"),
                    }
                } else {
                    error!("Let's Encrypt not configured");
                }
            }
            TlsMode::SelfSigned => {
                let domains = tls
                    .self_signed
                    .as_ref()
                    .map(|c| c.domains.clone())
                    .unwrap_or_else(|| vec!["localhost".to_string()]);

                match generate_self_signed(&domains).await {
                    Ok((new_cert, new_key)) => {
                        if let Err(e) = write_cert_to_disk(&new_cert, &new_key) {
                            error!(error = %e, "Failed to write renewed self-signed cert");
                        } else {
                            info!("Self-signed certificate renewed successfully");
                        }
                    }
                    Err(e) => error!(error = %e, "Self-signed renewal failed"),
                }
            }
            TlsMode::Provided => {
                if let Some(provided) = &tls.provided {
                    match load_provided_certs(&provided.cert_path, &provided.key_path).await {
                        Ok((cert, key)) => {
                            if let Err(e) = write_cert_to_disk(&cert, &key) {
                                error!(error = %e, "Failed to write provided cert");
                            } else {
                                info!("Provided certificate reloaded successfully");
                            }
                        }
                        Err(e) => error!(error = %e, "Provided cert reload failed"),
                    }
                } else {
                    error!("Provided TLS not configured");
                }
            }
            TlsMode::Disabled => {
                warn!("TLS mode is Disabled, no renewal needed");
            }
        }
    }
}

fn write_cert_to_disk(cert: &str, key: &str) -> Result<(), CertError> {
    std::fs::create_dir_all("/var/lib/tcs/certs/")?;
    std::fs::write("/var/lib/tcs/certs/cert.pem", cert)?;
    std::fs::write("/var/lib/tcs/certs/key.pem", key)?;
    Ok(())
}