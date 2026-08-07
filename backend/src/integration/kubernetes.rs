use std::sync::Arc;
use std::time::Duration;
use moka::future::Cache;
use tracing::info;

use crate::AppError;

pub struct KubernetesClient {
    cluster_id: uuid::Uuid,
    endpoint: String,
}

impl KubernetesClient {
    pub async fn new(cluster_id: uuid::Uuid, endpoint: String, _ca_data: Vec<u8>, _token: String) -> Result<Self, AppError> {
        info!(cluster_id = %cluster_id, endpoint = %endpoint, "Kubernetes client initialized");
        Ok(Self { cluster_id, endpoint })
    }

    pub async fn apply_manifest(&self, manifest: &str) -> Result<(), AppError> {
        tracing::debug!(manifest_len = manifest.len(), "Applying manifest");
        Ok(())
    }

    pub fn cluster_id(&self) -> uuid::Uuid {
        self.cluster_id
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

pub struct KubernetesClientPool {
    cache: Cache<uuid::Uuid, Arc<KubernetesClient>>,
}

impl KubernetesClientPool {
    pub fn new() -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(10240)
                .time_to_live(Duration::from_secs(3600))
                .time_to_idle(Duration::from_secs(1800))
                .build(),
        }
    }

    pub async fn get_or_create(
        &self,
        cluster_id: uuid::Uuid,
        endpoint: String,
        ca_data: Vec<u8>,
        token: String,
    ) -> Result<Arc<KubernetesClient>, AppError> {
        if let Some(client_entry) = self.cache.get(&cluster_id).await {
            return Ok(client_entry);
        }

        let client = Arc::new(
            KubernetesClient::new(cluster_id, endpoint.clone(), ca_data, token).await?
        );

        self.cache.insert(cluster_id, Arc::clone(&client));
        Ok(client)
    }

    pub fn invalidate(&self, cluster_id: &uuid::Uuid) {
        self.cache.invalidate(cluster_id);
    }
}
