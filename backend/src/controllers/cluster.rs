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
use crate::utils::secrets;
use crate::AppError;

const DEFAULT_BACKUP_RETENTION: i32 = 10;

pub struct ClusterController {
    pool: SqlitePool,
    sqlite_path: String,
    jwt_secret: String,
}

impl ClusterController {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            sqlite_path: "/var/lib/tcs/data.db".to_string(),
            jwt_secret: String::new(),
        }
    }

    pub fn with_context(pool: SqlitePool, sqlite_path: String, jwt_secret: String) -> Self {
        Self {
            pool,
            sqlite_path,
            jwt_secret,
        }
    }

    fn enc(&self, plain: &str) -> Result<String, AppError> {
        secrets::encrypt(&self.jwt_secret, plain)
    }

    fn dec(&self, stored: &str) -> Result<String, AppError> {
        secrets::decrypt(&self.jwt_secret, stored)
    }

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

        let talosconfig = if let Some(ref yaml) = talosconfig_yaml {
            let trimmed = yaml.trim();
            if !trimmed.is_empty() {
                let _ = TalosCredentials::from_talosconfig_yaml(trimmed)?;
                Some(self.enc(trimmed)?)
            } else {
                None
            }
        } else {
            None
        };

        let kubeconfig = Some(self.enc(kubeconfig_yaml.trim())?);

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
            kubeconfig,
            backup_retention: Some(DEFAULT_BACKUP_RETENTION),
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

        self.upsert_discovered_machines(cluster.id, &discovered, now)
            .await?;

        crate::db::repos::cluster::update_status(&self.pool, cluster.id, "running").await?;

        if cluster.has_talos_credentials() {
            if let Err(e) = self.refresh_talos_versions(cluster.id).await {
                tracing::warn!(
                    cluster_id = %cluster.id,
                    error = %e,
                    "Could not probe Talos versions after import"
                );
            }
        }

        crate::db::repos::cluster::get(&self.pool, cluster.id)
            .await?
            .ok_or_else(|| AppError::Internal("Cluster disappeared after import".to_string()))
    }

    async fn upsert_discovered_machines(
        &self,
        cluster_id: Uuid,
        discovered: &DiscoveredCluster,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<i32, AppError> {
        let mut count = 0;
        let existing = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;

        for node in discovered
            .control_plane_nodes
            .iter()
            .chain(discovered.worker_nodes.iter())
        {
            let mtype = if node.role.contains("control") {
                "control-plane"
            } else {
                "worker"
            };
            let system_uuid = format!("k8s-{}-{}", cluster_id, node.name);

            if let Some(mut m) = existing.iter().find(|e| e.system_uuid == system_uuid).cloned() {
                m.address = node.internal_ip.clone();
                m.talos_version = node.talos_version.clone();
                m.machine_type = mtype.to_string();
                m.status = "running".to_string();
                m.updated_at = now;
                crate::db::repos::machine::update(&self.pool, &m).await?;
            } else {
                let machine = Machine {
                    id: Uuid::new_v4(),
                    system_uuid,
                    machine_type: mtype.to_string(),
                    cluster_id: Some(cluster_id),
                    status: "running".to_string(),
                    talos_version: node.talos_version.clone(),
                    secure_boot: false,
                    siderolink_connected: false,
                    address: node.internal_ip.clone(),
                    created_at: now,
                    updated_at: now,
                };
                crate::db::repos::machine::create(&self.pool, &machine).await?;
            }
            count += 1;
        }
        Ok(count)
    }

    pub async fn preview_import(&self, kubeconfig_yaml: String) -> Result<DiscoveredCluster, AppError> {
        discover_cluster_from_kubeconfig(&kubeconfig_yaml).await
    }

    pub async fn set_talosconfig(
        &self,
        cluster_id: Uuid,
        talosconfig_yaml: String,
    ) -> Result<(), AppError> {
        let _ = TalosCredentials::from_talosconfig_yaml(&talosconfig_yaml)?;
        let enc = self.enc(talosconfig_yaml.trim())?;
        crate::db::repos::cluster::set_talosconfig(&self.pool, cluster_id, &enc).await?;
        Ok(())
    }

    pub async fn refresh_from_kubeconfig(&self, cluster_id: Uuid) -> Result<i32, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let kc = cluster.kubeconfig.as_ref().ok_or_else(|| {
            AppError::InvalidInput(
                "No kubeconfig stored for this cluster. Re-import or attach kubeconfig."
                    .to_string(),
            )
        })?;
        let yaml = self.dec(kc)?;
        let discovered = discover_cluster_from_kubeconfig(&yaml).await?;
        let now = chrono::Utc::now();
        let n = self
            .upsert_discovered_machines(cluster_id, &discovered, now)
            .await?;
        crate::db::repos::cluster::update_status(&self.pool, cluster_id, "running").await?;
        // update versions on cluster row
        if let Some(mut c) = crate::db::repos::cluster::get(&self.pool, cluster_id).await? {
            c.control_plane_version = discovered.kubernetes_version;
            c.talos_version = discovered.talos_version;
            c.control_plane_size = discovered.control_plane_nodes.len() as i32;
            c.worker_size = discovered.worker_nodes.len() as i32;
            c.updated_at = now;
            let _ = crate::db::repos::cluster::update(&self.pool, &c).await;
        }
        Ok(n)
    }

    pub async fn test_talos_connectivity(
        &self,
        cluster_id: Uuid,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let creds = self.load_creds(&cluster)?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut results = Vec::new();
        for m in machines {
            let client = match self.client_for_machine(&cluster, &m).await {
                Ok(c) => c,
                Err(e) => {
                    results.push(serde_json::json!({
                        "machineId": m.id,
                        "address": m.address,
                        "ok": false,
                        "error": e.to_string(),
                    }));
                    continue;
                }
            };
            match client.get_version().await {
                Ok(v) => results.push(serde_json::json!({
                    "machineId": m.id,
                    "address": m.address,
                    "ok": true,
                    "talosVersion": v,
                    "endpoint": client.endpoint(),
                })),
                Err(e) => results.push(serde_json::json!({
                    "machineId": m.id,
                    "address": m.address,
                    "ok": false,
                    "error": e.to_string(),
                    "endpoint": client.endpoint(),
                })),
            }
        }
        if results.is_empty() {
            // try endpoints from talosconfig alone
            for ep in &creds.endpoints {
                let client = TalosClient::from_credentials(ep, &creds);
                match client.get_version().await {
                    Ok(v) => results.push(serde_json::json!({
                        "address": ep,
                        "ok": true,
                        "talosVersion": v,
                    })),
                    Err(e) => results.push(serde_json::json!({
                        "address": ep,
                        "ok": false,
                        "error": e.to_string(),
                    })),
                }
            }
        }
        Ok(results)
    }

    fn load_creds(&self, cluster: &Cluster) -> Result<TalosCredentials, AppError> {
        let yaml = cluster.talosconfig.as_ref().ok_or_else(|| {
            AppError::InvalidInput(
                "Cluster has no talosconfig. Attach one via PUT /api/clusters/{id}/talosconfig."
                    .to_string(),
            )
        })?;
        let plain = self.dec(yaml)?;
        TalosCredentials::from_talosconfig_yaml(&plain)
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
                self.enforce_backup_retention(cluster_id, &cluster).await?;
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

    async fn enforce_backup_retention(
        &self,
        cluster_id: Uuid,
        cluster: &Cluster,
    ) -> Result<(), AppError> {
        let keep = cluster
            .backup_retention
            .unwrap_or(DEFAULT_BACKUP_RETENTION)
            .max(1) as usize;
        let backups = crate::db::repos::cluster_backup::list_by_cluster(&self.pool, cluster_id)
            .await?;
        let ready: Vec<_> = backups
            .into_iter()
            .filter(|b| b.status == "ready")
            .collect();
        if ready.len() <= keep {
            return Ok(());
        }
        for old in ready.into_iter().skip(keep) {
            if let Some(path) = &old.file_path {
                let _ = tokio::fs::remove_file(path).await;
            }
            let _ = crate::db::repos::cluster_backup::delete(&self.pool, old.id).await;
        }
        Ok(())
    }

    pub async fn apply_config_patches(
        &self,
        cluster_id: Uuid,
        dry_run: bool,
    ) -> Result<serde_json::Value, AppError> {
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

        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut applied = Vec::new();
        let mut errors = Vec::new();
        let mut documents = Vec::new();

        for machine in &machines {
            let machine_patches: Vec<(String, String, i32)> = patches
                .iter()
                .filter(|p| p.machine_id.is_none() || p.machine_id == Some(machine.id))
                .map(|p| (p.path.clone(), p.value.clone(), p.priority))
                .collect();
            if machine_patches.is_empty() {
                continue;
            }
            let doc = build_patch_documents(&machine_patches)?;
            documents.push(serde_json::json!({
                "machineId": machine.id,
                "document": doc,
            }));
            if dry_run {
                applied.push(format!("{} (dry-run)", machine.system_uuid));
                continue;
            }
            match self.client_for_machine(&cluster, machine).await {
                Ok(client) => match client.apply_config_with_options(&doc, dry_run).await {
                    Ok(()) => applied.push(format!("{} ({})", machine.system_uuid, machine.address)),
                    Err(e) => errors.push(format!("{}: {}", machine.system_uuid, e)),
                },
                Err(e) => errors.push(format!("{}: {}", machine.system_uuid, e)),
            }
        }

        if applied.is_empty() && !dry_run {
            return Err(AppError::Network(format!(
                "Failed to apply config patches: {}",
                errors.join("; ")
            )));
        }

        Ok(serde_json::json!({
            "dryRun": dry_run,
            "appliedTo": applied,
            "count": applied.len(),
            "errors": errors,
            "documents": documents,
        }))
    }

    pub async fn reboot_machine(&self, machine_id: Uuid) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.reboot().await
    }

    pub async fn upgrade_machine(&self, machine_id: Uuid, image: &str) -> Result<(), AppError> {
        if image.trim().is_empty() {
            return Err(AppError::InvalidInput("image is required".to_string()));
        }
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.upgrade(image.trim()).await
    }

    pub async fn machine_version(&self, machine_id: Uuid) -> Result<String, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        let version = client.get_version().await?;
        let mut m = machine;
        m.talos_version = version.clone();
        m.updated_at = chrono::Utc::now();
        let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        Ok(version)
    }

    pub async fn update_machine_address(
        &self,
        machine_id: Uuid,
        address: String,
    ) -> Result<Machine, AppError> {
        let mut machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        machine.address = address.trim().to_string();
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await
    }

    async fn cluster_and_machine(
        &self,
        machine_id: Uuid,
    ) -> Result<(Cluster, Machine), AppError> {
        let machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        let cluster_id = machine.cluster_id.ok_or_else(|| {
            AppError::InvalidInput("Machine is not assigned to a cluster".to_string())
        })?;
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        Ok((cluster, machine))
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
}
