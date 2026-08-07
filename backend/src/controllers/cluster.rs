use sqlx::SqlitePool;
use crate::db::models::cluster::Cluster;
use crate::AppError;

pub struct ClusterController {
    pool: SqlitePool,
}

impl ClusterController {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
