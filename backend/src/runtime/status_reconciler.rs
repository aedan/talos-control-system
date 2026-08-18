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

    let version = TalosctlClient::get_version(&machine.address, talosconfig.as_deref()).await;

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
