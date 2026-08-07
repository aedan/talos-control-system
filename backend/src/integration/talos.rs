use std::sync::Arc;
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
        }
    }

    pub async fn connect(&self) -> Result<(), AppError> {
        let url = format!("https://{}/v1alpha1", self.node_address);
        tracing::info!(url, "Would connect to Talos API (placeholder)");
        *self.connected.write().await = true;
        Ok(())
    }

    pub async fn get_machine_config(&self) -> Result<String, AppError> {
        self.ensure_connected().await?;
        Ok(String::new())
    }

    pub async fn apply_config(&self, config: &str) -> Result<(), AppError> {
        self.ensure_connected().await?;
        tracing::info!(config_len = config.len(), "Applied machine config (placeholder)");
        Ok(())
    }

    pub async fn reboot(&self) -> Result<(), AppError> {
        self.ensure_connected().await?;
        info!(node = %self.node_address, "Reboot initiated (placeholder)");
        Ok(())
    }

    pub async fn upgrade(&self, version: &str, image: &str) -> Result<(), AppError> {
        self.ensure_connected().await?;
        info!(node = %self.node_address, version, image, "Upgrade initiated (placeholder)");
        Ok(())
    }

    pub async fn get_version(&self) -> Result<String, AppError> {
        self.ensure_connected().await?;
        Ok(String::new())
    }

    pub async fn disconnect(&self) {
        *self.connected.write().await = false;
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
