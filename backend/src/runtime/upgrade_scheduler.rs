use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

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
    // Honor a cancel only when nothing is actively in flight. A k8s step, once
    // dispatched, runs `talosctl upgrade-k8s` to convergence (minutes of image
    // pre-pull + in-place roll) and can't be safely aborted mid-flight; letting
    // it finish and then stopping is correct. Force-cancelling here unconditionally
    // would stamp the job "cancelled" even after the cluster already converged
    // (observed: job showed cancelled while the live API server was at the target
    // version). When a target IS running, let the phase loops run so the in-flight
    // unit completes; their own per-unit cancel checks (run_talos_phase /
    // run_k8s_phase) stop it before the next unit starts.
    if job.cancel_requested {
        let targets = upgrade_job::list_targets(pool, job_id).await?;
        let any_in_flight = targets.iter().any(|t| t.status == "running");
        if !any_in_flight {
            // Still give converged work a chance to be recorded complete before
            // we overwrite the status: if the phase's completion check would
            // already mark it done, defer to that by falling through.
            let all_done = !targets.is_empty()
                && targets.iter().all(|t| t.status == "completed" || t.status == "failed");
            if !all_done {
                upgrade_job::update_job_status(pool, job_id, "cancelled", Some("cancel requested"))
                    .await?;
                return Ok(());
            }
        }
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
    reap_upgrade_clients(pool).await?;

    let mut targets = upgrade_job::list_targets(pool, job.id).await?;
    let want = desired_talos_version(&job, &targets);
    for t in targets.clone() {
        if t.status == "running" || t.status == "failed" {
            poll_talos_target(pool, sqlite_path, jwt_secret, &job, &t, want.as_deref()).await?;
        }
    }
    targets = upgrade_job::list_targets(pool, job.id).await?;

    let any_active = targets
        .iter()
        .any(|t| t.status == "pending" || t.status == "running" || (t.status == "failed" && target_is_cp(&t)));
    if !any_active {
        transition_after_talos(pool, &job, &targets).await?;
        return Ok(());
    }
    // A failed controller still blocks the roll. Do not finish the job, and do
    // not move on to Kubernetes, while one is down.
    if targets.iter().any(|t| target_is_cp(t) && t.status == "failed") {
        return Ok(());
    }

    let max = job.max_unavailable.max(1) as usize;
    let views: Vec<TargetView> = targets
        .iter()
        .map(|t| TargetView {
            is_cp: target_is_cp(t),
            status: t.status.clone(),
        })
        .collect();
    let start_idx = select_to_start(&views, max);

    for idx in start_idx {
        let t = &targets[idx];
        if cancel_requested(pool, job.id).await? {
            upgrade_job::update_job_status(pool, job.id, "cancelled", Some("cancel requested"))
                .await?;
            return Ok(());
        }
        if inflight_contains(t.id) {
            continue;
        }
        upgrade_job::update_target_status(pool, t.id, "running", None).await?;
        let image = t.image.clone().unwrap_or_else(|| job.image.clone());
        let controller =
            ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
        let is_cp = target_is_cp(t);
        match controller.drain_machine_for_upgrade(t.machine_id).await {
            Ok(Some(node)) => {
                if is_cp {
                    info!(machine = %t.machine_id, node, "cordoned control plane before upgrade");
                } else {
                    info!(machine = %t.machine_id, node, "drained worker before upgrade");
                }
            }
            Ok(None) => {}
            Err(e) => {
                upgrade_job::update_target_status(pool, t.id, "failed", Some(&format!("drain failed: {e}"))).await?;
                // A controller that failed to cordon is still the machine we must
                // not skip. Stop starting anything else this tick.
                if is_cp {
                    return Ok(());
                }
                continue;
            }
        }
        spawn_upgrade_client(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string(), t.id, t.machine_id, image);
    }
    Ok(())
}

async fn poll_talos_target(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    _job: &upgrade_job::UpgradeJob,
    t: &UpgradeJobTarget,
    want: Option<&str>,
) -> Result<(), crate::AppError> {
    let Some(want) = want else {
        return Ok(());
    };
    let controller =
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
    let ver = match controller.machine_version(t.machine_id).await {
        Ok(ver) => ver,
        Err(e) => {
            // Unreachable while rebooting. Keep the slot. Do not mark it failed.
            clear_stuck_since(t.id);
            warn!(machine = %t.machine_id, error = %e, "version poll during Talos upgrade");
            return Ok(());
        }
    };
    let want_n = norm_v(want);
    let got_n = norm_v(&ver);
    let version_ok = got_n == want_n || got_n.contains(&want_n) || want_n.contains(&got_n);
    let is_cp = target_is_cp(t);

    if is_cp {
        match controller.etcd_healthy(t.machine_id).await {
            Ok(true) => clear_stuck_since(t.id),
            Ok(false) => {
                if stuck_long_enough(t.id) {
                    let msg = "Talos API is up but etcd has not become healthy; holding this controller";
                    if t.error.as_deref() != Some(msg) || t.status != "running" {
                        upgrade_job::update_target_status(pool, t.id, "running", Some(msg)).await?;
                    }
                    info!(machine = %t.machine_id, "{msg}");
                }
                return Ok(());
            }
            Err(e) => {
                warn!(machine = %t.machine_id, error = %e, "etcd health poll during Talos upgrade");
                return Ok(());
            }
        }
    }

    if !version_ok {
        return Ok(());
    }

    let ready = match controller.kubernetes_node_ready(t.machine_id).await {
        Ok(ready) => ready,
        Err(e) => {
            warn!(machine = %t.machine_id, error = %e, "node readiness poll during Talos upgrade");
            return Ok(());
        }
    };
    if !ready {
        return Ok(());
    }

    // Uncordon only once the node is Ready. Leave the target running so the
    // next tick retries when the API was down for the uncordon call.
    if controller.uncordon_machine(t.machine_id).await.is_err() {
        warn!(machine = %t.machine_id, "uncordon failed after the node was Ready");
        return Ok(());
    }
    upgrade_job::update_target_status(pool, t.id, "completed", None).await?;
    if let Ok(Some(mut m)) = repos::machine::get(pool, t.machine_id).await {
        m.talos_version = ver;
        m.updated_at = chrono::Utc::now();
        let _ = repos::machine::update(pool, &m).await;
    }
    Ok(())
}

fn desired_talos_version(job: &upgrade_job::UpgradeJob, targets: &[UpgradeJobTarget]) -> Option<String> {
    if let Some(v) = job.target_talos_version.as_ref().filter(|v| !v.trim().is_empty()) {
        return Some(norm_v(v));
    }
    let image = targets
        .iter()
        .find_map(|t| t.image.as_deref())
        .unwrap_or(job.image.as_str());
    version_from_image(image)
}

fn version_from_image(image: &str) -> Option<String> {
    let tag = image.rsplit(':').next().unwrap_or("").trim();
    if tag.is_empty() || tag.contains('/') {
        return None;
    }
    let ok = tag.starts_with('v') || tag.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
    if ok { Some(norm_v(tag)) } else { None }
}

const UPGRADE_CLIENT_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const ETCD_HOLD_AFTER: Duration = Duration::from_secs(10 * 60);

enum ClientFinish {
    /// Install started, the client timed out, or the node rebooted under the client.
    RebootInProgress,
    /// The client failed before the installer started. The slot stays taken.
    FailedBeforeInstall(String),
}

static UPGRADE_INFLIGHT: LazyLock<Mutex<HashMap<Uuid, tokio::task::JoinHandle<ClientFinish>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ETCD_STUCK_SINCE: LazyLock<Mutex<HashMap<Uuid, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn inflight_contains(id: Uuid) -> bool {
    UPGRADE_INFLIGHT.lock().map(|g| g.contains_key(&id)).unwrap_or(false)
}

fn spawn_upgrade_client(
    pool: DbPool,
    sqlite_path: String,
    jwt_secret: String,
    target_id: Uuid,
    machine_id: Uuid,
    image: String,
) {
    let handle = tokio::spawn(async move {
        let controller = ClusterController::with_context(pool, sqlite_path, jwt_secret);
        match tokio::time::timeout(
            UPGRADE_CLIENT_TIMEOUT,
            controller.upgrade_machine(machine_id, &image),
        )
        .await
        {
            Ok(Ok(())) => ClientFinish::RebootInProgress,
            Ok(Err(e)) => {
                let msg = e.to_string();
                if upgrade_failure_is_reboot(&msg) {
                    ClientFinish::RebootInProgress
                } else {
                    ClientFinish::FailedBeforeInstall(msg)
                }
            }
            Err(_) => ClientFinish::RebootInProgress,
        }
    });
    if let Ok(mut guard) = UPGRADE_INFLIGHT.lock() {
        guard.insert(target_id, handle);
    }
    info!(machine = %machine_id, "Talos upgrade client started");
}

async fn reap_upgrade_clients(pool: &DbPool) -> Result<(), crate::AppError> {
    let finished: Vec<Uuid> = {
        let Ok(guard) = UPGRADE_INFLIGHT.lock() else {
            return Ok(());
        };
        guard
            .iter()
            .filter(|(_, handle)| handle.is_finished())
            .map(|(id, _)| *id)
            .collect()
    };
    for id in finished {
        let Some(handle) = UPGRADE_INFLIGHT.lock().ok().and_then(|mut g| g.remove(&id)) else {
            continue;
        };
        match handle.await {
            Ok(ClientFinish::RebootInProgress) => {
                info!(target = %id, "Talos upgrade client finished; waiting for the node to return");
            }
            Ok(ClientFinish::FailedBeforeInstall(msg)) => {
                let current = upgrade_job::get_target(pool, id).await?;
                if current.as_ref().map(|t| t.status.as_str()) == Some("running") {
                    let clipped = clip_upgrade_error(&msg);
                    upgrade_job::update_target_status(pool, id, "failed", Some(&clipped)).await?;
                    warn!(target = %id, "Talos upgrade failed before the install started");
                }
            }
            Err(e) => warn!(target = %id, error = %e, "Talos upgrade client task stopped"),
        }
    }
    Ok(())
}

fn clip_upgrade_error(msg: &str) -> String {
    const MAX: usize = 500;
    if msg.len() <= MAX {
        msg.to_string()
    } else {
        let mut end = MAX;
        while end > 0 && !msg.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &msg[..end])
    }
}

/// A disconnect after the installer has written the image is the reboot, not a
/// failed upgrade. Pre-install errors (pull, pre-flight, config) are failures.
fn upgrade_failure_is_reboot(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    let installed = m.contains("installation of") && m.contains("complete");
    let rebooting = m.contains("reboot sequence")
        || m.contains("upgraded successfully")
        || m.contains("waiting for the machine to reboot");
    installed || rebooting
}

fn stuck_long_enough(id: Uuid) -> bool {
    let Ok(mut guard) = ETCD_STUCK_SINCE.lock() else {
        return false;
    };
    let since = guard.entry(id).or_insert_with(Instant::now);
    since.elapsed() >= ETCD_HOLD_AFTER
}

fn clear_stuck_since(id: Uuid) {
    if let Ok(mut guard) = ETCD_STUCK_SINCE.lock() {
        guard.remove(&id);
    }
}

struct TargetView {
    is_cp: bool,
    status: String,
}

fn target_is_cp(t: &UpgradeJobTarget) -> bool {
    t.machine_type.as_deref().map(is_cp).unwrap_or(false)
}

/// Indexes of pending targets to start this tick, in list order.
///
/// A control plane that is `running` or `failed` blocks every later target.
/// At most one control plane is started. Workers use `max_unavailable`.
fn select_to_start(targets: &[TargetView], max_unavailable: usize) -> Vec<usize> {
    let cp_blocked = targets
        .iter()
        .any(|t| t.is_cp && t.status != "completed" && t.status != "pending");
    let mut running = targets.iter().filter(|t| t.status == "running").count();
    let mut selected: Vec<usize> = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        if t.status != "pending" {
            continue;
        }
        if running >= max_unavailable || cp_blocked {
            break;
        }
        if t.is_cp {
            selected.push(i);
            // One controller at a time, and nothing behind it this tick.
            break;
        }
        selected.push(i);
        running += 1;
    }
    selected
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
    if let Some(ver) = job.target_talos_version.as_deref() {
        // The cluster row is what the UI calls "current". Machine rows are
        // updated per node as they come back; this is the fleet summary.
        let cluster_id = targets.first().map(|t| t.cluster_id);
        if let Some(cluster_id) = cluster_id {
            refresh_talos_inventory(pool, cluster_id, ver).await;
        }
    }
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

async fn refresh_talos_inventory(pool: &DbPool, cluster_id: Uuid, version: &str) {
    let ver = crate::utils::talos_version::normalize_talos_version(version);
    if ver.is_empty() {
        return;
    }
    if let Ok(Some(mut c)) = repos::cluster::get(pool, cluster_id).await {
        if crate::utils::talos_version::normalize_talos_version(&c.talos_version) == ver {
            return;
        }
        c.talos_version = ver;
        c.updated_at = chrono::Utc::now();
        let _ = repos::cluster::update(pool, &c).await;
    }
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
        let Some((cp_addr,)) = pick_control_plane_address(pool, &targets).await else {
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

    poll_k8s_step(pool, sqlite_path, jwt_secret, cluster_id, &step_to, &targets).await
}

async fn poll_k8s_step(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    cluster_id: Uuid,
    step_to: &str,
    targets: &[UpgradeJobTarget],
) -> Result<(), crate::AppError> {
    let controller =
        ClusterController::with_context(pool.clone(), sqlite_path.to_string(), jwt_secret.to_string());
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

async fn pick_control_plane_address(
    pool: &DbPool,
    targets: &[UpgradeJobTarget],
) -> Option<(String,)> {
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
        .or_else(|| targets.first())?;

    // Resolve the CP node's effective endpoint: prefer its Siderolink tunnel IP
    // when connected, else the LAN address recorded on the target. This lets a
    // remote k8s upgrade run against a NAT'd control plane through the tunnel.
    if let Ok(Some(machine)) = repos::machine::get(pool, cp.machine_id).await {
        let endpoint = crate::controllers::cluster::effective_endpoint(pool, &machine)
            .await
            .unwrap_or_else(|_| machine.address.clone());
        if !endpoint.trim().is_empty() {
            return Some((endpoint,));
        }
    }
    cp.address
        .clone()
        .filter(|a| !a.trim().is_empty())
        .map(|a| (a,))
}

async fn cancel_requested(pool: &DbPool, job_id: Uuid) -> Result<bool, crate::AppError> {
    Ok(upgrade_job::get_job(pool, job_id)
        .await?
        .map(|j| j.cancel_requested)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(is_cp: bool, status: &str) -> TargetView {
        TargetView {
            is_cp,
            status: status.to_string(),
        }
    }

    #[test]
    fn failed_controller_occupies_the_slot() {
        let targets = vec![
            view(true, "failed"),
            view(true, "pending"),
            view(false, "pending"),
        ];
        assert!(select_to_start(&targets, 1).is_empty());
    }

    #[test]
    fn failed_controller_blocks_the_workers_behind_it() {
        let targets = vec![view(true, "failed"), view(false, "pending")];
        assert!(select_to_start(&targets, 1).is_empty());
    }

    #[test]
    fn running_controller_blocks_the_next_one() {
        let targets = vec![view(true, "running"), view(true, "pending")];
        assert!(select_to_start(&targets, 1).is_empty());
    }

    #[test]
    fn controllers_start_before_workers() {
        let targets = vec![
            view(true, "pending"),
            view(true, "pending"),
            view(false, "pending"),
        ];
        assert_eq!(select_to_start(&targets, 1), vec![0]);
    }

    #[test]
    fn workers_start_after_controllers_complete() {
        let targets = vec![
            view(true, "completed"),
            view(true, "completed"),
            view(false, "pending"),
            view(false, "pending"),
        ];
        assert_eq!(select_to_start(&targets, 1), vec![2]);
    }

    #[test]
    fn post_install_unavailable_is_a_reboot() {
        let msg = "installation of v1.13.11 complete\n192.168.7.244: unavailable, retrying...";
        assert!(upgrade_failure_is_reboot(msg));
    }

    #[test]
    fn image_pull_failure_is_not_a_reboot() {
        let msg = "failed to pull image: manifest unknown";
        assert!(!upgrade_failure_is_reboot(msg));
    }

    #[test]
    fn unmount_failure_before_install_is_not_a_reboot() {
        let msg = "task: unmountPodMounts action: START";
        assert!(!upgrade_failure_is_reboot(msg));
    }

    #[test]
    fn version_comes_from_the_image_tag() {
        assert_eq!(
            version_from_image("factory.talos.dev/metal-installer/abc:v1.13.11").as_deref(),
            Some("v1.13.11")
        );
        assert!(version_from_image("not-an-image").is_none());
    }
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
