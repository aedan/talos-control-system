use std::path::PathBuf;

use crate::db::pool::DbPool;
use uuid::Uuid;

use crate::db::models::cluster::Cluster;
use crate::db::models::cluster_backup::ClusterBackup;
use crate::db::models::machine::Machine;
use crate::integration::kubernetes::{discover_cluster_from_kubeconfig, DiscoveredCluster};
use crate::integration::talosctl::{
    backup_root_from_sqlite_path, build_patch_documents, merge_patches_into_machine_config,
    merge_yaml_docs_into_machine_config, pick_control_plane_address, TalosCredentials,
};
use crate::integration::talosctl::{
    cmp_k8s_versions, latest_k8s_patch_for_minor, parse_k8s_version, TalosctlClient,
};
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

    /// Pool-only constructor for callers that don't need talosconfig context
    /// (e.g. DB-side operations, or where the caller supplies its own).
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            sqlite_path: String::new(),
            jwt_secret: String::new(),
        }
    }

    fn enc(&self, plain: &str) -> Result<String, AppError> {
        secrets::encrypt(&self.jwt_secret, plain)
    }

    fn dec(&self, stored: &str) -> Result<String, AppError> {
        secrets::decrypt(&self.jwt_secret, stored)
    }

    /// Decrypt the cluster's talosconfig YAML.
    fn talosconfig_yaml(&self, cluster: &Cluster) -> Result<Option<String>, AppError> {
        match &cluster.talosconfig {
            Some(enc) => Ok(Some(self.dec(enc)?)),
            None => Ok(None),
        }
    }

/// Effective endpoint to reach `machine` with talosctl. Delegates to the
/// free function [`effective_endpoint`] (see its docs for the tunnel-vs-LAN
/// preference order).
    async fn effective_endpoint(&self, machine: &Machine) -> Result<String, AppError> {
        effective_endpoint(&self.pool, machine).await
    }

    /// Return the cluster's persistent Siderolink token, creating one if absent.
    pub async fn ensure_cluster_siderolink_token(&self, cluster_id: Uuid) -> Result<String, AppError> {
        if let Some(existing) =
            crate::db::repos::siderolink::get_cluster_token(&self.pool, cluster_id).await?
        {
            return Ok(existing);
        }
        let token = format!("slc_{}", Uuid::new_v4().simple());
        crate::db::repos::siderolink::set_cluster_token(&self.pool, cluster_id, &token)
            .await?;
        Ok(token)
    }

    /// Rotate the cluster's persistent Siderolink token. Returns the new token.
    pub async fn rotate_cluster_siderolink_token(&self, cluster_id: Uuid) -> Result<String, AppError> {
        let token = format!("slc_{}", Uuid::new_v4().simple());
        crate::db::repos::siderolink::set_cluster_token(&self.pool, cluster_id, &token)
            .await?;
        Ok(token)
    }

    /// Revoke the cluster's persistent Siderolink token (nodes must be
    /// re-provisioned with a new one).
    pub async fn revoke_cluster_siderolink_token(&self, cluster_id: Uuid) -> Result<(), AppError> {
        crate::db::repos::siderolink::revoke_cluster_token(&self.pool, cluster_id).await
    }

    /// Whether the cluster has an active Siderolink token (i.e. the tunnel is
    /// enabled) and the list of currently-connected peers.
    pub async fn siderolink_status(
        &self,
        cluster_id: Uuid,
    ) -> Result<(bool, Vec<serde_json::Value>), AppError> {
        let enabled = crate::db::repos::siderolink::get_cluster_token(&self.pool, cluster_id)
            .await?
            .is_some();
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        // SideroLink peers are keyed by the node's Talos **MUID**, while
        // `machines.system_uuid` is a TCS-invented `mac-<MAC>` alias — so we
        // correlate by the machine's captured `muid` (falling back to
        // `system_uuid` for any machine still identified the old way). Without
        // this the intersection is empty and the UI reports 0 nodes connected
        // even with every tunnel up.
        let machine_ids: std::collections::HashSet<String> = machines
            .iter()
            .flat_map(|m| {
                [m.muid.clone(), m.system_uuid.clone()].into_iter().filter(|s| !s.is_empty())
            })
            .collect();
        let peers = crate::db::repos::siderolink::list_peers(&self.pool)
            .await?
            .into_iter()
            .filter(|p| machine_ids.contains(&p.system_uuid))
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect();
        Ok((enabled, peers))
    }

    /// Enable Siderolink for the cluster: ensure a token exists and apply the
    /// standalone `SideroLinkConfig` doc to every running machine (live, no
    /// reboot). Returns the number of machines patched.
    pub async fn siderolink_enable(
        &self,
        cluster_id: Uuid,
        sl_doc: &str,
    ) -> Result<usize, AppError> {
        if sl_doc.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "no Siderolink doc to apply (token missing?)".into(),
            ));
        }
        let _token = self.ensure_cluster_siderolink_token(cluster_id).await?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut patched = 0usize;
        let mut last_err = String::new();
        for m in machines.iter().filter(|m| m.status == "running") {
            match self
                .apply_machine_config_ex(m.id, Some(sl_doc), false, false, true)
                .await
            {
                Ok(_) => patched += 1,
                Err(e) => last_err = e.to_string(),
            }
        }
        if patched == 0 && !last_err.is_empty() {
            return Err(AppError::Internal(format!(
                "no machines patched: {last_err}"
            )));
        }
        Ok(patched)
    }

    /// Disable Siderolink for the cluster: strip the `SideroLinkConfig` doc
    /// from every running machine's live config (no reboot), then revoke the
    /// cluster token. Returns the number of machines patched.
    pub async fn siderolink_disable(&self, cluster_id: Uuid) -> Result<usize, AppError> {
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut patched = 0usize;
        let mut last_err = String::new();
        for m in machines.iter().filter(|m| m.status == "running") {
            match self.strip_siderolink_from_machine(m.id).await {
                Ok(true) => patched += 1,
                Ok(false) => {} // doc not present; nothing to do
                Err(e) => last_err = e.to_string(),
            }
        }
        // Revoke the token so stale configs can no longer join.
        let _ = self.revoke_cluster_siderolink_token(cluster_id).await;
        if patched == 0 && !last_err.is_empty() {
            return Err(AppError::Internal(format!("no machines patched: {last_err}")));
        }
        Ok(patched)
    }

    /// Fetch a machine's live config, remove the standalone `SideroLinkConfig`
    /// doc, and apply the result (no reboot). Returns true if a doc was found
    /// and removed.
    async fn strip_siderolink_from_machine(&self, machine_id: Uuid) -> Result<bool, AppError> {
        let (cluster, m) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&m).await?;
        let live = TalosctlClient::get_machine_config(&endpoint, tc.as_deref()).await?;
        let stripped = remove_siderolink_doc(&live);
        let changed = stripped != live;
        if changed {
            TalosctlClient::apply_config(&endpoint, &stripped, false, false, tc.as_deref()).await?;
            let mut mm = m;
            mm.desired_config = Some(stripped);
            mm.updated_at = chrono::Utc::now();
            let _ = crate::db::repos::machine::update(&self.pool, &mm).await;
        }
        Ok(changed)
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
            network_config: None,
            factory_modules: None,
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

            // Match an existing machine by synthetic uuid, then by address,
            // then by hostname. Inventory-imported machines carry their real
            // hardware UUID (not the synthetic one), so address/hostname is
            // what actually links a discovered node to its stored row.
            let match_idx = existing
                .iter()
                .position(|e| e.system_uuid == system_uuid)
                .or_else(|| {
                    existing
                        .iter()
                        .position(|e| !node.internal_ip.is_empty() && e.address == node.internal_ip)
                })
                .or_else(|| {
                    existing
                        .iter()
                        .position(|e| !node.name.is_empty() && e.hostname == node.name)
                });

            if let Some(idx) = match_idx {
                let mut m = existing[idx].clone();
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

    pub async fn set_kubeconfig(
        &self,
        cluster_id: Uuid,
        kubeconfig_yaml: String,
    ) -> Result<(), AppError> {
        let _ = crate::integration::kubernetes::parse_kubeconfig(&kubeconfig_yaml)?;
        let enc = self.enc(kubeconfig_yaml.trim())?;
        crate::db::repos::cluster::set_kubeconfig(&self.pool, cluster_id, &enc).await?;
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
        let tc = self.talosconfig_yaml(&cluster)?;
        let creds = TalosCredentials::from_talosconfig_yaml(
            tc.as_deref()
                .ok_or_else(|| AppError::InvalidInput("No talosconfig".to_string()))?,
        )?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut results = Vec::new();
        for m in machines {
            let endpoint = self.effective_endpoint(&m).await.unwrap_or_else(|_| m.address.clone());
            if endpoint.is_empty() {
                continue;
            }
            match TalosctlClient::get_version(&endpoint, tc.as_deref()).await {
                Ok(v) => results.push(serde_json::json!({
                    "machineId": m.id,
                    "address": endpoint,
                    "ok": true,
                    "talosVersion": v,
                })),
                Err(e) => results.push(serde_json::json!({
                    "machineId": m.id,
                    "address": endpoint,
                    "ok": false,
                    "error": e.to_string(),
                })),
            }
        }
        if results.is_empty() {
            for ep in &creds.endpoints {
                match TalosctlClient::get_version(ep, tc.as_deref()).await {
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

    pub async fn create_etcd_backup(
        &self,
        cluster_id: Uuid,
        name: String,
    ) -> Result<ClusterBackup, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let tc = self.talosconfig_yaml(&cluster)?;
        let creds = TalosCredentials::from_talosconfig_yaml(
            tc.as_deref()
                .ok_or_else(|| AppError::InvalidInput("No talosconfig".to_string()))?,
        )?;
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

        let mut backup = ClusterBackup::pending(cluster_id, name);
        crate::db::repos::cluster_backup::create(&self.pool, &backup).await?;

        let root = backup_root_from_sqlite_path(&self.sqlite_path);
        let dest: PathBuf = root
            .join(cluster_id.to_string())
            .join(format!("{}.snapshot", backup.id));

        match TalosctlClient::etcd_snapshot(&address, dest.to_str().unwrap(), tc.as_deref()).await {
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
        let tc = self.talosconfig_yaml(&cluster)?;

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
                let tc = tc.clone();
                async move {
                    // Prefer the Siderolink tunnel IP when this machine is
                    // connected + fresh; else the LAN address.
                    let endpoint = self
                        .effective_endpoint(&machine)
                        .await
                        .unwrap_or_else(|_| machine.address.clone());
                    let document = serde_json::json!({
                        "machineId": machine.id,
                        "address": endpoint,
                        "patchPreview": patch_preview,
                    });
                    if endpoint.is_empty() {
                        return (
                            document,
                            Err(format!("{}: no address", machine.system_uuid)),
                        );
                    }

                    // Get live config, merge patch, apply
                    match TalosctlClient::get_machine_config(&endpoint, tc.as_deref()).await {
                        Ok(live) => {
                            let merged = match merge_yaml_docs_into_machine_config(&live, &patch_preview) {
                                Ok(m) => m,
                                Err(e) => {
                                    return (
                                        document,
                                        Err(format!("{}: merge failed: {}", machine.system_uuid, e)),
                                    );
                                }
                            };
                            match TalosctlClient::apply_config(
                                &endpoint, &merged, false, dry_run, tc.as_deref(),
                            ).await {
                                Ok(()) => {
                                    let tag = if dry_run { "dry-run" } else { "applied" };
                                    (
                                        document,
                                        Ok(format!(
                                            "{} {} ({})",
                                            machine.system_uuid, tag, endpoint
                                        )),
                                    )
                                }
                                Err(e) => (
                                    document,
                                    Err(format!("{}: {}", machine.system_uuid, e)),
                                ),
                            }
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
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::reboot(&endpoint, tc.as_deref()).await
    }

    pub async fn upgrade_machine(&self, machine_id: Uuid, image: &str) -> Result<(), AppError> {
        if image.trim().is_empty() {
            return Err(AppError::InvalidInput("image is required".to_string()));
        }
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::upgrade(&endpoint, image.trim(), tc.as_deref()).await
    }

    /// Cordon + drain a machine's Kubernetes node (by its hostname, which is
    /// the k8s node name) using the cluster's stored kubeconfig. Best-effort:
    /// if there's no kubeconfig or the node isn't in k8s, it's skipped — the
    /// Talos upgrade still proceeds (talosctl is called with --drain=false).
    ///
    /// Control-plane nodes are cordoned but NOT drained: a full drain of a CP
    /// node would evict etcd/apiserver/scheduler pods and take the cluster down.
    /// Workers are fully drained (force) so their workloads move before reboot.
    /// Returns the node name if it was drained/cordoned, else None.
    pub async fn drain_machine_for_upgrade(&self, machine_id: Uuid) -> Result<Option<String>, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let Some(enc) = &cluster.kubeconfig else {
            return Ok(None);
        };
        let plain = self.dec(enc)?;
        let Ok(k8s) = crate::integration::kubernetes::K8sClient::from_kubeconfig_yaml(&plain).await else {
            return Ok(None);
        };
        let node_name = machine.hostname.trim();
        if node_name.is_empty() {
            return Ok(None);
        }
        // Only act if the node actually exists in k8s.
        let api = kube::Api::<k8s_openapi::api::core::v1::Node>::all(k8s.raw().clone());
        if api.get(node_name).await.is_err() {
            tracing::debug!(node = %node_name, "node not in k8s; skipping drain");
            return Ok(None);
        }
        let is_cp = is_control_plane(&machine.machine_type);
        if is_cp {
            k8s.cordon(node_name).await?;
        } else {            let res = k8s.drain(node_name, true).await?;
            if !res.errors.is_empty() {
                // Uncordon so we don't leave the node unschedulable on failure.
                let _ = k8s.uncordon(node_name).await;
                return Err(AppError::Internal(format!(
                    "drain of {node_name} had errors: {}",
                    res.errors.join("; ")
                )));
            }
        }
        Ok(Some(node_name.to_string()))
    }

    /// Uncordon a machine's Kubernetes node after its upgrade completes.
    pub async fn uncordon_machine(&self, machine_id: Uuid) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let Some(enc) = &cluster.kubeconfig else {
            return Ok(());
        };
        let plain = self.dec(enc)?;
        let Ok(k8s) = crate::integration::kubernetes::K8sClient::from_kubeconfig_yaml(&plain).await else {
            return Ok(());
        };
        let node_name = machine.hostname.trim();
        if node_name.is_empty() {
            return Ok(());
        }
        // Uncordon is best-effort; a missing node is not an error.
        if k8s.uncordon(node_name).await.is_err() {
            tracing::warn!(node = %node_name, "uncordon failed (node may still be booting)");
        }
        Ok(())
    }

    /// Run `talosctl upgrade-k8s` against one control-plane endpoint. The
    /// command discovers the rest of the cluster itself, pre-pulls images, and
    /// patches every node's machineconfig — entirely in place, no reboots.
    pub async fn run_k8s_upgrade(
        &self,
        cluster_id: Uuid,
        cp_address: &str,
        from: &str,
        to: &str,
    ) -> Result<String, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let tc = self.talosconfig_yaml(&cluster)?;
        TalosctlClient::k8s_upgrade(cp_address, from, to, tc.as_deref()).await
    }

    /// The cluster's current Kubernetes version via the stored kubeconfig
    /// (API server `/version`), falling back to the stored inventory value.
    pub async fn cluster_k8s_version(&self, cluster_id: Uuid) -> Result<String, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        if let Some(enc) = &cluster.kubeconfig {
            let plain = self.dec(enc)?;
            if let Ok(client) = crate::integration::kubernetes::K8sClient::from_kubeconfig_yaml(&plain).await
            {
                if let Ok(v) = client.api_server_version().await {
                    return Ok(v);
                }
            }
        }
        Ok(cluster.control_plane_version)
    }

    pub async fn reset_machine(
        &self,
        machine_id: Uuid,
        graceful: bool,
        reboot: bool,
    ) -> Result<(), AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::reset(&endpoint, graceful, reboot, tc.as_deref()).await?;
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
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::bootstrap(&endpoint, tc.as_deref()).await?;
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
            let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
            let tc = self.talosconfig_yaml(&cluster)?;
            let endpoint = self.effective_endpoint(&machine).await?;
            TalosctlClient::list_disks_postinstall(&endpoint, tc.as_deref()).await
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

        let tc = self.talosconfig_yaml(&cluster)?;
        if let Some(endpoint) = endpoint_override.filter(|e| !e.is_empty()) {
            TalosctlClient::apply_config_maintenance(
                endpoint, &config_yaml, true, tc.as_deref(),
            ).await?;
        } else {
            TalosctlClient::apply_config(
                &machine.address, &config_yaml, true, false, tc.as_deref(),
            ).await?;
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
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::apply_config(&endpoint, config_yaml, false, false, tc.as_deref()).await
    }

    /// Fetch live machine config from the node (requires address + talosconfig).
    pub async fn get_live_machine_config(&self, machine_id: Uuid) -> Result<String, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::get_machine_config(&endpoint, tc.as_deref()).await
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
            let tc = self.talosconfig_yaml(&cluster)?;
            let endpoint = self.effective_endpoint(&m).await?;
            let live = TalosctlClient::get_machine_config(&endpoint, tc.as_deref()).await?;
            yaml = merge_yaml_docs_into_machine_config(&live, &yaml)?;
        }

        let (cluster, m) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&m).await?;
        TalosctlClient::apply_config(&endpoint, &yaml, reboot, dry_run, tc.as_deref()).await?;

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
            let patch = wrap_network_helper_yaml(net);
            base = merge_yaml_docs_into_machine_config(&base, &patch)?;
        }
        if let Some(mounts) = extra_mounts_yaml.map(str::trim).filter(|s| !s.is_empty()) {
            let patch = if mounts.contains("extraMounts:") || mounts.contains("machine:") {
                if mounts.contains("machine:") {
                    mounts.to_string()
                } else {
                    format!("machine:\n  kubelet:\n{}", indent_yaml(mounts, 4))
                }
            } else {
                format!("machine:\n  kubelet:\n    extraMounts:\n{}", indent_yaml(mounts, 6))
            };
            base = merge_yaml_docs_into_machine_config(&base, &patch)?;
        }
        if !patches.is_empty() {
            base = merge_patches_into_machine_config(&base, &patches)?;
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
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        let version = TalosctlClient::get_version(&endpoint, tc.as_deref()).await?;
        let mut m = machine;
        m.talos_version = version.clone();
        m.updated_at = chrono::Utc::now();
        let _ = crate::db::repos::machine::update(&self.pool, &m).await;
        Ok(version)
    }

    /// Like [`Self::machine_version`] but connects via an explicit endpoint.
    /// Tries maintenance mode first (installer), then falls back to talosconfig
    /// auth (installed Talos).
    pub async fn machine_version_with_endpoint(
        &self,
        machine_id: Uuid,
        endpoint: Option<&str>,
    ) -> Result<String, AppError> {
        if let Some(addr) = endpoint.filter(|a| !a.is_empty()) {
            // Try maintenance mode first (installer)
            match TalosctlClient::probe_maintenance(addr).await {
                Ok(v) => return Ok(v),
                Err(_) => {}
            }
            // Fall back to talosconfig auth (installed Talos)
            let cluster = self.cluster_and_machine(machine_id).await.map(|(c, _)| c)?;
            let tc = self.talosconfig_yaml(&cluster)?;
            TalosctlClient::probe_node(addr, tc.as_deref()).await
        } else {
            Err(AppError::InvalidInput("no endpoint provided".into()))
        }
    }

    pub async fn machine_services(
        &self,
        machine_id: Uuid,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::service_list(&endpoint, tc.as_deref()).await
    }

    pub async fn machine_hostname(&self, machine_id: Uuid) -> Result<String, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::hostname(&endpoint, tc.as_deref()).await
    }

    /// The node's installed/upgradable Talos versions (raw `talosctl get versions` JSON).
    pub async fn machine_versions(&self, machine_id: Uuid) -> Result<serde_json::Value, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::get_versions(&endpoint, tc.as_deref()).await
    }

    /// The node's installed Talos extensions (modules).
    pub async fn machine_extensions(
        &self,
        machine_id: Uuid,
    ) -> Result<Vec<crate::integration::talosctl::MachineExtension>, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::list_extensions(&endpoint, tc.as_deref()).await
    }

    /// Live Kubernetes upgrade options for a cluster: current version plus the
    /// targets this Talos build supports (probed via `upgrade-k8s --dry-run` on
    /// one control-plane node). Used to populate the UI dropdown.
    ///
    /// Candidate minors: the current minor (patch bump), the next minor, and the
    /// one after — any jump >1 minor is fine here because the upgrade job builds
    /// a sequential per-minor ladder. Only candidates the Talos build actually
    /// supports as a single hop are listed; the final hop of a ladder is always
    /// one minor ahead, so listing mi/mi+1/mi+2 covers targets up to +2 minors.
    pub async fn k8s_upgrade_targets(&self, cluster_id: Uuid) -> Result<serde_json::Value, AppError> {
        let cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        let machines = crate::db::repos::machine::list_by_cluster(&self.pool, cluster_id).await?;

        // Live k8s version first (kubeconfig), fall back to the stored inventory.
        let current = match self.cluster_k8s_version(cluster_id).await {
            Ok(v) if !v.trim().is_empty() => v,
            _ => cluster.control_plane_version.clone(),
        };

        // A missing/malformed talosconfig should yield "no k8s targets", not an
        // error — the Talos phase still works off the factory image.
        let tc = match self.talosconfig_yaml(&cluster) {
            Ok(t) => t,
            Err(e) => {
                return Ok(serde_json::json!({
                    "current": current,
                    "supported": [],
                    "note": format!("Kubernetes probe skipped: {e}"),
                }))
            }
        };
        let cp_addr = {
            let pairs: Vec<(String, Option<String>)> =
                machines.iter().map(|m| (m.machine_type.clone(), Some(m.address.clone()))).collect();
            let creds = tc
                .as_deref()
                .and_then(|t| TalosCredentials::from_talosconfig_yaml(t).ok())
                .unwrap_or(TalosCredentials {
                    endpoints: vec![],
                    nodes: vec![],
                });
            pick_control_plane_address(&pairs, &creds).ok()
        };

        let mut supported: Vec<String> = Vec::new();
        if let (Some(addr), Some((ma, mi, pa))) = (cp_addr.clone(), parse_k8s_version(&current)) {
            if ma == 1 {
                // Latest patch of each candidate minor line.
                for target_minor in [mi, mi + 1, mi + 2] {
                    let latest = match tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        latest_k8s_patch_for_minor(target_minor),
                    )
                    .await
                    {
                        Ok(Some(v)) => v,
                        _ => continue,
                    };
                    // No point offering the exact current version.
                    if target_minor == mi && parse_k8s_version(&latest).map(|p| p.2 <= pa).unwrap_or(true) {
                        continue;
                    }
                    // Only offer hops the Talos build supports as a single step.
                    // The final ladder hop is +1 minor to the exact target; a
                    // target 2 minors out needs mi+1 (always supported if any
                    // minor hop is) so we probe mi+1 once and mi+2 separately.
                    match TalosctlClient::k8s_upgrade_supported(&addr, &current, &latest, tc.as_deref()).await {
                        Ok(true) => supported.push(latest),
                        Ok(false) => {}
                        Err(e) => tracing::warn!(candidate = %latest, error = %e, "k8s upgrade probe failed"),
                    }
                }
            }
        }
        supported.sort();

        Ok(serde_json::json!({
            "current": current,
            "supported": supported,
        }))
    }

    /// Store the cluster's factory modules and resolve the resulting schematic id
    /// + installer image. `modules_json` is a JSON array of official extension
    /// names (e.g. `["siderolabs/bnx2-bnx2x"]`) or null/empty for the default.
    pub async fn set_cluster_factory_modules(
        &self,
        cluster_id: Uuid,
        modules: Option<Vec<String>>,
        factory: &crate::config::FactoryConfig,
    ) -> Result<serde_json::Value, AppError> {
        let list: Vec<String> = modules
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        let factory_client = crate::integration::image_factory::ImageFactoryClient::new(&factory.normalized_base());
        // Resolve the schematic id from the chosen module set (empty => default).
        let schematic = factory_client.create_schematic(&list).await?;
        let modules_json = if list.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&list).unwrap_or_default())
        };

        let mut cluster = crate::db::repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;
        cluster.factory_modules = modules_json;
        cluster.updated_at = chrono::Utc::now();
        crate::db::repos::cluster::update(&self.pool, &cluster).await?;

        Ok(serde_json::json!({
            "ok": true,
            "modules": list,
            "schematic": schematic,
            "installerImage": factory.installer_image(&schematic, &cluster.talos_version),
        }))
    }

    /// Effective modules for a machine, applying the delta override model:
    ///   * if `machine.factory_modules` is set (absolute override, legacy or
    ///     from the per-machine picker), it wins outright;
    ///   * otherwise effective = (cluster.factory_modules − machine.module_removes
    ///     + machine.module_adds), deduped, order-preserving (cluster order first,
    ///     then additions in stored order).
    pub async fn effective_modules(&self, machine_id: Uuid) -> Result<Vec<String>, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let parse = |raw: &Option<String>| -> Result<Vec<String>, AppError> {
            if let Some(s) = raw {
                let s = s.trim();
                if s.is_empty() {
                    return Ok(Vec::new());
                }
                return serde_json::from_str::<Vec<String>>(s)
                    .map_err(|e| AppError::InvalidInput(format!("Invalid module JSON: {e}")));
            }
            Ok(Vec::new())
        };
        if !parse(&machine.factory_modules)?.is_empty() {
            return Ok(parse(&machine.factory_modules)?);
        }
        let mut base = parse(&cluster.factory_modules)?;
        let removes = parse(&machine.module_removes)?;
        let adds = parse(&machine.module_adds)?;
        if !removes.is_empty() {
            base.retain(|m| !removes.iter().any(|r| r == m));
        }
        for a in adds {
            if !base.contains(&a) {
                base.push(a);
            }
        }
        Ok(base)
    }

    /// Set a machine's delta module overrides (`adds` / `removes` against the
    /// cluster default set). Passing `None` for a field clears that delta.
    /// Passing both `None` with `reset=true` clears the absolute override too.
    pub async fn set_machine_module_overrides(
        &self,
        machine_id: Uuid,
        adds: Option<Vec<String>>,
        removes: Option<Vec<String>>,
        reset: bool,
    ) -> Result<serde_json::Value, AppError> {
        let (cluster, mut machine) = self.cluster_and_machine(machine_id).await?;
        let clean = |v: Option<Vec<String>>| -> Vec<String> {
            v.unwrap_or_default()
                .into_iter()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect()
        };
        let adds = clean(adds);
        let removes = clean(removes);
        machine.module_adds = if adds.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&adds).unwrap_or_default())
        };
        machine.module_removes = if removes.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&removes).unwrap_or_default())
        };
        if reset {
            machine.factory_modules = None;
        }
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await?;
        Ok(serde_json::json!({
            "ok": true,
            "adds": adds,
            "removes": removes,
            "clusterModules": parse_cluster_modules(&cluster),
            "effective": self.effective_modules(machine_id).await?,
        }))
    }
}

fn is_control_plane(machine_type: &str) -> bool {
    let t = machine_type.to_ascii_lowercase();
    t == "control-plane" || t == "controlplane"
}

fn parse_cluster_modules(cluster: &Cluster) -> Vec<String> {
    cluster
        .factory_modules
        .clone()
        .filter(|s| !s.trim().is_empty())
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

impl ClusterController {
    /// Set a machine's factory modules override (None clears it → inherits cluster).
    pub async fn set_machine_factory_modules(
        &self,
        machine_id: Uuid,
        modules: Option<Vec<String>>,
    ) -> Result<serde_json::Value, AppError> {
        let mut machine = crate::db::repos::machine::get(&self.pool, machine_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Machine {} not found", machine_id)))?;
        let list: Vec<String> = modules
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        machine.factory_modules = if list.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&list).unwrap_or_default())
        };
        machine.updated_at = chrono::Utc::now();
        crate::db::repos::machine::update(&self.pool, &machine).await?;
        Ok(serde_json::json!({ "ok": true, "modules": list }))
    }

    /// Apply the machine's effective modules by upgrading it to the factory
    /// installer image that bundles them. This reboots the node; on return the
    /// driver modules (e.g. bnx2x) are present and its NICs come up with the
    /// addresses already in its machineconfig.
    pub async fn apply_machine_modules(
        &self,
        machine_id: Uuid,
        factory: &crate::config::FactoryConfig,
    ) -> Result<serde_json::Value, AppError> {
        let (cluster, machine) = self.cluster_and_machine(machine_id).await?;
        let tc = self.talosconfig_yaml(&cluster)?;
        let modules = self.effective_modules(machine_id).await?;
        if modules.is_empty() {
            return Err(AppError::InvalidInput(
                "No modules selected for this machine or its cluster".to_string(),
            ));
        }
        let factory_client =
            crate::integration::image_factory::ImageFactoryClient::new(&factory.normalized_base());
        let schematic = factory_client.create_schematic(&modules).await?;
        let image = factory.installer_image(&schematic, &cluster.talos_version);

        // talosctl upgrade --image <factory installer> --preserve (reboots the node).
        let endpoint = self.effective_endpoint(&machine).await?;
        TalosctlClient::upgrade(&endpoint, &image, tc.as_deref()).await?;

        Ok(serde_json::json!({
            "ok": true,
            "machine": machine.id,
            "modules": modules,
            "schematic": schematic,
            "image": image,
            "rebooting": true,
        }))
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

        let tc = self.talosconfig_yaml(&cluster)?;
        let creds = TalosCredentials::from_talosconfig_yaml(
            tc.as_deref()
                .ok_or_else(|| AppError::InvalidInput("No talosconfig".to_string()))?,
        )?;
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

        let uploaded = TalosctlClient::etcd_recover(&address, canon_path.to_str().unwrap(), tc.as_deref()).await?;

        let mut bootstrap_ok = false;
        let mut bootstrap_error: Option<String> = None;
        if run_bootstrap {
            match TalosctlClient::bootstrap_recover_etcd(&address, skip_hash_check, tc.as_deref()).await {
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
                "Snapshot uploaded. Call restore again with runBootstrap=true, or bootstrap manually."
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

/// Normalize a Network YAML helper into a patch that merges under
/// `machine.network`. Accepted forms:
///   - full `machine:`-rooted YAML (used as-is)
///   - `network:`-rooted YAML (nested under `machine:` — otherwise the merge
///     would create a duplicate top-level `network:` key)
///   - bare fragment such as `interfaces:` (nested under `machine.network:`)
/// Multi-document fragments may append standalone network config docs
/// (e.g. `kind: VLANConfig`); those pass through unchanged after the `---`.
fn wrap_network_helper_yaml(net: &str) -> String {
    let docs: Vec<&str> = net
        .trim()
        .split("\n---\n")
        .map(|d| d.trim().trim_start_matches("---").trim())
        .filter(|d| !d.is_empty())
        .collect();

    let mut out = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        if i == 0 {
            let first = doc.trim();
            if first.contains("machine:") {
                out.push(first.to_string());
            } else if first.starts_with("network:") {
                out.push(format!("machine:\n{}", indent_yaml(first, 2)));
            } else {
                out.push(format!("machine:\n  network:\n{}", indent_yaml(first, 4)));
            }
        } else {
            out.push(doc.to_string());
        }
    }
    out.join("\n---\n")
}

/// Rewrite `machine.install.disk` in Talos machine config YAML.
/// Remove any standalone `SideroLinkConfig` document from a (possibly
/// multi-doc) machine-config string. Returns the config unchanged when no such
/// doc is present.
pub fn remove_siderolink_doc(config_yaml: &str) -> String {
    config_yaml
        .split("\n---\n")
        .filter(|doc| !doc.lines().any(|l| l.trim() == "kind: SideroLinkConfig"))
        .collect::<Vec<_>>()
        .join("\n---\n")
        .trim_end_matches('\n')
        .to_string()
}

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
    config_yaml.to_string()
}

/// Effective endpoint to reach `machine` with talosctl.
///
/// Preference order (per operator decision "prefer tunnel when connected"):
/// 1. The machine's **Siderolink tunnel IP** (from `siderolink_peers`, keyed by
///    `system_uuid`) — used when the machine is flagged `siderolink_connected`
///    AND its peer `last_seen` is fresh (within 5 minutes). This is the only
///    path that reaches nodes behind NAT/firewalls.
/// 2. The machine's **LAN `address`** from inventory (direct routing).
///
/// Free function so non-controller callers (e.g. the status reconciler) can
/// reuse it. Returns the chosen IP; an empty string means neither path is
/// available and will surface as a talosctl connection error.
pub async fn effective_endpoint(
    pool: &crate::db::pool::DbPool,
    machine: &Machine,
) -> Result<String, AppError> {
    if machine.siderolink_connected {
        // SideroLink peers are keyed by the node's Talos MUID; try the machine's
        // captured `muid` first, then the legacy `system_uuid` alias.
        let peer = if !machine.muid.is_empty() {
            match crate::db::repos::siderolink::find_by_uuid(pool, &machine.muid).await? {
                Some(p) => Some(p),
                None => crate::db::repos::siderolink::find_by_uuid(pool, &machine.system_uuid).await?,
            }
        } else {
            crate::db::repos::siderolink::find_by_uuid(pool, &machine.system_uuid).await?
        };
        if let Some(peer) = peer {
            let fresh = peer.last_seen > chrono::Utc::now() - chrono::Duration::minutes(5);
            if fresh && !peer.assigned_ip.is_empty() {
                return Ok(peer.assigned_ip.clone());
            }
        }
    }
    Ok(machine.address.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_network_helper_bare_fragment_nests_under_machine_network() {
        let net = "interfaces:\n  - interface: eth0\n    dhcp: true";
        let wrapped = wrap_network_helper_yaml(net);
        assert!(wrapped.starts_with("machine:\n  network:\n"));
        assert!(wrapped.contains("interface: eth0"));
    }

    #[test]
    fn wrap_network_helper_passes_through_standalone_docs() {
        let net = "interfaces:\n  - interface: bond0\n    bond:\n      mode: 802.3ad\n      interfaces:\n        - eno49\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\n";
        let wrapped = wrap_network_helper_yaml(net);
        assert!(wrapped.starts_with("machine:\n  network:\n    interfaces:"));
        let parts: Vec<&str> = wrapped.split("\n---\n").collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[1].contains("kind: VLANConfig"));
        assert!(parts[1].contains("vlanID: 207"));
    }

    #[test]
    fn wrap_network_helper_network_prefix_nests_under_machine() {
        let net = "network:\n  interfaces:\n    - interface: bond0";
        let wrapped = wrap_network_helper_yaml(net);
        assert!(wrapped.starts_with("machine:\n  network:\n"), "got: {wrapped}");
        assert!(!wrapped.starts_with("network:"), "got: {wrapped}");
    }

    #[test]
    fn wrap_network_helper_full_machine_used_as_is() {
        let net = "machine:\n  type: worker\n  network:\n    interfaces: []";
        assert_eq!(wrap_network_helper_yaml(net), net);
    }

    #[test]
    fn wrap_network_helper_merge_produces_single_network_key() {
        let net = "network:\n  interfaces:\n    - interface: bond0";
        let wrapped = wrap_network_helper_yaml(net);
        let base = "version: v1alpha1\nmachine:\n  type: controlplane\n  network:\n    interfaces:\n      - interface: eno1\ncluster:\n  clusterName: demo\n";
        let merged = merge_yaml_docs_into_machine_config(base, &wrapped).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&merged).unwrap();
        let map = doc.as_mapping().unwrap();
        assert!(
            !map.contains_key(serde_yaml::Value::String("network".into())),
            "duplicate top-level network key:\n{merged}"
        );
        assert!(merged.contains("bond0"));
        assert!(merged.contains("clusterName: demo") || merged.contains("clusterName:demo"));
    }

    #[tokio::test]
    async fn effective_endpoint_prefers_fresh_siderolink_tunnel() {
        use crate::config::{DatabaseBackend, DatabaseConfig};
        use crate::db::pool::{connect, DbPool};
        use crate::db::repos::siderolink::{self, SiderolinkPeer};
        use chrono::{Duration, Utc};

        // Fresh temp SQLite DB + migrations.
        let path = std::env::temp_dir().join(format!("tcs-eff-ep-{}.db", Uuid::new_v4().simple()));
        let _ = std::fs::remove_file(&path);
        let cfg = DatabaseConfig {
            backend: DatabaseBackend::Sqlite,
            sqlite_path: path.to_string_lossy().into(),
            postgres_url: String::new(),
            max_connections: 1,
            connection_timeout: 5,
        };
        let pool: DbPool = connect(&cfg).await.expect("connect");
        crate::db::run_migrations(&pool).await.expect("migrate");

        let mut m = Machine::new("uuid-abc-123".into(), "worker".into());
        m.address = "192.168.1.50".into();
        m.siderolink_connected = true;

        // (3) No peer registered yet → falls back to LAN address.
        assert_eq!(effective_endpoint(&pool, &m).await.unwrap(), "192.168.1.50");

        // (1) Peer registered with a fresh last_seen → tunnel IP wins.
        let peer = SiderolinkPeer {
            id: Uuid::new_v4(),
            system_uuid: "uuid-abc-123".into(),
            public_key: "pubkey".into(),
            assigned_ip: "100.64.0.7".into(),
            last_seen: Utc::now(),
            created_at: Utc::now(),
        };
        siderolink::upsert_peer(&pool, &peer).await.unwrap();
        assert_eq!(effective_endpoint(&pool, &m).await.unwrap(), "100.64.0.7");

        // (2) Stale peer (last seen 10 min ago > 5 min window) → back to LAN.
        let stale = SiderolinkPeer {
            id: peer.id,
            system_uuid: peer.system_uuid.clone(),
            public_key: peer.public_key.clone(),
            assigned_ip: peer.assigned_ip.clone(),
            last_seen: Utc::now() - Duration::minutes(10),
            created_at: peer.created_at,
        };
        siderolink::upsert_peer(&pool, &stale).await.unwrap();
        assert_eq!(effective_endpoint(&pool, &m).await.unwrap(), "192.168.1.50");

        // Not connected at all → LAN even with a fresh peer present.
        m.siderolink_connected = false;
        siderolink::upsert_peer(&pool, &peer).await.unwrap();
        assert_eq!(effective_endpoint(&pool, &m).await.unwrap(), "192.168.1.50");

        let _ = std::fs::remove_file(&path);
    }
}
