use tracing::info;
use std::hash::Hasher;
use std::hash::Hash;

use crate::db::models::cluster::Cluster;
use crate::db::models::machine::Machine;
use crate::AppError;
use crate::runtime::cache::AppCache;

pub struct ConfigController {
    cache: AppCache,
}

impl ConfigController {
    pub fn new(cache: AppCache) -> Self {
        Self { cache }
    }

    pub async fn generate_config(&self, cluster: &Cluster, machine: &Machine) -> Result<String, AppError> {
        let cache_key = format!("{}-{}", cluster.id, machine.id);

        if let Some(cached) = self.cache.get_config(&cache_key).await {
            info!(machine_id = %machine.id, "Config cache hit");
            return Ok(String::from_utf8(cached).map_err(|e| AppError::Internal(e.to_string()))?);
        }

        let config = self.build_talos_config(cluster, machine)?;

        self.cache.set_config(cache_key, config.as_bytes().to_vec());
        info!(machine_id = %machine.id, "Generated Talos config");

        Ok(config)
    }

    fn build_talos_config(&self, cluster: &Cluster, machine: &Machine) -> Result<String, AppError> {
        let is_control_plane = machine.machine_type == "controlplane";
        let role = if is_control_plane { "controlplane" } else { &machine.machine_type };

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        cluster.id.to_string().hash(&mut hasher);
        machine.id.to_string().hash(&mut hasher);
        let ip_suffix = (hasher.finish() % 254) + 1;

        let endpoint = format!("https://100.64.0.{}:6443", ip_suffix);

        let mut config = format!(
            "version: v1alpha1\n\
             machine:\n\
               type: {}\n\
               install:\n\
                 disk: /dev/sda\n\
                 image: factory.talos.dev/installer/{}\n\
               network:\n\
                 hostname: {}-{}\n",
            role,
            cluster.talos_version,
            cluster.name,
            &machine.system_uuid[..machine.system_uuid.find('-').unwrap_or(machine.system_uuid.len())]
        );

        if is_control_plane {
            config.push_str("               tokens:\n                 - secret-token\n");
        }

        config.push_str(&format!(
            "cluster:\n\
               name: {}\n\
               id: {}\n\
               controlPlane:\n\
                 endpoint: {}\n\
               apiServer:\n\
                 certSANs:\n\
                   - 100.64.0.{}\n\
               network:\n\
                 cni:\n\
                   name: flannel\n\
                 dns:\n\
                   clusterDomain: cluster.local\n\
               token: secret-token\n\
               secret: cluster-secret\n",
            cluster.name,
            cluster.id,
            endpoint,
            ip_suffix
        ));

        Ok(config)
    }
}
