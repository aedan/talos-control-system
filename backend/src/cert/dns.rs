use thiserror::Error;

#[derive(Error, Debug)]
pub enum CertError {
    #[error("DNS provider error: {0}")]
    Dns(String),
    #[error("ACME error: {0}")]
    Acme(String),
    #[error("File I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Certificate error: {0}")]
    Cert(String),
    #[error("Configuration error: {0}")]
    Config(String),
}

use async_trait::async_trait;

#[async_trait]
pub trait DnsProvider: Send + Sync {
    async fn add_txt_record(&self, domain: &str, record: &str, value: &str) -> Result<(), CertError>;
    async fn remove_txt_record(&self, domain: &str, record: &str) -> Result<(), CertError>;
}

pub struct GoDaddyProvider {
    api_key: String,
    api_secret: String,
    /// Optional registered-domain override (e.g. "cloudmunchers.net"). When
    /// empty, the zone is derived from each record's FQDN.
    zone: String,
    client: reqwest::Client,
}

impl GoDaddyProvider {
    pub fn new(api_key: &str, api_secret: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            zone: String::new(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_zone(mut self, zone: &str) -> Self {
        self.zone = zone.to_string();
        self
    }

    /// Split a TXT record FQDN into (base_domain, record_name) for GoDaddy's
    /// /v1/domains/{base}/records/TXT/{name} API. GoDaddy accepts the full FQDN
    /// as `record_name`, so we only need the correct base (registered) domain.
    fn split_zone(&self, fqdn: &str) -> (String, String) {
        let record = format!("_acme-challenge.{}", fqdn.trim_end_matches('.'));
        let base = if !self.zone.is_empty() {
            self.zone.clone()
        } else {
            // Derive: use the last two labels, or three if it looks like a
            // two-part TLD (co.uk, com.au, ...). Good enough for the common
            // case; an explicit zone override always wins.
            let labels: Vec<&str> = fqdn.split('.').collect();
            let take = if labels.len() >= 3 {
                let maybe_tld = format!("{}.{}", labels[labels.len() - 2], labels[labels.len() - 1]);
                if matches!(
                    maybe_tld.as_str(),
                    "co.uk" | "com.au" | "co.jp" | "com.br" | "com.mx" | "co.nz" | "co.in" | "com.hk"
                ) {
                    3
                } else {
                    2
                }
            } else {
                labels.len()
            };
            labels[labels.len().saturating_sub(take)..].join(".")
        };
        (base, record)
    }
}

#[async_trait]
impl DnsProvider for GoDaddyProvider {
    async fn add_txt_record(&self, challenge_domain: &str, record: &str, value: &str) -> Result<(), CertError> {
        let (base, name) = self.split_zone(challenge_domain);
        // `record` is the full TXT FQDN (e.g. _acme-challenge.tcs.kronos.…).
        // GoDaddy accepts the full FQDN as the record name under its base domain.
        let url = format!(
            "https://api.godaddy.com/v1/domains/{}/records/TXT/{}",
            base, record
        );

        let record_body = serde_json::json!([{
            "data": value,
            "ttl": 600,
        }]);

        let resp = self.client
            .put(&url)
            .header("Authorization", format!("sso-key {}:{}", self.api_key, self.api_secret))
            .json(&record_body)
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("GoDaddy request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("GoDaddy API error {} (base={base}, record={name}): {body}", status)));
        }

        tracing::info!(domain = challenge_domain, base, record = name, "Added TXT record via GoDaddy");
        Ok(())
    }

    async fn remove_txt_record(&self, challenge_domain: &str, record: &str) -> Result<(), CertError> {
        let (base, name) = self.split_zone(challenge_domain);
        let url = format!(
            "https://api.godaddy.com/v1/domains/{}/records/TXT/{}",
            base, record
        );

        let resp = self.client
            .delete(&url)
            .header("Authorization", format!("sso-key {}:{}", self.api_key, self.api_secret))
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("GoDaddy request failed: {}", e)))?;

        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NOT_FOUND {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("GoDaddy delete API error {}: {}", status, body)));
        }

        tracing::info!(domain = challenge_domain, base, record = name, "Removed TXT record via GoDaddy");
        Ok(())
    }
}

pub struct CloudflareProvider {
    api_token: String,
    zone_id: String,
    client: reqwest::Client,
}

impl CloudflareProvider {
    pub fn new(api_token: &str, zone_id: &str) -> Self {
        Self {
            api_token: api_token.to_string(),
            zone_id: zone_id.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DnsProvider for CloudflareProvider {
    async fn add_txt_record(&self, _domain: &str, name: &str, value: &str) -> Result<(), CertError> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            self.zone_id
        );

        let resp = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .query(&[("type", "TXT"), ("name", name)])
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("Cloudflare request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("Cloudflare API error {}: {}", status, body)));
        }

        #[derive(serde::Deserialize)]
        struct CfResponse {
            result: Vec<CfRecord>,
        }
        #[derive(serde::Deserialize)]
        struct CfRecord {
            id: String,
        }

        let cf_resp: CfResponse = resp.json().await
            .map_err(|e| CertError::Dns(format!("Failed to parse Cloudflare response: {}", e)))?;

        for record in cf_resp.result {
            let delete_url = format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
                self.zone_id, record.id
            );

            self.client
                .delete(&delete_url)
                .header("Authorization", format!("Bearer {}", self.api_token))
                .send()
                .await
                .map_err(|e| CertError::Dns(format!("Cloudflare record delete failed: {}", e)))?;
        }

        let create_resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .json(&serde_json::json!({
                "type": "TXT",
                "name": name,
                "content": format!("\"{}\"", value),
                "ttl": 120,
                "proxied": false,
                "comment": "Managed by TCS cert manager",
            }))
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("Cloudflare record create failed: {}", e)))?;

        if !create_resp.status().is_success() {
            let status = create_resp.status();
            let body = create_resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("Cloudflare API error {}: {}", status, body)));
        }

        tracing::info!(record = name, "Added TXT record via Cloudflare");
        Ok(())
    }

    async fn remove_txt_record(&self, _domain: &str, name: &str) -> Result<(), CertError> {
        let url = format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            self.zone_id
        );

        let resp = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_token))
            .query(&[("type", "TXT"), ("name", name)])
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("Cloudflare request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("Cloudflare API error {}: {}", status, body)));
        }

        #[derive(serde::Deserialize)]
        struct CfResponse {
            result: Vec<CfRecord>,
        }
        #[derive(serde::Deserialize)]
        struct CfRecord {
            id: String,
        }

        let cf_resp: CfResponse = resp.json().await
            .map_err(|e| CertError::Dns(format!("Failed to parse Cloudflare response: {}", e)))?;

        for record in cf_resp.result {
            let delete_url = format!(
                "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
                self.zone_id, record.id
            );

            self.client
                .delete(&delete_url)
                .header("Authorization", format!("Bearer {}", self.api_token))
                .send()
                .await
                .map_err(|e| CertError::Dns(format!("Cloudflare record delete failed: {}", e)))?;
        }

        tracing::info!(record = name, "Removed TXT record via Cloudflare");
        Ok(())
    }
}

pub struct Route53Provider;

#[async_trait]
impl DnsProvider for Route53Provider {
    async fn add_txt_record(&self, _domain: &str, _record: &str, _value: &str) -> Result<(), CertError> {
        Err(CertError::Dns("Route53 provider requires AWS credentials".to_string()))
    }

    async fn remove_txt_record(&self, _domain: &str, _record: &str) -> Result<(), CertError> {
        Err(CertError::Dns("Route53 provider requires AWS credentials".to_string()))
    }
}

/// Build a concrete DNS provider from config. Returns an error for unknown or
/// under-configured providers so DNS-01 fails loudly instead of silently
/// producing a self-signed cert.
pub fn build_dns_provider(
    cfg: &crate::config::tls::DnsProviderConfig,
) -> Result<Box<dyn DnsProvider>, CertError> {
    match cfg.provider.trim().to_lowercase().as_str() {
        "godaddy" | "go_daddy" => {
            if cfg.api_key.is_empty() || cfg.api_secret.is_empty() {
                return Err(CertError::Config(
                    "GoDaddy DNS provider requires api_key and api_secret".to_string(),
                ));
            }
            Ok(Box::new(GoDaddyProvider::new(&cfg.api_key, &cfg.api_secret).with_zone(&cfg.dns_zone)))
        }
        "cloudflare" => {
            if cfg.api_token.is_empty() || cfg.zone_id.is_empty() {
                return Err(CertError::Config(
                    "Cloudflare DNS provider requires api_token and zone_id".to_string(),
                ));
            }
            Ok(Box::new(CloudflareProvider::new(&cfg.api_token, &cfg.zone_id)))
        }
        "route53" | "aws" => Ok(Box::new(Route53Provider)),
        other => Err(CertError::Config(format!(
            "Unsupported DNS provider: {other} (supported: godaddy, cloudflare, route53)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn godaddy_zone_split_two_label_tld() {
        let p = GoDaddyProvider::new("k", "s");
        let (base, rec) = p.split_zone("tcs.kronos.cloudmunchers.net");
        assert_eq!(base, "cloudmunchers.net");
        assert_eq!(rec, "_acme-challenge.tcs.kronos.cloudmunchers.net");
    }

    #[test]
    fn godaddy_zone_split_explicit_override_wins() {
        let p = GoDaddyProvider::new("k", "s").with_zone("kronos.cloudmunchers.net");
        let (base, rec) = p.split_zone("tcs.kronos.cloudmunchers.net");
        assert_eq!(base, "kronos.cloudmunchers.net");
        assert_eq!(rec, "_acme-challenge.tcs.kronos.cloudmunchers.net");
    }

    #[test]
    fn godaddy_zone_split_apex_domain() {
        let p = GoDaddyProvider::new("k", "s");
        let (base, rec) = p.split_zone("cloudmunchers.net");
        assert_eq!(base, "cloudmunchers.net");
        assert_eq!(rec, "_acme-challenge.cloudmunchers.net");
    }

    #[test]
    fn build_provider_rejects_missing_credentials() {
        let cfg = crate::config::tls::DnsProviderConfig {
            provider: "godaddy".into(),
            api_key: "".into(),
            api_secret: "".into(),
            api_token: "".into(),
            zone_id: "".into(),
            dns_zone: "".into(),
        };
        assert!(matches!(
            build_dns_provider(&cfg),
            Err(CertError::Config(_))
        ));
    }

    #[test]
    fn build_provider_unknown_name_errors() {
        let cfg = crate::config::tls::DnsProviderConfig {
            provider: "aws-route53-typos".into(),
            api_key: "k".into(),
            api_secret: "s".into(),
            api_token: "".into(),
            zone_id: "".into(),
            dns_zone: "".into(),
        };
        assert!(matches!(
            build_dns_provider(&cfg),
            Err(CertError::Config(_))
        ));
    }
}
