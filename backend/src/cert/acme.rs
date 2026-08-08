use crate::cert::dns::{CloudflareProvider, DnsProvider, GoDaddyProvider};
use crate::cert::CertError;
use crate::config::tls::{ChallengeType, DnsProviderConfig};

pub struct AcmeClient {
    email: String,
    challenge_type: ChallengeType,
    dns_provider: Option<Box<dyn DnsProvider>>,
}

impl AcmeClient {
    pub fn new(
        email: &str,
        dns_config: Option<DnsProviderConfig>,
        challenge_type: ChallengeType,
    ) -> Result<Self, CertError> {
        let dns_provider = match (&challenge_type, &dns_config) {
            (ChallengeType::Dns01, Some(cfg)) => {
                let provider: Box<dyn DnsProvider> = match cfg.provider.as_str() {
                    "godaddy" => Box::new(GoDaddyProvider::new(
                        &cfg.api_key,
                        &cfg.api_secret,
                    )),
                    "cloudflare" => Box::new(CloudflareProvider::new(
                        &cfg.api_token,
                        &cfg.zone_id,
                    )),
                    other => {
                        return Err(CertError::Config(format!(
                            "Unsupported DNS provider: {}",
                            other
                        )))
                    }
                };
                Some(provider)
            }
            _ => None,
        };

        Ok(Self {
            email: email.to_string(),
            challenge_type,
            dns_provider,
        })
    }

    pub async fn obtain_certificate(&self, domains: &[String]) -> Result<(String, String), CertError> {
        if domains.is_empty() {
            return Err(CertError::Config("No domains specified for certificate".to_string()));
        }

        // For now, use rcgen as fallback until acme2 integration is complete
        crate::cert::self_signed::generate_self_signed(domains).await
    }

    pub async fn renew_certificate(&self, domains: &[String]) -> Result<(String, String), CertError> {
        self.obtain_certificate(domains).await
    }
}
