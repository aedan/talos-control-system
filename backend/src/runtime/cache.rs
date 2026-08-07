use std::sync::Arc;
use std::time::Duration;
use moka::future::Cache;
use uuid::Uuid;
use tracing::info;

use crate::db::models::cluster::Cluster;
use crate::db::models::machine::Machine;

pub struct AppCache {
    pub clusters: Cache<Uuid, Cluster>,
    pub machines: Cache<Uuid, Machine>,
    pub configs: Cache<String, Vec<u8>>,
    pub branding: Cache<String, serde_json::Value>,
}

impl AppCache {
    pub fn new() -> Self {
        Self {
            clusters: Cache::builder()
                .max_capacity(1024)
                .time_to_live(Duration::from_secs(300))
                .time_to_idle(Duration::from_secs(60))
                .build(),
            machines: Cache::builder()
                .max_capacity(4096)
                .time_to_live(Duration::from_secs(300))
                .time_to_idle(Duration::from_secs(60))
                .build(),
            configs: Cache::builder()
                .max_capacity(2048)
                .time_to_live(Duration::from_secs(3600))
                .time_to_idle(Duration::from_secs(300))
                .build(),
            branding: Cache::builder()
                .max_capacity(128)
                .time_to_live(Duration::from_secs(600))
                .time_to_idle(Duration::from_secs(120))
                .build(),
        }
    }

    pub async fn get_cluster(&self, id: &Uuid) -> Option<Cluster> {
        self.clusters.get(id).await
    }

    pub fn set_cluster(&self, cluster: Cluster) {
        self.clusters.insert(cluster.id, cluster);
    }

    pub fn remove_cluster(&self, id: Uuid) {
        self.clusters.invalidate(&id);
    }

    pub async fn get_machine(&self, id: &Uuid) -> Option<Machine> {
        self.machines.get(id).await
    }

    pub fn set_machine(&self, machine: Machine) {
        self.machines.insert(machine.id, machine);
    }

    pub fn remove_machine(&self, id: Uuid) {
        self.machines.invalidate(&id);
    }

    pub async fn get_config(&self, key: &str) -> Option<Vec<u8>> {
        self.configs.get(key).await
    }

    pub fn set_config(&self, key: String, data: Vec<u8>) {
        self.configs.insert(key, data);
    }

    pub async fn get_branding(&self, tenant_id: &str) -> Option<serde_json::Value> {
        self.branding.get(tenant_id).await
    }

    pub fn set_branding(&self, tenant_id: String, branding: serde_json::Value) {
        self.branding.insert(tenant_id, branding);
    }

    pub fn invalidate_branding(&self, tenant_id: &str) {
        self.branding.invalidate(tenant_id);
    }

    pub fn clear(&self) {
        self.clusters.invalidate_all();
        self.machines.invalidate_all();
        self.configs.invalidate_all();
        self.branding.invalidate_all();
        info!("All caches invalidated");
    }

    pub fn stats(&self) -> serde_json::Value {
        serde_json::json!({
            "clusters": self.clusters.entry_count(),
            "machines": self.machines.entry_count(),
            "configs": self.configs.entry_count(),
            "branding": self.branding.entry_count(),
        })
    }
}

impl Clone for AppCache {
    fn clone(&self) -> Self {
        Self {
            clusters: self.clusters.clone(),
            machines: self.machines.clone(),
            configs: self.configs.clone(),
            branding: self.branding.clone(),
        }
    }
}