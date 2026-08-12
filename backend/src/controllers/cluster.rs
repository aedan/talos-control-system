use std::path::PathBuf;

use crate::db::pool::DbPool;
use uuid::Uuid;

use crate::db::models::cluster::Cluster;
use crate::db::models::cluster_backup::ClusterBackup;
use crate::db::models::machine::Machine;
use crate::integration::kubernetes::{discover_cluster_from_kubeconfig, DiscoveredCluster};
use crate::integration::talos::{
    backup_root_from_sqlite_path, build_patch_documents, pick_control_plane_address, TalosClient,
    TalosCredentials,
};
use crate::integration::talosctl::TalosctlClient;
use crate::utils::secrets;
use crate::AppError;

const DEFAULT_BACKUP_RETENTION: i32 = 10;

pub struct ClusterController {
    pool: DbPool,
    sqlite_path: String,
    jwt_secret: String,
}

impl ClusterController {
    pub fn with_context(pool: DbPool, sqlite_path: String, jwt_secret: String) -> Self {
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
            backup_schedule_hours: None,
            last_auto_backup_at: None,
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
                let mut machine = Machine::new(system_uuid, mtype.to_string());
                machine.cluster_id = Some(cluster_id);
                machine.status = "running".to_string();
                machine.talos_version = node.talos_version.clone();
                machine.address = node.internal_ip.clone();
                machine.created_at = now;
                machine.updated_at = now;
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
        self.client_for_machine_at(cluster, machine, None).await
    }

    /// Like [`Self::client_for_machine`] but with an explicit endpoint
    /// override (used during PXE/installer phase when the machine only has
    /// its DHCP lease address, not its post-install static one).
    async fn client_for_machine_at(
        &self,
        cluster: &Cluster,
        machine: &Machine,
        endpoint_override: Option<&str>,
    ) -> Result<TalosClient, AppError> {
        if let Some(addr) = endpoint_override.filter(|a| !a.is_empty()) {
            return Err(AppError::InvalidInput(format!(
                "endpoint_override ({}) requires talosctl; use list_disks/install_machine directly",
                addr
            )));
        }
        tracing::info!("using standard mTLS client for installed node");
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
        let creds = self.load_creds(&cluster)?;

        // Build per-machine work items first (CPU-bound YAML only).
        let mut work = Vec::new();
        for machine in machines {
            let machine_patches: Vec<(String, String, i32)> = patches
                .iter()
                .filter(|p| p.machine_id.is_none() || p.machine_id == Some(machine.id))
                .map(|p| (p.path.clone(), p.value.clone(), p.priority))
                .collect();
            if machine_patches.is_empty() {
                continue;
            }
            let patch_preview = build_patch_documents(&machine_patches)?;
            work.push((machine, patch_preview));
        }

        if work.is_empty() {
            return Err(AppError::InvalidInput(
                "No config patches apply to any machine".to_string(),
            ));
        }

        let futs: Vec<_> = work
            .into_iter()
            .map(|(machine, patch_preview)| {
                let creds = creds.clone();
                async move {
                    let document = serde_json::json!({
                        "machineId": machine.id,
                        "address": machine.address,
                        "patchPreview": patch_preview,
                    });
                    let client = match TalosClient::for_machine(
                        if machine.address.is_empty() {
                            None
                        } else {
                            Some(machine.address.as_str())
                        },
                        &creds,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            return (
                                document,
                                Err(format!("{}: {}", machine.system_uuid, e)),
                            );
                        }
                    };
                    match client.apply_config_patch(&patch_preview, dry_run).await {
                        Ok(()) => {
                            let tag = if dry_run { "dry-run" } else { "applied" };
                            (
                                document,
                                Ok(format!(
                                    "{} {} ({})",
                                    machine.system_uuid, tag, machine.address
                                )),
                            )
                        }
                        Err(e) => (
                            document,
                            Err(format!("{}: {}", machine.system_uuid, e)),
                        ),
                    }
                }
            })
            .collect();

        let outcomes = futures_util::future::join_all(futs).await;
        let mut applied = Vec::new();
        let mut errors = Vec::new();
        let mut documents = Vec::new();

        for (doc, result) in outcomes {
            documents.push(doc);
            match result {
                Ok(line) => applied.push(line),
                Err(e) => errors.push(e),
            }
        }

        if applied.is_empty() {
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

    pub async fn reset_machine(
        &self,
        machine_id: Uuid,
        graceful: bool,
        reboot: bool,
    ) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.reset(graceful, reboot).await?;
        let mut m = machine;
        m.status = "resetting".to_string();
        m.updated_at = chrono::Utc::now();
        let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        Ok(())
    }

    pub async fn bootstrap_machine(&self, machine_id: Uuid) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        if !machine.machine_type.to_ascii_lowercase().contains("control") {
            return Err(AppError::InvalidInput(
                "Bootstrap is only for control-plane machines".into(),
            ));
        }
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.bootstrap().await?;
        let mut m = machine;
        m.status = "running".to_string();
        m.updated_at = chrono::Utc::now();
        let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        Ok(())
    }

    /// List disks available on a machine via the Talos Storage service.
    pub async fn list_disks(
        &self,
        machine_id: Uuid,
        endpoint_override: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        if let Some(endpoint) = endpoint_override.filter(|e| !e.is_empty()) {
            TalosctlClient::list_disks(endpoint).await
        } else {
            let (_cluster, _machine) = self.cluster_and_machine(machine_id).await?;
            let (_cluster, machine) = self.cluster_and_machine(machine_id).await?;
            let client = self.client_for_machine(&_cluster, &machine).await?;
            let disks = client.list_disks().await?;
            Ok(disks.into_iter().map(|d| serde_json::json!({
                "deviceName": d.device_name,
                "name": d.name,
                "serial": d.serial,
                "size": d.size,
                "type": d.r#type,
                "model": d.model,
                "systemDisk": d.system_disk,
            })).collect())
        }
    }

    /// Set the install disk for a machine (DB-only, no Talos API call).
    pub async fn set_install_disk(&self, machine_id: Uuid, disk: &str) -> Result<Machine, AppError> {
        if disk.trim().is_empty() {
            return Err(AppError::InvalidInput("disk must not be empty".to_string()));
        }
        let mut machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        machine.install_disk = disk.trim().to_string();
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await
    }

    /// Apply config with reboot to install Talos on a machine.
    pub async fn install_machine(
        &self,
        machine_id: Uuid,
        config_yaml: &str,
        endpoint_override: Option<&str>,
    ) -> Result<(), AppError> {
        let (cluster, mut machine) = self.cluster_and_machine(machine_id).await?;

        if machine.install_disk.is_empty() {
            return Err(AppError::InvalidInput("install_disk not set for this machine".into()));
        }

        let config_yaml = inject_install_disk(config_yaml, &machine.install_disk);

        machine.status = "installing".to_string();
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await?;

        if let Some(endpoint) = endpoint_override.filter(|e| !e.is_empty()) {
            TalosctlClient::apply_config(endpoint, &config_yaml, true).await?;
        } else {
            let client = self.client_for_machine(&cluster, &machine).await?;
            client
                .apply_config_with_options(&config_yaml, false, true)
                .await?;
        }

        machine.status = "booting".to_string();
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await?;

        Ok(())
    }

    /// Apply full machine config YAML (from provision artifact) to a node.
    pub async fn apply_machine_config(
        &self,
        machine_id: Uuid,
        config_yaml: &str,
    ) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.apply_config(config_yaml).await
    }

    /// Fetch live machine config from the node (requires address + talosconfig).
    pub async fn get_live_machine_config(&self, machine_id: Uuid) -> Result<String, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.get_machine_config().await
    }

    /// Desired (saved) config YAML for a machine, if any.
    pub async fn get_desired_machine_config(
        &self,
        machine_id: Uuid,
    ) -> Result<Option<String>, AppError> {
        let m = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {machine_id} not found")))?;
        Ok(m.desired_config.filter(|s| !s.trim().is_empty()))
    }

    /// Save desired machine config working copy (does not apply to node).
    pub async fn set_desired_machine_config(
        &self,
        machine_id: Uuid,
        config_yaml: &str,
    ) -> Result<(), AppError> {
        if config_yaml.trim().is_empty() {
            return Err(AppError::InvalidInput("configYaml required".into()));
        }
        if !config_yaml.contains("machine:") && !config_yaml.contains("cluster:") {
            return Err(AppError::InvalidInput(
                "config does not look like a Talos machine config".into(),
            ));
        }
        let mut m = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {machine_id} not found")))?;
        m.desired_config = Some(config_yaml.to_string());
        m.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &m).await?;
        Ok(())
    }

    /// Apply config to node. Prefer body yaml, else desired, else error.
    pub async fn apply_machine_config_ex(
        &self,
        machine_id: Uuid,
        config_yaml: Option<&str>,
        dry_run: bool,
        reboot: bool,
        merge_with_live: bool,
    ) -> Result<serde_json::Value, AppError> {
        let mut yaml = if let Some(y) = config_yaml.filter(|s| !s.trim().is_empty()) {
            y.to_string()
        } else {
            self.get_desired_machine_config(machine_id)
                .await?
                .ok_or_else(|| {
                    AppError::InvalidInput(
                        "No configYaml provided and no desired_config saved".into(),
                    )
                })?
        };

        // Optionally inject install disk from inventory
        let machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {machine_id} not found")))?;
        if !machine.install_disk.is_empty() {
            yaml = inject_install_disk(&yaml, &machine.install_disk);
        }

        if merge_with_live {
            let (cluster, m) = self.cluster_and_machine(machine_id).await?;
            let client = self.client_for_machine(&cluster, &m).await?;
            let live = client.get_machine_config().await?;
            yaml = crate::integration::talos::merge_yaml_docs_into_machine_config(&live, &yaml)?;
        }

        let (cluster, m) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &m).await?;
        client
            .apply_config_with_options(&yaml, dry_run, reboot)
            .await?;

        if !dry_run {
            // Keep desired in sync with what we applied
            let mut m = m;
            m.desired_config = Some(yaml.clone());
            m.updated_at = chrono::Utc::now();
            let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        }

        Ok(serde_json::json!({
            "ok": true,
            "dryRun": dry_run,
            "reboot": reboot,
            "bytes": yaml.len(),
        }))
    }

    /// Merge structured helpers into desired (or live base) and save.
    pub async fn apply_machine_config_helpers(
        &self,
        machine_id: Uuid,
        install_image: Option<&str>,
        network_yaml: Option<&str>,
        extra_mounts_yaml: Option<&str>,
        hostname: Option<&str>,
        base_from_live: bool,
    ) -> Result<String, AppError> {
        let mut base = if let Some(d) = self.get_desired_machine_config(machine_id).await? {
            d
        } else if base_from_live {
            self.get_live_machine_config(machine_id).await?
        } else {
            return Err(AppError::InvalidInput(
                "No desired config yet — load live config first or paste a full YAML".into(),
            ));
        };

        let mut patches: Vec<(String, String, i32)> = Vec::new();
        if let Some(img) = install_image.map(str::trim).filter(|s| !s.is_empty()) {
            // YAML string value for image
            patches.push((
                "/machine/install/image".into(),
                format!("\"{img}\""),
                10,
            ));
        }
        if let Some(h) = hostname.map(str::trim).filter(|s| !s.is_empty()) {
            patches.push((
                "/machine/network/hostname".into(),
                format!("\"{h}\""),
                10,
            ));
        }
        if let Some(net) = network_yaml.map(str::trim).filter(|s| !s.is_empty()) {
            // Merge network document under machine.network
            let patch = if net.contains("machine:") || net.trim_start().starts_with("network:") {
                net.to_string()
            } else {
                format!("machine:\n  network:\n{}", indent_yaml(net, 4))
            };
            base = crate::integration::talos::merge_yaml_docs_into_machine_config(&base, &patch)?;
        }
        if let Some(mounts) = extra_mounts_yaml.map(str::trim).filter(|s| !s.is_empty()) {
            // Expect list of extraMounts items or full kubelet.extraMounts
            let patch = if mounts.contains("extraMounts:") || mounts.contains("machine:") {
                if mounts.contains("machine:") {
                    mounts.to_string()
                } else {
                    format!("machine:\n  kubelet:\n{}", indent_yaml(mounts, 4))
                }
            } else {
                // assume raw sequence of mounts
                format!("machine:\n  kubelet:\n    extraMounts:\n{}", indent_yaml(mounts, 6))
            };
            base = crate::integration::talos::merge_yaml_docs_into_machine_config(&base, &patch)?;
        }
        if !patches.is_empty() {
            base = crate::integration::talos::merge_patches_into_machine_config(&base, &patches)?;
        }

        self.set_desired_machine_config(machine_id, &base).await?;
        Ok(base)
    }

    /// Scale worker inventory desired size and emit worker config for additional nodes.
    pub async fn scale_workers(
        &self,
        cluster_id: Uuid,
        desired_workers: i32,
    ) -> Result<Cluster, AppError> {
        if desired_workers < 0 {
            return Err(AppError::InvalidInput("desired_workers must be >= 0".into()));
        }
        let mut cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        cluster.worker_size = desired_workers;
        cluster.updated_at = chrono::Utc::now();
        crate::db::repos::cluster::update(&self.pool, &cluster).await
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

    pub async fn machine_services(
        &self,
        machine_id: Uuid,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.service_list().await
    }

    pub async fn machine_hostname(&self, machine_id: Uuid) -> Result<String, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let client = self.client_for_machine(&cluster, &machine).await?;
        client.hostname().await
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

    /// Disaster recovery: upload a stored etcd snapshot to a control-plane node,
    /// optionally run Bootstrap with recover_etcd.
    ///
    /// Requires `confirm == true`. Prefer a maintenance window; this can break a healthy cluster.
    pub async fn set_backup_schedule(
        &self,
        cluster_id: Uuid,
        schedule_hours: Option<i32>,
        retention: Option<i32>,
    ) -> Result<(), AppError> {
        if let Some(h) = schedule_hours {
            if h < 0 {
                return Err(AppError::InvalidInput(
                    "backup_schedule_hours must be >= 0 (0 or null disables)".to_string(),
                ));
            }
        }
        if let Some(r) = retention {
            if r < 1 {
                return Err(AppError::InvalidInput(
                    "backup_retention must be >= 1".to_string(),
                ));
            }
        }
        let hours = schedule_hours.and_then(|h| if h == 0 { None } else { Some(h) });
        crate::db::repos::cluster::set_backup_schedule(
            &self.pool,
            cluster_id,
            hours,
            retention,
        )
        .await
    }

    pub async fn restore_etcd_backup(
        &self,
        cluster_id: Uuid,
        backup_id: Uuid,
        confirm: bool,
        run_bootstrap: bool,
        skip_hash_check: bool,
        machine_id: Option<Uuid>,
    ) -> Result<serde_json::Value, AppError> {
        if !confirm {
            return Err(AppError::InvalidInput(
                "Restore requires confirm=true. This is destructive disaster recovery."
                    .to_string(),
            ));
        }

        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let backup = crate::db::repos::cluster_backup::get(&self.pool, backup_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Backup {} not found", backup_id)))?;

        if backup.cluster_id != cluster_id {
            return Err(AppError::NotFound("Backup not found for cluster".to_string()));
        }
        if backup.status != "ready" {
            return Err(AppError::InvalidInput(format!(
                "Backup is not ready (status: {})",
                backup.status
            )));
        }

        let path_str = backup.file_path.as_ref().ok_or_else(|| {
            AppError::InvalidInput("Backup has no file on disk".to_string())
        })?;
        let path = PathBuf::from(path_str);

        // Prevent path traversal: file must live under the backup root
        let root = backup_root_from_sqlite_path(&self.sqlite_path);
        let canon_root = root.canonicalize().unwrap_or(root.clone());
        let canon_path = path.canonicalize().map_err(|e| {
            AppError::NotFound(format!("Backup file missing: {}", e))
        })?;
        if !canon_path.starts_with(&canon_root) {
            return Err(AppError::InvalidInput(
                "Backup path is outside the configured backup directory".to_string(),
            ));
        }

        let creds = self.load_creds(&cluster)?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;

        let address = if let Some(mid) = machine_id {
            let m = machines
                .iter()
                .find(|m| m.id == mid)
                .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", mid)))?;
            if m.machine_type != "control-plane" && m.machine_type != "controlplane" {
                return Err(AppError::InvalidInput(
                    "Etcd restore target must be a control-plane machine".to_string(),
                ));
            }
            if m.address.is_empty() {
                return Err(AppError::InvalidInput(
                    "Target machine has no address".to_string(),
                ));
            }
            m.address.clone()
        } else {
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
            pick_control_plane_address(&pairs, &creds)?
        };

        let client = TalosClient::from_credentials(&address, &creds);
        let uploaded = client.etcd_recover(&canon_path).await?;

        let mut bootstrap_ok = false;
        let mut bootstrap_error: Option<String> = None;
        if run_bootstrap {
            match client.bootstrap_recover_etcd(skip_hash_check).await {
                Ok(()) => bootstrap_ok = true,
                Err(e) => bootstrap_error = Some(e.to_string()),
            }
        }

        Ok(serde_json::json!({
            "ok": bootstrap_error.is_none(),
            "backupId": backup_id,
            "target": address,
            "bytesUploaded": uploaded,
            "bootstrapRequested": run_bootstrap,
            "bootstrapOk": bootstrap_ok,
            "bootstrapError": bootstrap_error,
            "message": if run_bootstrap {
                if bootstrap_ok {
                    "Snapshot uploaded and Bootstrap(recover_etcd) requested"
                } else {
                    "Snapshot uploaded but Bootstrap failed — see bootstrapError"
                }
            } else {
                "Snapshot uploaded via EtcdRecover. Call restore again with runBootstrap=true, or bootstrap manually."
            },
        }))
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
        let _ = self.probe_cluster_talos_versions(cluster_id).await;
        Ok(())
    }

    /// Probe Talos version on every machine in parallel; update inventory +
    /// cluster `talos_version` summary. Returns ok/fail counts.
    pub async fn probe_cluster_talos_versions(
        &self,
        cluster_id: Uuid,
    ) -> Result<serde_json::Value, AppError> {
        let _cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;

        let futs: Vec<_> = machines
            .into_iter()
            .map(|m| {
                let pool = self.pool.clone();
                let sqlite_path = self.sqlite_path.clone();
                let jwt_secret = self.jwt_secret.clone();
                async move {
                    let ctrl = ClusterController::with_context(pool, sqlite_path, jwt_secret);
                    let machine_id = m.id;
                    let address = m.address.clone();
                    match ctrl.machine_version(machine_id).await {
                        Ok(v) => serde_json::json!({
                            "machineId": machine_id,
                            "address": address,
                            "ok": true,
                            "talosVersion": v,
                        }),
                        Err(e) => serde_json::json!({
                            "machineId": machine_id,
                            "address": address,
                            "ok": false,
                            "error": e.to_string(),
                        }),
                    }
                }
            })
            .collect();

        let results = futures_util::future::join_all(futs).await;
        let mut ok = 0u32;
        let mut failed = 0u32;
        let mut versions: Vec<String> = Vec::new();

        for r in &results {
            if r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                ok += 1;
                if let Some(v) = r.get("talosVersion").and_then(|v| v.as_str()) {
                    if !versions.iter().any(|x| x == v) {
                        versions.push(v.to_string());
                    }
                }
            } else {
                failed += 1;
            }
        }

        if !versions.is_empty() {
            if let Ok(Some(mut c)) = crate::db::repos::cluster::get(&self.pool, cluster_id).await {
                c.talos_version = versions.join(", ");
                c.updated_at = chrono::Utc::now();
                let _ = crate::db::repos::cluster::update(&self.pool, &c).await;
            }
        }

        Ok(serde_json::json!({
            "ok": ok,
            "failed": failed,
            "versions": versions,
            "results": results,
        }))
    }
}

fn indent_yaml(yaml: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    yaml.lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrite `machine.install.disk` in Talos machine config YAML.
pub fn inject_install_disk(config_yaml: &str, disk: &str) -> String {
    let disk = disk.trim();
    if disk.is_empty() {
        return config_yaml.to_string();
    }
    let re = regex::Regex::new(r"(?m)^([ \t]*disk:[ \t]*).+$").ok();
    if let Some(re) = re {
        if re.is_match(config_yaml) {
            return re
                .replace(config_yaml, format!("${{1}}{disk}"))
                .into_owned();
        }
    }
    // Fallback: no disk line found — leave YAML unchanged
    config_yaml.to_string()
}
