use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::AppError;

pub struct TalosClient {
    node_address: String,
    ca: Vec<u8>,
    crt: Vec<u8>,
    key: Vec<u8>,
    insecure: bool,
    connected: Arc<RwLock<bool>>,
    client: Arc<RwLock<Option<reqwest::Client>>>,
}

impl TalosClient {
    pub fn new(node_address: String, ca: Vec<u8>, crt: Vec<u8>, key: Vec<u8>) -> Self {
        Self {
            node_address,
            ca,
            crt,
            key,
            insecure: false,
            connected: Arc::new(RwLock::new(false)),
            client: Arc::new(RwLock::new(None)),
        }
    }

    pub fn with_insecure(node_address: String) -> Self {
        Self {
            node_address,
            ca: Vec::new(),
            crt: Vec::new(),
            key: Vec::new(),
            insecure: true,
            connected: Arc::new(RwLock::new(false)),
            client: Arc::new(RwLock::new(None)),
        }
    }

    fn build_client(&self) -> Result<reqwest::Client, AppError> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10));

        if self.insecure {
            builder = builder.danger_accept_invalid_certs(true);
        } else if !self.ca.is_empty() && !self.crt.is_empty() && !self.key.is_empty() {
            let ca_cert = reqwest::Certificate::from_pem(&self.ca)
                .map_err(|e| AppError::Internal(format!("Invalid CA cert: {}", e)))?;
            let client_cert = reqwest::Certificate::from_pem(&self.crt)
                .map_err(|e| AppError::Internal(format!("Invalid client cert: {}", e)))?;
            let combined_pem = [&self.crt[..], &b"\n"[..], &self.key[..]].concat();
            let client_key = reqwest::Identity::from_pem(&combined_pem)
                .map_err(|e| AppError::Internal(format!("Invalid client identity: {}", e)))?;
            builder = builder
                .add_root_certificate(ca_cert)
                .identity(client_key);
        }

        let client = builder
            .build()
            .map_err(|e| AppError::Internal(format!("Failed to build HTTP client: {}", e)))?;

        Ok(client)
    }

    async fn get_client(&self) -> Result<reqwest::Client, AppError> {
        if let Some(client) = &*self.client.read().await {
            return Ok(client.clone());
        }
        let client = self.build_client()?;
        *self.client.write().await = Some(client.clone());
        Ok(client)
    }

    async fn request<T>(&self, method: &str, url: &str, body: Option<&str>) -> Result<T, AppError>
    where
        T: serde::de::DeserializeOwned,
    {
        let client = self.get_client().await?;
        let mut req = client.request(method.parse().unwrap_or(reqwest::Method::GET), url);

        if let Some(b) = body {
            req = req.body(b.to_string()).header("Content-Type", "application/json");
        }

        let response = req.send()
            .await
            .map_err(|e| AppError::Network(format!("Request to {} failed: {}", url, e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return Err(AppError::Network(format!("API error {} from {}: {}", status.as_u16(), url, body_text)));
        }

        response.json().await
            .map_err(|e| AppError::Internal(format!("Failed to parse response: {}", e)))
    }

    pub async fn connect(&self) -> Result<(), AppError> {
        let url = format!("https://{}/v1alpha1/version", self.node_address);
        tracing::info!(url, "Connecting to Talos API");
        let _client = self.build_client()?;
        *self.client.write().await = Some(_client);

        // Test the connection
        let test_url = format!("https://{}/v1alpha1/version", self.node_address);
        match self.request::<serde_json::Value>("GET", &test_url, None).await {
            Ok(_) => {
                *self.connected.write().await = true;
                tracing::info!(node = %self.node_address, "Talos API connection established");
            }
            Err(e) => {
                warn!(node = %self.node_address, error = %e, "Talos API connection failed (non-fatal, will retry on use)");
                *self.connected.write().await = true;
            }
        }

        Ok(())
    }

    pub async fn get_machine_config(&self) -> Result<String, AppError> {
        self.ensure_connected().await?;
        let url = format!("https://{}/v1alpha1/machine/config", self.node_address);
        let response: serde_json::Value = self.request("GET", &url, None).await?;
        let config_str = response.get("config")
            .and_then(|c| c.as_str())
            .unwrap_or_else(|| response.get("nodeConfig")
                .and_then(|c| c.as_str())
                .unwrap_or(""));

        if config_str.is_empty() {
            return Ok(response.to_string());
        }

        Ok(config_str.to_string())
    }

    pub async fn apply_config(&self, config: &str) -> Result<(), AppError> {
        self.ensure_connected().await?;
        let url = format!("https://{}/v1alpha1/machine/config", self.node_address);
        let body = serde_json::json!({ "config": config }).to_string();
        let response: serde_json::Value = self.request("PUT", &url, Some(&body)).await?;
        tracing::info!(node = %self.node_address, status = response.get("status").and_then(|s| s.as_str()).unwrap_or("unknown"), "Config applied");
        Ok(())
    }

    pub async fn reboot(&self) -> Result<(), AppError> {
        self.ensure_connected().await?;
        let url = format!("https://{}/v1alpha1/reboot", self.node_address);
        self.request::<serde_json::Value>("POST", &url, None).await?;
        info!(node = %self.node_address, "Reboot initiated");
        Ok(())
    }

    pub async fn upgrade(&self, version: &str, image: &str) -> Result<(), AppError> {
        self.ensure_connected().await?;
        let url = format!("https://{}/v1alpha1/upgrade", self.node_address);
        let body = serde_json::json!({
            "version": version,
            "image": image,
        }).to_string();
        self.request::<serde_json::Value>("POST", &url, Some(&body)).await?;
        info!(node = %self.node_address, version, image, "Upgrade initiated");
        Ok(())
    }

    pub async fn get_version(&self) -> Result<String, AppError> {
        self.ensure_connected().await?;
        let url = format!("https://{}/v1alpha1/version", self.node_address);
        let response: serde_json::Value = self.request("GET", &url, None).await?;
        let version = response.get("talosVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        Ok(version)
    }

    pub async fn disconnect(&self) {
        *self.connected.write().await = false;
        *self.client.write().await = None;
    }

    async fn ensure_connected(&self) -> Result<(), AppError> {
        if !*self.connected.read().await {
            self.connect().await?;
        }
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        *self.connected.blocking_read()
    }

    pub fn node_address(&self) -> &str {
        &self.node_address
    }
}
