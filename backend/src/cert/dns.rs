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
    domain: String,
    api_key: String,
    api_secret: String,
    client: reqwest::Client,
}

impl GoDaddyProvider {
    pub fn new(domain: &str, api_key: &str, api_secret: &str) -> Self {
        Self {
            domain: domain.to_string(),
            api_key: api_key.to_string(),
            api_secret: api_secret.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl DnsProvider for GoDaddyProvider {
    async fn add_txt_record(&self, _domain: &str, name: &str, value: &str) -> Result<(), CertError> {
        let url = format!(
            "https://api.godaddy.com/v1/domains/{}/records/TXT/{}",
            self.domain, name
        );
        
        let record = serde_json::json!([{
            "data": [value],
            "ttl": 3600,
        }]);

        let resp = self.client
            .put(&url)
            .header("Authorization", format!("sso-key {}:{}", self.api_key, self.api_secret))
            .json(&record)
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("GoDaddy request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("GoDaddy API error {}: {}", status, body)));
        }

        tracing::info!(domain = %self.domain, record = name, "Added TXT record via GoDaddy");
        Ok(())
    }

    async fn remove_txt_record(&self, _domain: &str, name: &str) -> Result<(), CertError> {
        let url = format!(
            "https://api.godaddy.com/v1/domains/{}/records/TXT/{}",
            self.domain, name
        );

        let resp = self.client
            .delete(&url)
            .header("Authorization", format!("sso-key {}:{}", self.api_key, self.api_secret))
            .send()
            .await
            .map_err(|e| CertError::Dns(format!("GoDaddy request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(CertError::Dns(format!("GoDaddy API error {}: {}", status, body)));
        }

        tracing::info!(domain = %self.domain, record = name, "Removed TXT record via GoDaddy");
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
