//! Periodic etcd snapshot scheduler for clusters with backup_schedule_hours set.

use std::time::Duration;

use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::controllers::cluster::ClusterController;
use crate::utils::audit;

/// Spawn a background loop that creates etcd backups for scheduled clusters.
pub fn spawn_backup_scheduler(
    pool: SqlitePool,
    sqlite_path: String,
    jwt_secret: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Check every 15 minutes
        let interval = Duration::from_secs(15 * 60);
        info!("Etcd backup scheduler started (interval {:?})", interval);
        loop {
            if let Err(e) = run_once(&pool, &sqlite_path, &jwt_secret).await {
                warn!(error = %e, "Backup scheduler tick failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

async fn run_once(
    pool: &SqlitePool,
    sqlite_path: &str,
    jwt_secret: &str,
) -> Result<(), crate::AppError> {
    let clusters = crate::db::repos::cluster::list_with_backup_schedule(pool).await?;
    if clusters.is_empty() {
        return Ok(());
    }

    let controller = ClusterController::with_context(
        pool.clone(),
        sqlite_path.to_string(),
        jwt_secret.to_string(),
    );
    let now = chrono::Utc::now();

    for cluster in clusters {
        if !cluster.has_talos_credentials() {
            continue;
        }
        let hours = cluster.backup_schedule_hours.unwrap_or(0);
        if hours <= 0 {
            continue;
        }
        let due = match cluster.last_auto_backup_at {
            None => true,
            Some(last) => {
                let elapsed = now.signed_duration_since(last);
                elapsed.num_hours() >= hours as i64
            }
        };
        if !due {
            continue;
        }

        let name = format!("auto-{}", now.format("%Y%m%d-%H%M%S"));
        info!(cluster = %cluster.name, hours, "Running scheduled etcd backup");
        match controller.create_etcd_backup(cluster.id, name).await {
            Ok(b) => {
                let _ = crate::db::repos::cluster::mark_auto_backup(pool, cluster.id).await;
                audit::log_action(
                    pool,
                    "scheduler",
                    "auto_backup",
                    &cluster.id.to_string(),
                    &format!("backup={} status={}", b.id, b.status),
                )
                .await;
            }
            Err(e) => {
                warn!(cluster = %cluster.name, error = %e, "Scheduled backup failed");
                audit::log_action(
                    pool,
                    "scheduler",
                    "auto_backup_failed",
                    &cluster.id.to_string(),
                    &e.to_string(),
                )
                .await;
            }
        }
    }

    Ok(())
}
