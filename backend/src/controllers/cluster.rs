use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::models::cluster::Cluster;
use crate::db::models::cluster_backup::ClusterBackup;
use crate::db::models::machine::Machine;
use crate::integration::kubernetes::{discover_cluster_from_kubeconfig, DiscoveredCluster};
use crate::integration::talos::{
    backup_root_from_sqlite_path, build_patch_documents, pick_control_plane_address, TalosClient,
    TalosCredentials,
};
use crate::AppError;

pub struct ClusterController {
    pool: SqlitePool,
    sqlite_path: String,
}

impl ClusterController {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            sqlite_path: "/var/lib/tcs/data.db".to_string(),
        }
    }

    pub fn with_sqlite_path(pool: SqlitePool, sqlite_path: String) -> Self {
        Self { pool, sqlite_path }
    }

    /// Import an existing Talos cluster by parsing kubeconfig and discovering nodes.
    /// Optional `talosconfig_yaml` enables real Talos API actions (backups, apply, reboot).
    pub async fn import_cluster(
        &self,
        name: String,
        kubeconfig_yaml: String,
        talosconfig_yaml: Option<String>,
    ) -> Result<Cluster, AppError> {
        let discovered = discover_cluster_from_kubeconfig(&kubeconfig_yaml).await?;

        if !discovered.is_talos {
            return Err(AppError::InvalidInput(
                "Cluster does not appear to be running Talos Linux. \
                 Only Talos Linux clusters can be imported."
                    .to_string(),
            ));
        }

        let existing = crate::db::repos::cluster::list(&self.pool).await?;
        if existing.iter().any(|c| c.name == name) {
            return Err(AppError::InvalidInput(format!(
                "A cluster with name '{}' already exists",
                name
            )));
        }

        // Validate talosconfig early if provided
        let talosconfig = if let Some(ref yaml) = talosconfig_yaml {
            let trimmed = yaml.trim();
            if !trimmed.is_empty() {
                let _ = TalosCredentials::from_talosconfig_yaml(trimmed)?;
                Some(trimmed.to_string())
            } else {
                None
            }
        } else {
            None
        };

        let cluster_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let cluster = Cluster {
            id: cluster_id,
            name: name.clone(),
            control_plane_version: discovered.kubernetes_version.clone(),
            talos_version: discovered.talos_version.clone(),
            status: "importing".to_string(),
            control_plane_size: discovered.control_plane_nodes.len() as i32,
            worker_size: discovered.worker_nodes.len() as i32,
            talosconfig,
            created_at: now,
            updated_at: now,
        };

        let cluster = crate::db::repos::cluster::create(&self.pool, &cluster).await?;
        tracing::info!(
            cluster_id = %cluster.id,
            name = %cluster.name,
            cp_nodes = discovered.control_plane_nodes.len(),
            worker_nodes = discovered.worker_nodes.len(),
            has_talosconfig = cluster.has_talos_credentials(),
            "Cluster imported successfully"
        );

        let mut imported_machines = 0;
        for node in &discovered.control_plane_nodes {
            let machine = Machine {
                id: Uuid::new_v4(),
                system_uuid: format!("k8s-{}-{}", cluster_id, node.name),
                machine_type: "control-plane".to_string(),
                cluster_id: Some(cluster.id),
                status: "running".to_string(),
                talos_version: node.talos_version.clone(),
                secure_boot: false,
                siderolink_connected: false,
                address: node.internal_ip.clone(),
                created_at: now,
                updated_at: now,
            };
            if crate::db::repos::machine::create(&self.pool, &machine).await.is_ok() {
                imported_machines += 1;
            }
        }

        for node in &discovered.worker_nodes {
            let machine = Machine {
                id: Uuid::new_v4(),
                system_uuid: format!("k8s-{}-{}", cluster_id, node.name),
                machine_type: "worker".to_string(),
                cluster_id: Some(cluster.id),
                status: "running".to_string(),
                talos_version: node.talos_version.clone(),
                secure_boot: false,
                siderolink_connected: false,
                address: node.internal_ip.clone(),
                created_at: now,
                updated_at: now,
            };
            if crate::db::repos::machine::create(&self.pool, &machine).await.is_ok() {
                imported_machines += 1;
            }
        }

        tracing::info!(
            cluster_id = %cluster.id,
            imported_machines,
            "Imported machines for cluster"
        );

        crate::db::repos::cluster::update_status(&self.pool, cluster.id, "running").await?;

        // Optionally probe Talos version when credentials were supplied
        if cluster.has_talos_credentials() {
            if let Err(e) = self.refresh_talos_versions(cluster.id).await {
                tracing::warn!(
                    cluster_id = %cluster.id,
                    error = %e,
                    "Could not probe Talos versions after import (credentials may lack node reachability)"
                );
            }
        }

        crate::db::repos::cluster::get(&self.pool, cluster.id)
            .await?
            .ok_or_else(|| AppError::Internal("Cluster disappeared after import".to_string()))
    }

    /// Preview cluster discovery without saving
    pub async fn preview_import(&self, kubeconfig_yaml: String) -> Result<DiscoveredCluster, AppError> {
        discover_cluster_from_kubeconfig(&kubeconfig_yaml).await
    }

    pub async fn set_talosconfig(
        &self,
        cluster_id: Uuid,
        talosconfig_yaml: String,
    ) -> Result<(), AppError> {
        let _ = TalosCredentials::from_talosconfig_yaml(&talosconfig_yaml)?;
        crate::db::repos::cluster::set_talosconfig(&self.pool, cluster_id, &talosconfig_yaml)
            .await?;
        Ok(())
    }

    fn load_creds(&self, cluster: &Cluster) -> Result<TalosCredentials, AppError> {
        let yaml = cluster.talosconfig.as_ref().ok_or_else(|| {
            AppError::InvalidInput(
                "Cluster has no talosconfig. Attach one via PUT /api/clusters/{id}/talosconfig \
                 or re-import with talosconfig."
                    .to_string(),
            )
        })?;
        TalosCredentials::from_talosconfig_yaml(yaml)
    }

    async fn client_for_machine(
        &self,
        cluster: &Cluster,
        machine: &Machine,
    ) -> Result<TalosClient, AppError> {
        let creds = self.load_creds(cluster)?;
        let addr = if machine.address.is_empty() {
            None
        } else {
            Some(machine.address.as_str())
        };
        TalosClient::for_machine(addr, &creds)
    }

    /// Take a real etcd snapshot from a control-plane node and store it on disk.
    pub async fn create_etcd_backup(
        &self,
        cluster_id: Uuid,
        name: String,
    ) -> Result<ClusterBackup, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let creds = self.load_creds(&cluster)?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let pairs: Vec<(String, Option<String>)> = machines
            .iter()
            .map(|m| {
                (
                    m.machine_type.clone(),
                    if m.address.is_empty() {
                        None
                    } else {
                        Some(m.address.clone())
                    },
                )
            })
            .collect();
        let address = pick_control_plane_address(&pairs, &creds)?;
        let client = TalosClient::from_credentials(&address, &creds);

        let mut backup = ClusterBackup::pending(cluster_id, name);
        crate::db::repos::cluster_backup::create(&self.pool, &backup).await?;

        let root = backup_root_from_sqlite_path(&self.sqlite_path);
        let dest: PathBuf = root
            .join(cluster_id.to_string())
            .join(format!("{}.snapshot", backup.id));

        match client.etcd_snapshot(&dest).await {
            Ok(size) => {
                backup.status = "ready".to_string();
                backup.file_path = Some(dest.to_string_lossy().to_string());
                backup.size_bytes = size as i64;
                backup.updated_at = chrono::Utc::now();
                crate::db::repos::cluster_backup::update(&self.pool, &backup).await?;
                Ok(backup)
            }
            Err(e) => {
                backup.status = "failed".to_string();
                backup.updated_at = chrono::Utc::now();
                let _ = crate::db::repos::cluster_backup::update(&self.pool, &backup).await;
                Err(e)
            }
        }
    }

    /// Apply stored config patches to cluster machines via Talos ApplyConfiguration.
    pub async fn apply_config_patches(
        &self,
        cluster_id: Uuid,
    ) -> Result<Vec<String>, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let patches =
            crate::db::repos::config_patch::list_by_cluster(&self.pool, cluster_id).await?;
        if patches.is_empty() {
            return Err(AppError::InvalidInput(
                "No config patches to apply".to_string(),
            ));
        }

        let patch_tuples: Vec<(String, String, i32)> = patches
            .iter()
            .map(|p| (p.path.clone(), p.value.clone(), p.priority))
            .collect();
        let document = build_patch_documents(&patch_tuples)?;

        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut applied = Vec::new();
        let mut errors = Vec::new();

        for machine in &machines {
            // Machine-scoped patches: only apply to that machine when set
            let machine_patches: Vec<(String, String, i32)> = patches
                .iter()
                .filter(|p| p.machine_id.is_none() || p.machine_id == Some(machine.id))
                .map(|p| (p.path.clone(), p.value.clone(), p.priority))
                .collect();
            if machine_patches.is_empty() {
                continue;
            }
            let doc = build_patch_documents(&machine_patches)?;
            match self.client_for_machine(&cluster, machine).await {
                Ok(client) => match client.apply_config(&doc).await {
                    Ok(()) => applied.push(format!("{} ({})", machine.system_uuid, machine.address)),
                    Err(e) => errors.push(format!("{}: {}", machine.system_uuid, e)),
                },
                Err(e) => errors.push(format!("{}: {}", machine.system_uuid, e)),
            }
        }

        // silence unused if all machine-scoped filtered differently
        let _ = document;

        if applied.is_empty() {
            return Err(AppError::Network(format!(
                "Failed to apply config patches: {}",
                errors.join("; ")
            )));
        }

        if !errors.is_empty() {
            tracing::warn!(
                cluster_id = %cluster_id,
                errors = ?errors,
                "Some machines failed config apply"
            );
        }

        Ok(applied)
    }

    pub async fn reboot_machine(&self, machine_id: Uuid) -> Result<(), AppError> {
        let machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        let cluster_id = machine.cluster_id.ok_or_else(|| {
            AppError::InvalidInput("Machine is not assigned to a cluster".to_string())
        })?;
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.reboot().await
    }

    pub async fn machine_version(&self, machine_id: Uuid) -> Result<String, AppError> {
        let machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        let cluster_id = machine.cluster_id.ok_or_else(|| {
            AppError::InvalidInput("Machine is not assigned to a cluster".to_string())
        })?;
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        let version = client.get_version().await?;
        // Persist discovered version
        let mut m = machine;
        m.talos_version = version.clone();
        m.updated_at = chrono::Utc::now();
        let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        Ok(version)
    }

    async fn refresh_talos_versions(&self, cluster_id: Uuid) -> Result<(), AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        for m in machines {
            if let Err(e) = self.machine_version(m.id).await {
                tracing::debug!(machine = %m.system_uuid, error = %e, "version probe failed");
            }
        }
        Ok(())
    }

    pub async fn reconcile(&self) -> Result<(), AppError> {
        let clusters = crate::db::repos::cluster::list(&self.pool).await?;

        for cluster in clusters {
            if self.reconcile_cluster(&cluster).await.is_err() {
                tracing::warn!(cluster_id = %cluster.id, "Failed to reconcile cluster");
            }
        }

        Ok(())
    }

    async fn reconcile_cluster(&self, cluster: &Cluster) -> Result<(), AppError> {
        match cluster.status.as_str() {
            "running" => {
                self.aggregate_cluster_status(cluster).await?;
            },
            "scaling_up" => {
                self.handle_scaling_up(cluster).await?;
            },
            "scaling_down" => {
                self.handle_scaling_down(cluster).await?;
            },
            "destroying" => {
                self.handle_destroy(cluster).await?;
            },
            _ => {
                self.aggregate_cluster_status(cluster).await?;
            },
        }

        Ok(())
    }

    async fn aggregate_cluster_status(&self, cluster: &Cluster) -> Result<(), AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster.id).await?;

        if machines.is_empty() {
            if cluster.status != "unknown" {
                crate::db::repos::cluster::update_status(&self.pool, cluster.id, "unknown").await?;
            }
            return Ok(());
        }

        let all_running = machines.iter().all(|m| m.status == "running");
        let any_destroying = machines.iter().any(|m| m.status == "destroying");

        let new_status = if any_destroying {
            "destroying"
        } else if all_running {
            "running"
        } else {
            "unknown"
        };

        if new_status != cluster.status {
            crate::db::repos::cluster::update_status(&self.pool, cluster.id, new_status).await?;
            tracing::info!(cluster_id = %cluster.id, status = new_status, "Cluster status updated");
        }

        Ok(())
    }

    async fn handle_scaling_up(&self, cluster: &Cluster) -> Result<(), AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster.id).await?;
        let desired_size = cluster.control_plane_size + cluster.worker_size;

        if machines.len() >= desired_size as usize {
            crate::db::repos::cluster::update_status(&self.pool, cluster.id, "running").await?;
            tracing::info!(cluster_id = %cluster.id, "Cluster scaling up complete");
        }

        Ok(())
    }

    async fn handle_scaling_down(&self, cluster: &Cluster) -> Result<(), AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster.id).await?;
        let desired_size = cluster.control_plane_size + cluster.worker_size;

        if machines.len() <= desired_size as usize {
            crate::db::repos::cluster::update_status(&self.pool, cluster.id, "running").await?;
            tracing::info!(cluster_id = %cluster.id, "Cluster scaling down complete");
        }

        Ok(())
    }

    async fn handle_destroy(&self, cluster: &Cluster) -> Result<(), AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster.id).await?;

        if machines.iter().all(|m| m.status == "destroying") {
            self.cascade_cleanup(cluster).await?;
        }

        Ok(())
    }

    async fn cascade_cleanup(&self, cluster: &Cluster) -> Result<(), AppError> {
        tracing::info!(cluster_id = %cluster.id, "Performing cascade cleanup");
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster.id).await?;
        for machine in machines {
            tracing::info!(machine_id = %machine.id, "Deleting machine in cascade cleanup");
            let _ = crate::db::repos::machine::delete(&self.pool, machine.id).await;
        }
        let _ = crate::db::repos::cluster::delete(&self.pool, cluster.id).await;
        tracing::info!(cluster_id = %cluster.id, "Cascade cleanup complete");
        Ok(())
    }
}
