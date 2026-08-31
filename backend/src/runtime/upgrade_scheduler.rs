use std::time::Duration;

use crate::db::pool::DbPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::controllers::cluster::ClusterController;
use crate::db::repos::upgrade_job::{self, UpgradeJobTarget};
use crate::db::repos::{self};
use crate::integration::talosctl::cmp_k8s_versions;

pub fn spawn_upgrade_scheduler(
    pool: DbPool,
    sqlite_path: String,
    jwt_secret: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Upgrade job scheduler started");
        loop {
            match crate::runtime::ha::try_acquire(&pool, "upgrade_scheduler", 15).await {
                Ok(true) => {
                    if let Err(e) = tick(&pool, &sqlite_path, &jwt_secret).await {
                        warn!(error = %e, "Upgrade scheduler tick failed");
                    }
                }
                Ok(false) => {}
                Err(e) => warn!(error = %e, "HA lock acquire failed (upgrade)"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

async fn tick(
    pool: &DbPool,
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
    pool: &DbPool,
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

    match job.phase.as_str() {
        "talos" => run_talos_phase(pool, sqlite_path, jwt_secret, job).await,
        "k8s" => run_k8s_phase(pool, sqlite_path, jwt_secret, job).await,
        other => {
            warn!(job = %job_id, phase = other, "unknown upgrade job phase");
            Ok(())
        }
    }
}

// ── Phase 1: Talos image roll (reboots nodes) ──────────────────────────────

async fn run_talos_phase(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    job: upgrade_job::UpgradeJob,
) -> Result<(), crate::AppError> {
    let targets = upgrade_job::list_targets(pool, job.id).await?;
    let any_active = targets
        .iter()
        .any(|t| t.status == "pending" || t.status == "running");
    if !any_active {
        transition_after_talos(pool, &job, &targets).await?;
        return Ok(());
    }

    let max = job.max_unavailable.max(1) as usize;
    let running = targets.iter().filter(|t| t.status == "running").count();
    let slots = max.saturating_sub(running);

    if slots > 0 {
        let to_start: Vec<_> = targets
            .iter()
            .filter(|t| t.status == "pending")
            .take(slots)
            .cloned()
            .collect();
        for t in to_start {
            if cancel_requested(pool, job.id).await? {
                upgrade_job::update_job_status(pool, job.id, "cancelled", Some("cancel requested"))
                    .await?;
                return Ok(());
            }
            upgrade_job::update_target_status(pool, t.id, "running", None).await?;
            let image = t.image.clone().unwrap_or_else(|| job.image.clone());
            let controller =
                ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
            // Cordon + drain this node's k8s workload first (via the stored
            // kubeconfig), so talosctl upgrade --drain=false can't fail trying
            // to fetch a kubeconfig from a worker.
            match controller.drain_machine_for_upgrade(t.machine_id).await {
                Ok(Some(node)) => info!(machine = %t.machine_id, node, "drained node before upgrade"),
                Ok(None) => {}
                Err(e) => {
                    upgrade_job::update_target_status(pool, t.id, "failed", Some(&format!("drain failed: {e}"))).await?;
                    continue;
                }
            }
            match controller.upgrade_machine(t.machine_id, &image).await {
                Ok(()) => info!(machine = %t.machine_id, image, "Talos upgrade initiated"),
                Err(e) => {
                    let _ = controller.uncordon_machine(t.machine_id).await;
                    upgrade_job::update_target_status(pool, t.id, "failed", Some(&e.to_string()))
                        .await?;
                }
            }
        }
    }

    for t in targets.iter().filter(|t| t.status == "running") {
        poll_talos_target(pool, sqlite_path, jwt_secret, &job, t).await?;
    }
    Ok(())
}

async fn poll_talos_target(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    job: &upgrade_job::UpgradeJob,
    t: &UpgradeJobTarget,
) -> Result<(), crate::AppError> {
    let controller =
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
    let Some(want) = job.target_talos_version.as_ref() else {
        return Ok(());
    };
    match controller.machine_version(t.machine_id).await {
        Ok(ver) => {
            let want_n = norm_v(want);
            let got_n = norm_v(&ver);
            if got_n == want_n || got_n.contains(&want_n) || want_n.contains(&got_n) {
                upgrade_job::update_target_status(pool, t.id, "completed", None).await?;
                // Node is back — return it to the scheduler.
                let _ = controller.uncordon_machine(t.machine_id).await;
                if let Ok(Some(mut m)) = repos::machine::get(pool, t.machine_id).await {
                    m.talos_version = ver;
                    m.updated_at = chrono::Utc::now();
                    let _ = repos::machine::update(pool, &m).await;
                }
            }
        }
        Err(e) => {
            // Nodes are unreachable while rebooting — not a failure, just not done.
            warn!(machine = %t.machine_id, error = %e, "version poll during Talos upgrade");
        }
    }
    Ok(())
}

/// When the Talos phase has no active targets, either move on to the k8s phase
/// or finish the job.
async fn transition_after_talos(
    pool: &DbPool,
    job: &upgrade_job::UpgradeJob,
    targets: &[UpgradeJobTarget],
) -> Result<(), crate::AppError> {
    let any_fail = targets.iter().any(|t| t.status == "failed");
    if any_fail {
        upgrade_job::update_job_status(
            pool,
            job.id,
            "failed",
            Some("one or more machines failed the Talos phase"),
        )
        .await?;
        return Ok(());
    }
    let has_k8s = job
        .target_k8s_version
        .as_ref()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    if has_k8s {
        upgrade_job::update_job_phase(pool, job.id, "k8s").await?;
        for t in targets {
            upgrade_job::update_target_fields(pool, t.id, Some("pending"), None, None, Some("k8s"), None)
                .await?;
        }
        info!(job = %job.id, "Talos phase complete; starting k8s phase");
    } else {
        upgrade_job::update_job_status(pool, job.id, "completed", None).await?;
    }
    Ok(())
}

// ── Phase 2: in-place Kubernetes upgrade (no reboots) ──────────────────────

async fn run_k8s_phase(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    job: upgrade_job::UpgradeJob,
) -> Result<(), crate::AppError> {
    let Some(target_k8s) = job
        .target_k8s_version
        .clone()
        .filter(|v| !v.trim().is_empty())
    else {
        upgrade_job::update_job_status(pool, job.id, "completed", None).await?;
        return Ok(());
    };
    let steps: Vec<String> = job
        .steps
        .clone()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(|s: &Vec<String>| !s.is_empty())
        .unwrap_or_else(|| vec![target_k8s.clone()]);

    let targets = upgrade_job::list_targets(pool, job.id).await?;
    if targets.is_empty() {
        upgrade_job::update_job_status(pool, job.id, "completed", None).await?;
        return Ok(());
    }
    let cluster_id = targets[0].cluster_id;

    let done = targets
        .iter()
        .map(|t| completed_steps(t).len())
        .min()
        .unwrap_or(0);

    if done >= steps.len() {
        let any_fail = targets.iter().any(|t| t.status == "failed");
        upgrade_job::update_job_status(
            pool,
            job.id,
            if any_fail { "failed" } else { "completed" },
            None,
        )
        .await?;
        refresh_k8s_inventory(pool, cluster_id, &target_k8s).await;
        return Ok(());
    }

    let step_to = steps[done].clone();
    let step_from = if done == 0 {
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string())
            .cluster_k8s_version(cluster_id)
            .await
            .unwrap_or_default()
    } else {
        steps[done - 1].clone()
    };

    let dispatched = targets.iter().any(|t| t.k8s_version.as_deref() == Some(step_to.as_str()));

    if !dispatched {
        if cancel_requested(pool, job.id).await? {
            upgrade_job::update_job_status(pool, job.id, "cancelled", Some("cancel requested"))
                .await?;
            return Ok(());
        }
        if step_from.trim().is_empty() {
            return Err(crate::AppError::Internal(format!(
                "k8s phase: could not determine current version for step to {step_to}"
            )));
        }
        let controller =
            ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
        let Some((cp_addr,)) = pick_control_plane_address(&targets).await else {
            return Err(crate::AppError::InvalidInput(
                "k8s phase: no control-plane machine address available".into(),
            ));
        };
        for t in &targets {
            upgrade_job::update_target_fields(
                pool, t.id, Some("running"), None, Some(&step_to), Some("k8s"), None,
            )
            .await?;
        }
        info!(job = %job.id, step = %step_to, from = %step_from, cp = %cp_addr, "dispatching talosctl upgrade-k8s");
        if let Err(e) = controller
            .run_k8s_upgrade(cluster_id, &cp_addr, &step_from, &step_to)
            .await
        {
            // Leave targets marked running+step so the next tick retries
            // (upgrade-k8s from==to degrades to a no-op plan once converged).
            warn!(job = %job.id, error = %e, "k8s upgrade dispatch failed; retrying next tick");
        }
    }

    poll_k8s_step(pool, cluster_id, &step_to, &targets).await
}

async fn poll_k8s_step(
    pool: &DbPool,
    cluster_id: Uuid,
    step_to: &str,
    targets: &[UpgradeJobTarget],
) -> Result<(), crate::AppError> {
    let controller = ClusterController::with_context(pool.clone(), String::new(), String::new());
    match controller.cluster_k8s_version(cluster_id).await {
        Ok(ver) => {
            let reached = cmp_k8s_versions(&ver, step_to)
                .map(|o| o != std::cmp::Ordering::Less)
                .unwrap_or(false);
            if reached {
                for t in targets {
                    let mut steps_done = completed_steps(t);
                    if !steps_done.iter().any(|s| s == step_to) {
                        steps_done.push(step_to.to_string());
                    }
                    upgrade_job::update_target_fields(
                        pool,
                        t.id,
                        Some("completed"),
                        None,
                        None,
                        Some("k8s"),
                        Some(&serde_json::to_string(&steps_done).unwrap_or_default()),
                    )
                    .await?;
                }
                info!(cluster_version = %ver, step = %step_to, "k8s step complete");
            }
        }
        Err(e) => {
            warn!(step = %step_to, error = %e, "k8s step poll failed");
        }
    }
    Ok(())
}

async fn refresh_k8s_inventory(pool: &DbPool, cluster_id: Uuid, final_version: &str) {
    if let Ok(Some(mut c)) = repos::cluster::get(pool, cluster_id).await {
        c.control_plane_version = final_version.to_string();
        c.updated_at = chrono::Utc::now();
        let _ = repos::cluster::update(pool, &c).await;
    }
    if let Ok(ms) = repos::machine::list_by_cluster(pool, cluster_id).await {
        for mut m in ms {
            m.updated_at = chrono::Utc::now();
            let _ = repos::machine::update(pool, &m).await;
        }
    }
}

fn completed_steps(t: &UpgradeJobTarget) -> Vec<String> {
    t.completed_steps
        .clone()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

async fn pick_control_plane_address(targets: &[UpgradeJobTarget]) -> Option<(String,)> {
    let cp = targets
        .iter()
        .find(|t| {
            is_cp(
                t.machine_type
                    .clone()
                    .unwrap_or_default()
                    .as_str(),
            )
        })
        .or_else(|| targets.first());
    cp.and_then(|t| {
        t.address
            .clone()
            .filter(|a| !a.trim().is_empty())
            .map(|a| (a,))
    })
}

async fn cancel_requested(pool: &DbPool, job_id: Uuid) -> Result<bool, crate::AppError> {
    Ok(upgrade_job::get_job(pool, job_id)
        .await?
        .map(|j| j.cancel_requested)
        .unwrap_or(false))
}

fn norm_v(v: &str) -> String {
    let t = v.trim();
    if t.starts_with('v') {
        t.to_string()
    } else {
        format!("v{t}")
    }
}

fn is_cp(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    t == "control-plane" || t == "controlplane"
}
