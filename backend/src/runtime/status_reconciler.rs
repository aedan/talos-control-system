//! Background reconciler that keeps `machines.status` in sync with reality.
//!
//! Machine status is otherwise only written by explicit API actions
//! (install / bootstrap / import) or the (dead) siderolink flow, so
//! out-of-band-provisioned machines stay stuck at `pending`/`booting`/
//! `installing` forever. This scheduler probes each machine's Talos API and
//! marks it `running` (with a fresh `talos_version`) when reachable, or
//! `offline` when it is not. Machines that are part of an in-flight
//! provision job are left alone so we never clobber a live install.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use tokio::sync::Semaphore;
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::models::machine::Machine;
use crate::db::pool::DbPool;
use crate::db::repos;
use crate::integration::talosctl::TalosctlClient;
use crate::utils::secrets;
use crate::AppError;

const TICK_SECS: u64 = 60;
const LOCK_TTL_SECS: i64 = 60;
const PROBE_CONCURRENCY: usize = 8;

/// Statuses we actively reconcile. `destroying`/`failed` are left untouched.
const RECONCILABLE: &[&str] = &[
    "pending",
    "booting",
    "configuring",
    "installing",
    "running",
    "offline",
];

/// Cluster statuses the reconciler is allowed to rewrite. Transient
/// provisioning states (`importing`, `destroying`, `failed`, ...) are left
/// alone so we never clobber an in-flight operation.
const CLUSTER_RECONCILABLE: &[&str] = &["pending", "unknown", "offline", "running"];

/// Derive a cluster's status from the set of its machines' statuses.
///
/// A cluster is `running` as soon as any of its machines is up. With nothing
/// up, an actively-transitioning machine means it is still coming up
/// (`pending`), everything down means `offline`, and no machines at all means
/// `unknown`.
fn derive_cluster_status(statuses: &[&str]) -> &'static str {
    if statuses.is_empty() {
        return "unknown";
    }
    if statuses.iter().any(|s| *s == "running") {
        return "running";
    }
    const TRANSITIONAL: &[&str] = &["booting", "configuring", "installing"];
    if statuses.iter().any(|s| TRANSITIONAL.contains(s)) {
        return "pending";
    }
    if statuses.iter().all(|s| *s == "offline") {
        return "offline";
    }
    "pending"
}

pub fn spawn_status_reconciler(
    pool: DbPool,
    jwt_secret: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Machine status reconciler started");
        loop {
            match crate::runtime::ha::try_acquire(&pool, "status_reconciler", LOCK_TTL_SECS).await {
                Ok(true) => {
                    if let Err(e) = tick(&pool, &jwt_secret).await {
                        warn!(error = %e, "Status reconciler tick failed");
                    }
                }
                Ok(false) => {}
                Err(e) => warn!(error = %e, "HA lock acquire failed (status_reconciler)"),
            }
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        }
    })
}

async fn tick(pool: &DbPool, jwt_secret: &str) -> Result<(), AppError> {
    let machines = repos::machine::list(pool).await?;
    let in_flight = in_flight_machine_ids(pool).await;

    let candidates: Vec<Machine> = machines
        .into_iter()
        .filter(|m| m.cluster_id.is_some())
        .filter(|m| !m.address.trim().is_empty())
        .filter(|m| RECONCILABLE.contains(&m.status.as_str()))
        .filter(|m| !in_flight.contains(&m.id))
        .collect();

    if candidates.is_empty() {
        return Ok(());
    }

    let probed_cluster_ids: HashSet<Uuid> = candidates
        .iter()
        .filter_map(|m| m.cluster_id)
        .collect();

    let sem = Arc::new(Semaphore::new(PROBE_CONCURRENCY));
    let futures = candidates.into_iter().map(|m| {
        let pool = pool.clone();
        let secret = jwt_secret.to_string();
        let sem = Arc::clone(&sem);
        async move {
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return None,
            };
            Some(probe_and_update(&pool, &secret, m).await)
        }
    });

    for result in join_all(futures).await {
        if let Some(Err(e)) = result {
            warn!(error = %e, "status probe failed");
        }
    }

    // Re-read machine rows so cluster derivation reflects the statuses the
    // probes just wrote (the `machines` slice above is pre-probe).
    let fresh = repos::machine::list(pool).await?;
    reconcile_cluster_statuses(pool, &fresh, &probed_cluster_ids).await
}

/// Re-derive `clusters.status` for every cluster that had a machine probed this
/// tick. Machine status is the source of truth; the cluster row is otherwise
/// only written by explicit import/provision flows, so an out-of-band-built
/// cluster stays stuck at `pending` without this.
async fn reconcile_cluster_statuses(
    pool: &DbPool,
    machines: &[Machine],
    cluster_ids: &HashSet<Uuid>,
) -> Result<(), AppError> {
    for cluster_id in cluster_ids {
        let Some(cluster) = repos::cluster::get(pool, *cluster_id).await? else {
            continue;
        };
        if !CLUSTER_RECONCILABLE.contains(&cluster.status.as_str()) {
            continue;
        }
        let member_statuses: Vec<&str> = machines
            .iter()
            .filter(|m| m.cluster_id == Some(*cluster_id))
            .map(|m| m.status.as_str())
            .collect();
        let derived = derive_cluster_status(&member_statuses);
        if derived != cluster.status {
            repos::cluster::update_status(pool, cluster.id, derived).await?;
            info!(
                cluster_id = %cluster_id,
                name = %cluster.name,
                old_status = %cluster.status,
                new_status = derived,
                "Cluster status reconciled"
            );
        }
    }
    Ok(())
}

/// Machine ids referenced by any active provision job (never clobber these).
async fn in_flight_machine_ids(pool: &DbPool) -> HashSet<Uuid> {
    let mut ids = HashSet::new();
    if let Ok(jobs) = repos::provision_job::list_active(pool).await {
        for job in jobs {
            if let Some(payload) = job.payload.as_ref() {
                if let Ok(p) = serde_json::from_str::<ProvisionJobPayload>(payload) {
                    for id in p.machine_ids {
                        ids.insert(id);
                    }
                }
            }
        }
    }
    ids
}

#[derive(serde::Deserialize)]
struct ProvisionJobPayload {
    #[serde(default)]
    machine_ids: Vec<Uuid>,
}

async fn probe_and_update(
    pool: &DbPool,
    jwt_secret: &str,
    machine: Machine,
) -> Result<(), AppError> {
    let cluster_id = machine.cluster_id.unwrap();
    let machine_id = machine.id;
    let hostname = machine.hostname.clone();
    let old_status = machine.status.clone();
    let old_version = machine.talos_version.clone();

    let cluster = repos::cluster::get(pool, cluster_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("cluster {cluster_id}")))?;

    let talosconfig = match &cluster.talosconfig {
        Some(enc) => Some(secrets::decrypt(jwt_secret, enc)?),
        None => None,
    };

    let version = {
        // Prefer the Siderolink tunnel IP when connected; else the LAN address.
        let endpoint = crate::controllers::cluster::effective_endpoint(pool, &machine)
            .await
            .unwrap_or_else(|_| machine.address.clone());
        TalosctlClient::get_version(&endpoint, talosconfig.as_deref()).await
    };

    let mut updated = machine;
    match version {
        Ok(v) => {
            updated.status = "running".to_string();
            updated.talos_version = v;
        }
        Err(_) => {
            updated.status = "offline".to_string();
        }
    }
    updated.updated_at = chrono::Utc::now();

    if updated.status != old_status || updated.talos_version != old_version {
        repos::machine::update(pool, &updated).await?;
        info!(
            machine_id = %machine_id,
            hostname = %hostname,
            old_status = %old_status,
            new_status = %updated.status,
            talos_version = %updated.talos_version,
            "Machine status reconciled"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::derive_cluster_status;

    #[test]
    fn empty_is_unknown() {
        assert_eq!(derive_cluster_status(&[]), "unknown");
    }

    #[test]
    fn any_running_makes_cluster_running() {
        assert_eq!(derive_cluster_status(&["running", "offline"]), "running");
        assert_eq!(
            derive_cluster_status(&["offline", "offline", "running"]),
            "running"
        );
    }

    #[test]
    fn transitional_without_running_is_pending() {
        assert_eq!(derive_cluster_status(&["booting", "offline"]), "pending");
        assert_eq!(derive_cluster_status(&["installing"]), "pending");
        assert_eq!(derive_cluster_status(&["configuring", "pending"]), "pending");
    }

    #[test]
    fn all_offline_is_offline() {
        assert_eq!(derive_cluster_status(&["offline", "offline"]), "offline");
    }

    #[test]
    fn all_pending_is_pending() {
        assert_eq!(derive_cluster_status(&["pending", "pending"]), "pending");
    }
}
