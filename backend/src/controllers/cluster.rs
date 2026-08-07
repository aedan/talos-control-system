use sqlx::SqlitePool;
use crate::db::models::cluster::Cluster;
use crate::db::models::machine::Machine;
use crate::AppError;
use crate::integration::kubernetes::{discover_cluster_from_kubeconfig, DiscoveredCluster};
use uuid::Uuid;

pub struct ClusterController {
    pool: SqlitePool,
}

impl ClusterController {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Import an existing Talos cluster by parsing kubeconfig and discovering nodes
    pub async fn import_cluster(&self, name: String, kubeconfig_yaml: String) -> Result<Cluster, AppError> {
        // Discover the cluster from kubeconfig
        let discovered = discover_cluster_from_kubeconfig(&kubeconfig_yaml).await?;

        // Validate it's a Talos cluster
        if !discovered.is_talos {
            return Err(AppError::InvalidInput(
                "Cluster does not appear to be running Talos Linux. Detected OS: {}. \
                 Only Talos Linux clusters can be imported."
                    .to_string()
            ));
        }

        // Check for duplicate cluster name
        let existing = crate::db::repos::cluster::list(&self.pool).await?;
        if existing.iter().any(|c| c.name == name) {
            return Err(AppError::InvalidInput(
                format!("A cluster with name '{}' already exists", name)
            ));
        }

        // Create the cluster record
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
            created_at: now,
            updated_at: now,
        };

        let cluster = crate::db::repos::cluster::create(&self.pool, &cluster).await?;
        tracing::info!(
            cluster_id = %cluster.id,
            name = %cluster.name,
            cp_nodes = discovered.control_plane_nodes.len(),
            worker_nodes = discovered.worker_nodes.len(),
            talos_version = %cluster.talos_version,
            k8s_version = %cluster.control_plane_version,
            "Cluster imported successfully"
        );

        // Create machine records for each discovered node
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

        // Update cluster status to running
        crate::db::repos::cluster::update_status(&self.pool, cluster.id, "running").await?;

        Ok(cluster)
    }

    /// Preview cluster discovery without saving
    pub async fn preview_import(&self, kubeconfig_yaml: String) -> Result<DiscoveredCluster, AppError> {
        discover_cluster_from_kubeconfig(&kubeconfig_yaml).await
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

    async fn cascade_cleanup(&self, _cluster: &Cluster) -> Result<(), AppError> {
        tracing::info!(cluster_id = %_cluster.id, "Performing cascade cleanup");
        Ok(())
    }
}
