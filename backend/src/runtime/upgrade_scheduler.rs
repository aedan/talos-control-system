use std::time::Duration;

use sqlx::SqlitePool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::controllers::cluster::ClusterController;
use crate::db::repos::upgrade_job::{self, UpgradeJobTarget};
use crate::db::repos::{self};

pub fn spawn_upgrade_scheduler(
    pool: SqlitePool,
    sqlite_path: String,
    jwt_secret: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Upgrade job scheduler started");
        loop {
            if let Err(e) = tick(&pool, &sqlite_path, &jwt_secret).await {
                warn!(error = %e, "Upgrade scheduler tick failed");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

async fn tick(
    pool: &SqlitePool,
    sqlite_path: &str,
    jwt_secret: &str,
) -> Result<(), crate::AppError> {
    let jobs = upgrade_job::list_pending_jobs(pool).await?;
    for job in jobs {
        if job.status == "pending" {
            upgrade_job::update_job_status(pool, job.id, "running", None).await?;
        }
        run_job(pool, sqlite_path, jwt_secret, job.id).await?;
    }
    Ok(())
}

async fn run_job(
    pool: &SqlitePool,
    sqlite_path: &str,
    jwt_secret: &str,
    job_id: Uuid,
) -> Result<(), crate::AppError> {
    let Some(job) = upgrade_job::get_job(pool, job_id).await? else {
        return Ok(());
    };
    if job.cancel_requested {
        upgrade_job::update_job_status(pool, job_id, "cancelled", Some("cancel requested"))
            .await?;
        return Ok(());
    }

    let targets = upgrade_job::list_targets(pool, job_id).await?;
    if !targets
        .iter()
        .any(|t| t.status == "pending" || t.status == "running")
    {
        let any_fail = targets.iter().any(|t| t.status == "failed");
        upgrade_job::update_job_status(
            pool,
            job_id,
            if any_fail { "failed" } else { "completed" },
            None,
        )
        .await?;
        return Ok(());
    }

    let max = job.max_unavailable.max(1) as usize;
    let running = targets.iter().filter(|t| t.status == "running").count();
    let slots = max.saturating_sub(running);

    let controller =
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());

    if slots > 0 {
        let to_start: Vec<_> = targets
            .iter()
            .filter(|t| t.status == "pending")
            .take(slots)
            .cloned()
            .collect();
        for t in to_start {
            if upgrade_job::get_job(pool, job_id)
                .await?
                .map(|j| j.cancel_requested)
                .unwrap_or(false)
            {
                upgrade_job::update_job_status(pool, job_id, "cancelled", Some("cancel requested"))
                    .await?;
                return Ok(());
            }
            upgrade_job::update_target_status(pool, t.id, "running", None).await?;
            match controller.upgrade_machine(t.machine_id, &job.image).await {
                Ok(()) => info!(machine = %t.machine_id, "Upgrade initiated"),
                Err(e) => {
                    upgrade_job::update_target_status(pool, t.id, "failed", Some(&e.to_string()))
                        .await?;
                }
            }
        }
    }

    let targets = upgrade_job::list_targets(pool, job_id).await?;
    for t in targets.iter().filter(|t| t.status == "running") {
        poll_target(pool, sqlite_path, jwt_secret, &job.image, t).await?;
    }
    Ok(())
}

async fn poll_target(
    pool: &SqlitePool,
    sqlite_path: &str,
    jwt_secret: &str,
    image: &str,
    t: &UpgradeJobTarget,
) -> Result<(), crate::AppError> {
    let controller =
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
    match controller.machine_version(t.machine_id).await {
        Ok(ver) => {
            let want = image.rsplit(':').next().unwrap_or(image);
            if ver.contains(want) || image.contains(&ver) {
                upgrade_job::update_target_status(pool, t.id, "completed", None).await?;
                if let Ok(Some(mut m)) = repos::machine::get(pool, t.machine_id).await {
                    m.talos_version = ver;
                    m.updated_at = chrono::Utc::now();
                    let _ = repos::machine::update(pool, &m).await;
                }
            }
        }
        Err(e) => {
            warn!(machine = %t.machine_id, error = %e, "version poll during upgrade");
        }
    }
    Ok(())
}
