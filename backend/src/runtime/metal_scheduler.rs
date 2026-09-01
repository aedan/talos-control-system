//! Metal provision job worker: BMC PXE → wait installer → install → bootstrap.

use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::MetalConfig;
use crate::controllers::cluster::ClusterController;
use crate::controllers::provision::ProvisionController;
use crate::db::pool::DbPool;
use crate::db::repos::{self, provision_job::ProvisionJob};
use crate::integration::bmc::{BootTarget, BmcCredentials, BmcSession};
use crate::utils::secrets;
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetalJobPayload {
    pub machine_ids: Vec<Uuid>,
    pub artifact_id: Option<Uuid>,
    pub install_disk: Option<String>,
    pub auto_bootstrap: bool,
    pub current_machine_index: usize,
    pub step: String,
    pub steps_log: Vec<String>,
    /// Shared config artifact generated once for the whole job so every
    /// machine uses the same PKI / cluster id / tokens.
    #[serde(default)]
    pub job_artifact_id: Option<Uuid>,
}

pub fn spawn_metal_scheduler(
    pool: DbPool,
    sqlite_path: String,
    jwt_secret: String,
    metal: MetalConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("Metal provision scheduler started");
        loop {
            match crate::runtime::ha::try_acquire(&pool, "metal_scheduler", 20).await {
                Ok(true) => {
                    if let Err(e) =
                        tick(&pool, &sqlite_path, &jwt_secret, &metal).await
                    {
                        warn!(error = %e, "Metal scheduler tick failed");
                    }
                }
                Ok(false) => {}
                Err(e) => warn!(error = %e, "HA lock acquire failed (metal)"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

async fn tick(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    metal: &MetalConfig,
) -> Result<(), AppError> {
    let jobs = repos::provision_job::list_active(pool).await?;
    for job in jobs {
        if job.kind != "metal_provision" {
            continue;
        }
        if let Err(e) = run_job(pool, sqlite_path, jwt_secret, metal, job).await {
            warn!(error = %e, "Metal job step failed");
        }
    }
    Ok(())
}

async fn run_job(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    metal: &MetalConfig,
    job: ProvisionJob,
) -> Result<(), AppError> {
    let mut payload: MetalJobPayload = job
        .payload
        .as_ref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or_default();

    if payload.machine_ids.is_empty() {
        repos::provision_job::update_status(
            pool,
            job.id,
            "failed",
            Some("no machineIds in payload"),
            None,
        )
        .await?;
        return Ok(());
    }

    if payload.current_machine_index >= payload.machine_ids.len() {
        repos::provision_job::update_status(pool, job.id, "succeeded", None, None).await?;
        return Ok(());
    }

    let machine_id = payload.machine_ids[payload.current_machine_index];
    let mut machine = repos::machine::get(pool, machine_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("machine {machine_id}")))?;

    let log = |payload: &mut MetalJobPayload, msg: &str| {
        info!(job_id = %job.id, machine_id = %machine_id, "{msg}");
        payload.steps_log.push(format!("{} {msg}", Utc::now().to_rfc3339()));
        if payload.steps_log.len() > 100 {
            let drain = payload.steps_log.len() - 100;
            payload.steps_log.drain(0..drain);
        }
    };

    // Advance based on step
    if payload.step.is_empty() || payload.step == "pending" {
        payload.step = "set_pxe".into();
    }

    let step = payload.step.clone();
    match step.as_str() {
        "set_pxe" => {
            if machine.has_bmc() {
                match open_bmc(pool, jwt_secret, metal, &machine).await {
                    Ok(sess) => {
                        if let Err(e) = sess.set_boot(BootTarget::Pxe, true).await {
                            fail_job(pool, job.id, &mut payload, &format!("set PXE boot: {e}"))
                                .await?;
                            return Ok(());
                        }
                        log(&mut payload, "BMC boot device set to PXE once");
                    }
                    Err(e) => {
                        log(
                            &mut payload,
                            &format!("BMC unavailable ({e}); assuming manual PXE"),
                        );
                    }
                }
            } else {
                log(&mut payload, "No BMC; assuming host already PXE-capable");
            }
            payload.step = "power".into();
            save_progress(pool, job.id, "running", &payload).await?;
        }
        "power" => {
            if machine.has_bmc() {
                match open_bmc(pool, jwt_secret, metal, &machine).await {
                    Ok(sess) => {
                        let state = sess.get_power_state().await.unwrap_or(
                            crate::integration::bmc::PowerState::Unknown,
                        );
                        let action = if state == crate::integration::bmc::PowerState::On {
                            "cycle"
                        } else {
                            "on"
                        };
                        if let Err(e) = sess.power(action).await {
                            fail_job(pool, job.id, &mut payload, &format!("power {action}: {e}"))
                                .await?;
                            return Ok(());
                        }
                        machine.last_power_state = action.into();
                        machine.updated_at = Utc::now();
                        let _ = repos::machine::update(pool, &machine).await;
                        log(&mut payload, &format!("BMC power {action}"));
                    }
                    Err(e) => log(&mut payload, &format!("skip power: {e}")),
                }
            }
            payload.step = "wait_installer".into();
            save_progress(pool, job.id, "waiting_installer", &payload).await?;
        }
        "wait_installer" => {
            // During PXE boot the installer may be reachable on either the
            // DHCP lease address or the machine's static address (firmware
            // may assign a static IP that overrides DHCP). Try both.
            let lease_ip = if !machine.mac_address.is_empty() {
                repos::dhcp_lease::get_by_mac(pool, &machine.mac_address)
                    .await
                    .ok()
                    .flatten()
                    .map(|l| l.ip)
                    .filter(|ip| !ip.is_empty())
            } else {
                None
            };
            let mut candidates: Vec<String> = Vec::new();
            if let Some(ref ip) = lease_ip {
                candidates.push(ip.clone());
            }
            if !machine.address.is_empty() && !candidates.contains(&machine.address) {
                candidates.push(machine.address.clone());
            }
            if candidates.is_empty() {
                log(&mut payload, "waiting for machine address / DHCP lease");
                save_progress(pool, job.id, "waiting_installer", &payload).await?;
                return Ok(());
            }
            if lease_ip.is_some() && !machine.address.is_empty() {
                log(
                    &mut payload,
                    &format!(
                        "probing installer: lease {} static {}",
                        lease_ip.as_deref().unwrap_or("?"),
                        machine.address
                    ),
                );
            }
            let ctrl = ClusterController::with_context(
                pool.clone(),
                sqlite_path.to_string(),
                jwt_secret.to_string(),
            );
            let mut disks: Option<Vec<serde_json::Value>> = None;
            let mut last_err = String::new();
            for cand in &candidates {
                match ctrl.list_disks(machine_id, Some(cand)).await {
                    Ok(d) => {
                        disks = Some(d);
                        break;
                    }
                    Err(e) => {
                        last_err = format!("{cand}: {e}");
                    }
                }
            }
            match disks {
                Some(d) => {
                    log(
                        &mut payload,
                        &format!("installer reachable ({} disks)", d.len()),
                    );
                    if machine.install_disk.is_empty() {
                        if let Some(disk) = payload.install_disk.clone() {
                            machine.install_disk = disk;
                        } else if let Some(best) = d.iter().max_by_key(|x| {
                            x.get("size").and_then(|v| v.as_u64()).unwrap_or(0)
                        }) {
                            if let Some(name) = best
                                .get("deviceName")
                                .or_else(|| best.get("name"))
                                .and_then(|v| v.as_str())
                            {
                                machine.install_disk = name.to_string();
                                log(&mut payload, &format!("auto-selected disk {name}"));
                            }
                        }
                        machine.updated_at = Utc::now();
                        let _ = repos::machine::update(pool, &machine).await;
                    }
                    payload.step = "install".into();
                    save_progress(pool, job.id, "installing", &payload).await?;
                }
                None => {
                    log(&mut payload, &format!("installer not ready: {last_err}"));
                    save_progress(pool, job.id, "waiting_installer", &payload).await?;
                }
            }
        }
        "install" => {
            let ctrl = ClusterController::with_context(
                pool.clone(),
                sqlite_path.to_string(),
                jwt_secret.to_string(),
            );
            // Installer still runs from PXE RAM. It may be reachable on the
            // DHCP lease or the machine's static address (firmware may assign
            // a static IP that overrides DHCP). Try the lease first, then the
            // static address.
            let lease_ip = if !machine.mac_address.is_empty() {
                repos::dhcp_lease::get_by_mac(pool, &machine.mac_address)
                    .await
                    .ok()
                    .flatten()
                    .map(|l| l.ip)
                    .filter(|ip| !ip.is_empty())
            } else {
                None
            };
            let mut candidates: Vec<String> = Vec::new();
            if let Some(ip) = lease_ip {
                candidates.push(ip);
            }
            if !machine.address.is_empty() && !candidates.contains(&machine.address) {
                candidates.push(machine.address.clone());
            }
            let yaml = load_config_yaml(pool, &jwt_secret, &machine, &mut payload).await?;
            let mut applied = false;
            let mut last_err = String::new();
            for cand in &candidates {
                match ctrl.install_machine(machine_id, &yaml, Some(cand)).await {
                    Ok(()) => {
                        applied = true;
                        log(&mut payload, &format!("install applied at {cand} (reboot)"));
                        break;
                    }
                    Err(e) => {
                        last_err = format!("{cand}: {e}");
                    }
                }
            }
            if applied {
                payload.step = "wait_post_install".into();
                save_progress(pool, job.id, "bootstrapping", &payload).await?;
            } else {
                fail_job(pool, job.id, &mut payload, &format!("install: {last_err}")).await?;
            }
        }
        "wait_post_install" => {
            // Post-install the node is on its static (10G) address; the DHCP
            // lease only applied to the PXE installer. Try the static address
            // first, then fall back to the DHCP lease.
            let mut candidates: Vec<String> = Vec::new();
            if !machine.address.is_empty() {
                candidates.push(machine.address.clone());
            }
            let lease_ip = if !machine.mac_address.is_empty() {
                repos::dhcp_lease::get_by_mac(pool, &machine.mac_address)
                    .await
                    .ok()
                    .flatten()
                    .map(|l| l.ip)
                    .filter(|ip| !ip.is_empty())
            } else {
                None
            };
            if let Some(ip) = lease_ip {
                if !candidates.contains(&ip) {
                    candidates.push(ip);
                }
            }
            let ctrl = ClusterController::with_context(
                pool.clone(),
                sqlite_path.to_string(),
                jwt_secret.to_string(),
            );
            let mut ok = false;
            let mut version = String::new();
            for cand in &candidates {
                match ctrl.machine_version_with_endpoint(machine_id, Some(cand)).await {
                    Ok(v) => {
                        ok = true;
                        version = v;
                        break;
                    }
                    Err(_) => continue,
                }
            }
            if ok {
                log(&mut payload, &format!("post-install version {version}"));
                if payload.auto_bootstrap {
                    let t = machine.machine_type.to_ascii_lowercase();
                    if t.contains("control") {
                        payload.step = "bootstrap".into();
                    } else {
                        payload.step = "boot_disk".into();
                    }
                } else {
                    payload.step = "boot_disk".into();
                }
                save_progress(pool, job.id, "running", &payload).await?;
            } else {
                log(&mut payload, "waiting for node after install reboot");
                save_progress(pool, job.id, "bootstrapping", &payload).await?;
            }
        }
        "bootstrap" => {
            let ctrl = ClusterController::with_context(
                pool.clone(),
                sqlite_path.to_string(),
                jwt_secret.to_string(),
            );
            match ctrl.bootstrap_machine(machine_id).await {
                Ok(()) => {
                    log(&mut payload, "bootstrap complete");
                    payload.step = "boot_disk".into();
                    save_progress(pool, job.id, "running", &payload).await?;
                }
                Err(e) => {
                    // bootstrap may fail if already done
                    log(&mut payload, &format!("bootstrap note: {e}"));
                    payload.step = "boot_disk".into();
                    save_progress(pool, job.id, "running", &payload).await?;
                }
            }
        }
        "boot_disk" => {
            if machine.has_bmc() {
                if let Ok(sess) = open_bmc(pool, jwt_secret, metal, &machine).await {
                    let _ = sess.set_boot(BootTarget::Disk, false).await;
                    log(&mut payload, "BMC boot restored to disk");
                }
            }
            // next machine or done
            payload.current_machine_index += 1;
            payload.step = "pending".into();
            if payload.current_machine_index >= payload.machine_ids.len() {
                save_progress(pool, job.id, "succeeded", &payload).await?;
                log(&mut payload, "all machines provisioned");
            } else {
                save_progress(pool, job.id, "running", &payload).await?;
            }
        }
        other => {
            warn!(step = other, "unknown metal job step; failing");
            fail_job(pool, job.id, &mut payload, &format!("unknown step {other}")).await?;
        }
    }

    Ok(())
}

async fn open_bmc(
    _pool: &DbPool,
    jwt_secret: &str,
    metal: &MetalConfig,
    machine: &crate::db::models::machine::Machine,
) -> Result<BmcSession, AppError> {
    let enc = machine
        .bmc_password_enc
        .as_ref()
        .ok_or_else(|| AppError::InvalidInput("no BMC password".into()))?;
    let plain = secrets::decrypt(jwt_secret, enc)?;
    let creds = BmcCredentials::from_machine(
        machine,
        &plain,
        metal.bmc.connect_timeout_secs,
        &metal.bmc.ipmi_interface,
    )?;
    BmcSession::connect(&creds).await
}

async fn load_config_yaml(
    pool: &DbPool,
    jwt_secret: &str,
    machine: &crate::db::models::machine::Machine,
    payload: &mut MetalJobPayload,
) -> Result<String, AppError> {
    // An explicit artifact_id (from the API request) always wins. Otherwise we
    // generate ONE shared artifact for the whole job so every machine uses the
    // same PKI / cluster id / bootstrap tokens. The shared id is cached in the
    // payload so it is generated only once.
    let artifact_id = match payload.artifact_id {
        Some(id) => id,
        None => match payload.job_artifact_id {
            Some(id) => id,
            None => {
                let cluster_id = machine.cluster_id.ok_or_else(|| {
                    AppError::InvalidInput(
                        "artifact_id not provided and machine has no cluster_id".into(),
                    )
                })?;
                let cluster = repos::cluster::get(pool, cluster_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("cluster {cluster_id}")))?;

                // Collect machine addresses: the first control-plane address is
                // the cluster endpoint; ALL addresses become cert SANs so each
                // node's API server cert is valid when probed via its own IP.
                let mut cp_addresses: Vec<String> = Vec::new();
                let mut all_addresses: Vec<String> = Vec::new();
                for mid in &payload.machine_ids {
                    if let Ok(Some(m)) = repos::machine::get(pool, *mid).await {
                        if m.address.is_empty() {
                            continue;
                        }
                        let ip = m.address.strip_suffix(":6443").unwrap_or(&m.address).to_string();
                        if !all_addresses.contains(&ip) {
                            all_addresses.push(ip.clone());
                        }
                        let is_cp = m.machine_type.to_ascii_lowercase().contains("control");
                        if is_cp && !cp_addresses.contains(&ip) {
                            cp_addresses.push(ip);
                        }
                    }
                }
                let endpoint = if !cp_addresses.is_empty() {
                    cp_addresses[0].clone()
                } else if !all_addresses.is_empty() {
                    all_addresses[0].clone()
                } else if !machine.address.is_empty() {
                    machine.address.strip_suffix(":6443").unwrap_or(&machine.address).to_string()
                } else {
                    cluster.name.clone()
                };

                // Restore network config from cluster metadata (persisted by generate_config).
                // Hostname is left empty so render_network_yaml emits the
                // __HOSTNAME__ placeholder, patched per-machine below.
                let nc = cluster.network_config.as_ref().and_then(|j| {
                    serde_json::from_str::<crate::controllers::provision::NetworkConfigParams>(j).ok()
                });

                let install_disk = if machine.install_disk.is_empty() {
                    "/dev/sda"
                } else {
                    &machine.install_disk
                };
                let prov_ctrl = ProvisionController::new(pool.clone(), jwt_secret.to_string());
                let art = prov_ctrl
                    .generate_config(
                        &cluster.name,
                        &format!("https://{}:6443", endpoint),
                        &cluster.talos_version,
                        &cluster.control_plane_version,
                        Some(cluster_id),
                        nc,
                        install_disk,
                        true,
                        &all_addresses,
                        &cp_addresses,
                        "cluster.local",
                        "", // Siderolink block baked in the wizard path; metal path omits for now
                    )
                    .await?;

                info!(
                    artifact_id = %art.id,
                    cluster_id = %cluster_id,
                    endpoint = %endpoint,
                    cp_count = cp_addresses.len(),
                    sans_count = all_addresses.len(),
                    "Generated shared Talos config artifact for metal provisioning job"
                );
                payload.job_artifact_id = Some(art.id);
                art.id
            }
        },
    };

    let art = repos::provision::get(pool, artifact_id)
        .await?
        .ok_or_else(|| AppError::NotFound("provision artifact".into()))?;
    let is_cp = {
        let t = machine.machine_type.to_ascii_lowercase();
        t.contains("control")
    };
    let mut yaml = if is_cp {
        art.controlplane_config
    } else {
        art.worker_config
    }
    .ok_or_else(|| AppError::InvalidInput("artifact missing config yaml".into()))?;

    // Patch per-machine IP and hostname placeholders.
    if !machine.address.is_empty() {
        let ip = machine.address.strip_suffix(":6443").unwrap_or(&machine.address);
        yaml = yaml.replace("__IP__", ip);
    }
    if !machine.hostname.is_empty() {
        yaml = yaml.replace("__HOSTNAME__", &machine.hostname);
    }

    Ok(yaml)
}

async fn save_progress(
    pool: &DbPool,
    id: Uuid,
    status: &str,
    payload: &MetalJobPayload,
) -> Result<(), AppError> {
    let p = serde_json::to_string(payload).unwrap_or_else(|_| "{}".into());
    repos::provision_job::update_status(pool, id, status, None, Some(&p)).await
}

async fn fail_job(
    pool: &DbPool,
    id: Uuid,
    payload: &mut MetalJobPayload,
    err: &str,
) -> Result<(), AppError> {
    payload
        .steps_log
        .push(format!("{} ERROR {err}", Utc::now().to_rfc3339()));
    let p = serde_json::to_string(payload).unwrap_or_else(|_| "{}".into());
    repos::provision_job::update_status(pool, id, "failed", Some(err), Some(&p)).await
}
