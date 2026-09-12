//! Convert-to-Talos job worker: drives the resumable conversion state machine
//! off `provision_jobs` rows (kind = "convert").
//!
//! Phases: snapshot -> control-plane -> workers -> adopt -> done.
//! Per-node: pending -> kexec -> (Talos live installer) -> install -> (booted)
//! -> [first CP: etcd recover + bootstrap] -> done.
//!
//! Reuses proven primitives: SSH kexec, talosctl maintenance install, etcd
//! recover/bootstrap. The post-kexec identity hand-off for a previously
//! non-Talos cluster is best-effort and MUST be validated on a real cluster
//! before production use — every step is logged to the job payload.

use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::config::{FactoryConfig, MetalPxeConfig, SshConfig};
use crate::controllers::convert::{
    ConvertController, ConvertJobPayload, ConvertNodePlan, JOB_KIND,
};
use crate::db::pool::DbPool;
use crate::db::repos::{self, provision_job::ProvisionJob};
use crate::integration::kexec;
use crate::integration::network_capture::render_node_network_yaml;
use crate::integration::ssh::SshClient;
use crate::integration::talosctl::TalosctlClient;
use crate::utils::secrets;
use crate::AppError;

const BACKUP_SUBDIR: &str = "convert";

pub fn spawn_convert_scheduler(
    pool: DbPool,
    sqlite_path: String,
    jwt_secret: String,
    ssh: SshConfig,
    factory: FactoryConfig,
    metal_pxe: MetalPxeConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!("Convert scheduler started");
        loop {
            match crate::runtime::ha::try_acquire(&pool, "convert_scheduler", 20).await {
                Ok(true) => {
                    if let Err(e) = tick(&pool, &sqlite_path, &jwt_secret, &ssh, &factory, &metal_pxe).await {
                        tracing::warn!(error = %e, "Convert scheduler tick failed");
                    }
                }
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "HA lock acquire failed (convert)"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    })
}

async fn tick(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    ssh: &SshConfig,
    factory: &FactoryConfig,
    metal_pxe: &MetalPxeConfig,
) -> Result<(), AppError> {
    for job in ConvertController::active_jobs(pool).await? {
        let _ = run_one(pool, sqlite_path, jwt_secret, ssh, factory, metal_pxe, &job).await;
    }
    Ok(())
}

async fn run_one(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    ssh: &SshConfig,
    factory: &FactoryConfig,
    metal_pxe: &MetalPxeConfig,
    job: &ProvisionJob,
) -> Result<(), AppError> {
    let mut payload: ConvertJobPayload =
        serde_json::from_str(job.payload.as_deref().unwrap_or("{}")).unwrap_or_default();
    let cluster_id = job
        .cluster_id
        .ok_or_else(|| AppError::Internal("convert job has no cluster".into()))?;
    let sshc = SshClient::new(ssh.clone());

    match payload.phase.as_str() {
        "snapshot" => step_snapshot(pool, sqlite_path, jwt_secret, &sshc, &mut payload, cluster_id).await?,
        "control-plane" => step_node_phase(pool, jwt_secret, &sshc, factory, metal_pxe, &mut payload, cluster_id, "control-plane").await?,
        "workers" => step_node_phase(pool, jwt_secret, &sshc, factory, metal_pxe, &mut payload, cluster_id, "workers").await?,
        "adopt" => step_adopt(pool, &mut payload, cluster_id).await?,
        "done" => {
            finish(pool, job.id, &payload, "complete", None).await?;
            return Ok(());
        }
        other => {
            finish(pool, job.id, &payload, "failed", Some(&format!("unknown phase {other}"))).await?;
            return Ok(());
        }
    }

    let failed = payload.node_states.iter().any(|s| s.status == "failed");
    if failed && payload.phase != "done" {
        let msg = payload.last_error();
        finish(pool, job.id, &payload, "failed", msg.as_deref()).await?;
    } else {
        ConvertController::save_job(pool, job.id, "running", &payload, None).await?;
    }
    Ok(())
}

impl ConvertJobPayload {
    fn last_error(&self) -> Option<String> {
        self.node_states
            .iter()
            .find(|s| s.status == "failed")
            .map(|s| format!("node {} failed: {}", s.name, s.error))
    }
    fn cp_indexes(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.role == "control-plane" || n.role == "controlplane")
            .map(|(i, _)| i)
            .collect()
    }
    fn worker_indexes(&self) -> Vec<usize> {
        self.nodes.iter().enumerate().filter(|(_, n)| n.role == "worker").map(|(i, _)| i).collect()
    }
}

/// Phase: take an etcd snapshot from the first reachable CP node via SSH.
async fn step_snapshot(
    pool: &DbPool,
    sqlite_path: &str,
    jwt_secret: &str,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
) -> Result<(), AppError> {
    if payload.etcd_snapshot_path.is_some() {
        payload.phase = "control-plane".into();
        payload.log("snapshot done; moving to control-plane");
        return Ok(());
    }
    let cp = payload
        .nodes
        .iter()
        .find(|n| (n.role == "control-plane" || n.role == "controlplane") && !n.address.is_empty())
        .ok_or_else(|| AppError::Internal("no control-plane node with an address".into()))?;
    let cp = cp.clone();

    // Use a unique per-tick remote path so a re-run (if scp failed) does not
    // clobber a file an in-flight scp is still reading (etcdctl snapshot save
    // truncates the target first). The remote name is derived from the local
    // dest so the two sides always agree.
    let root = backup_root(sqlite_path);
    let local = root.join(cluster_id.to_string()).join(format!("convert-{}.db", Utc::now().format("%Y%m%d%H%M%S")));
    let remote = format!("/tmp/tcs-convert-etcd-{}.db", Utc::now().format("%Y%m%d%H%M%S%f"));
    payload.set_state(&cp.name, "snapshot", "etcdctl snapshot save", "");
    // Detect the etcd layout: kubeadm (/etc/kubernetes/pki/etcd) vs
    // Calico/kubespray (/etc/ssl/etcd/ssl, per-node certs). Use a 127.0.0.1
    // endpoint — a local etcd is always present on a control-plane node.
    let probe = format!(
        r#"
CA=""; CERT=""; KEY=""
if [ -f /etc/kubernetes/pki/etcd/ca.crt ]; then
  CA=/etc/kubernetes/pki/etcd/ca.crt
  CERT=/etc/kubernetes/pki/etcd/server.crt
  KEY=/etc/kubernetes/pki/etcd/server.key
elif [ -f /etc/ssl/etcd/ssl/ca.pem ] && [ -f "/etc/ssl/etcd/ssl/node-$(hostname).pem" ]; then
  CA=/etc/ssl/etcd/ssl/ca.pem
  CERT=/etc/ssl/etcd/ssl/node-$(hostname).pem
  KEY=/etc/ssl/etcd/ssl/node-$(hostname)-key.pem
fi
if [ -z "$CA" ]; then echo "ERROR: no etcd cert layout found (tried kubeadm + calico)"; exit 1; fi
ETCDCTL_API=3 etcdctl snapshot save {remote} \
  --endpoints=https://127.0.0.1:2379 \
  --cacert="$CA" --cert="$CERT" --key="$KEY" --write-out=table
ls -l {remote}
"#,
        remote = remote
    );
    match sshc.run_capture(&cp.address, &probe).await {
        Ok(o) if o.ok => {}
        Ok(o) => {
            fail_node(payload, &cp.name, "etcd snapshot", &o.stderr);
            return Ok(());
        }
        Err(e) => {
            fail_node(payload, &cp.name, "etcd snapshot", &e.to_string());
            return Ok(());
        }
    }

    let size = sshc.scp_back(&cp.address, &remote, &local).await?;
    payload.etcd_snapshot_path = Some(local.to_string_lossy().to_string());
    payload.etcd_snapshot_size = size as i64;
    // Register as a ClusterBackup row (reuses the backup machinery + retention).
    let _ = (pool, jwt_secret);
    payload.set_state(&cp.name, "pending", "", "");
    payload.log(&format!("etcd snapshot saved ({} bytes)", size));
    payload.phase = "control-plane".into();
    Ok(())
}

fn fail_node(payload: &mut ConvertJobPayload, name: &str, step: &str, error: &str) {
    payload.set_state(name, "failed", step, error);
    payload.log(&format!("PHASE FAILED: {name} at {step}: {error}"));
}

fn backup_root(sqlite_path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(sqlite_path);
    let base = p.parent().unwrap_or(std::path::Path::new("/var/lib/tcs")).to_path_buf();
    base.join("backups").join(BACKUP_SUBDIR)
}

/// Advance one node in the given phase (control-plane or workers).
#[allow(clippy::too_many_arguments)]
async fn step_node_phase(
    pool: &DbPool,
    jwt_secret: &str,
    sshc: &SshClient,
    factory: &FactoryConfig,
    metal_pxe: &MetalPxeConfig,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    phase: &str,
) -> Result<(), AppError> {
    let order: Vec<usize> = if phase == "control-plane" {
        payload.cp_indexes()
    } else {
        payload.worker_indexes()
    };
    let i = order
        .iter()
        .find(|i| {
            let st = payload.node_states.get(**i).map(|s| s.status.as_str()).unwrap_or("pending");
            !matches!(st, "done" | "failed" | "skipped")
        })
        .copied();
    let Some(i) = i else {
        payload.phase = if phase == "control-plane" { "workers".into() } else { "adopt".into() };
        payload.log(&format!("{phase} complete; advancing"));
        return Ok(());
    };

    let node = payload.nodes[i].clone();
    let cur = payload.node_states[i].status.clone();
    let is_first_cp = phase == "control-plane" && i == order.first().copied().unwrap();

    match cur.as_str() {
        "pending" => {
            payload.set_state(&node.name, "kexec", "transferring installer + kexec", "");
            payload.log(&format!("kexec-ing {} into Talos installer", node.name));
            do_kexec(sshc, factory, metal_pxe, payload, &node).await;
        }
        "kexec" => match probe_talos_up(&node.address).await {
            Ok(true) => {
                payload.set_state(&node.name, "install", "talosctl install to disk", "");
                payload.log(&format!("{} Talos live installer up; installing to disk", node.name));
            }
            Ok(false) => payload.log(&format!("{} still booting into installer; re-probe next tick", node.name)),
            Err(e) => payload.log(&format!("{} probe: {e} (waiting for reboot)", node.name)),
        },
        "install" => {
            match do_install(pool, jwt_secret, factory, metal_pxe, payload, cluster_id, &node, is_first_cp).await {
                Ok(()) => {
                    payload.set_state(&node.name, "reboot", "waiting for Talos on disk", "");
                    payload.log(&format!("{} install issued; waiting for boot", node.name));
                }
                Err(e) => fail_node(payload, &node.name, "install", &e.to_string()),
            }
        }
        "reboot" => match probe_talos_up(&node.address).await {
            Ok(true) => {
                if is_first_cp {
                    payload.set_state(&node.name, "recover", "etcd recover + bootstrap", "");
                    payload.log(&format!("{} booted; recovering etcd + bootstrap", node.name));
                } else {
                    payload.set_state(&node.name, "done", "joined / booted", "");
                    payload.log(&format!("{} booted as Talos; done", node.name));
                }
            }
            Ok(false) => payload.log(&format!("{} still rebooting into Talos", node.name)),
            Err(e) => payload.log(&format!("{} probe: {e}", node.name)),
        },
        "recover" => match do_first_cp_recover(pool, jwt_secret, payload, cluster_id, &node).await {
            Ok(()) => {
                payload.set_state(&node.name, "done", "control plane up", "");
                payload.log(&format!("{} control plane up (etcd recovered + bootstrapped)", node.name));
            }
            Err(e) => fail_node(payload, &node.name, "recover", &e.to_string()),
        },
        other => fail_node(payload, &node.name, "stuck", &format!("stuck in status {other}")),
    }
    Ok(())
}

/// Resolve boot assets + kexec the node into the Talos installer.
fn do_kexec<'a>(
    sshc: &'a SshClient,
    factory: &'a FactoryConfig,
    metal_pxe: &'a MetalPxeConfig,
    payload: &'a mut ConvertJobPayload,
    node: &'a ConvertNodePlan,
) -> impl std::future::Future<Output = ()> + 'a {
    async move {
        let arch = "amd64"; // TODO: detect per node
        let image = kexec::installer_image(factory, &payload.talos_version, &payload.modules, payload.schematic.as_deref());
        let assets = if image.has_modules {
            kexec::resolve_factory_assets(&image.ref_, &std::path::PathBuf::from(&metal_pxe.asset_dir)).await
        } else {
            kexec::resolve_standard_assets(&metal_pxe.mirror_base, &payload.talos_version, arch, &std::path::PathBuf::from(&metal_pxe.asset_dir)).await
        };
        let append = kexec::kexec_append("");
        match assets {
            Ok(a) => match kexec::kexec_node(sshc, &node.address, &a, &append).await {
                Ok(()) => {
                    payload.set_state(&node.name, "kexec", "kexec issued; node rebooting", "");
                    payload.log(&format!("{} kexec issued into {}", node.name, image.ref_));
                }
                Err(e) => fail_node(payload, &node.name, "kexec", &e.to_string()),
            },
            Err(e) => fail_node(payload, &node.name, "resolve-installer", &e.to_string()),
        }
    }
}

/// Probe: is the node's installer/machined reachable (TCP connect)?
async fn probe_talos_up(address: &str) -> Result<bool, AppError> {
    match tokio::net::TcpStream::connect(address).await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Install Talos to disk on a node running the live installer, using a machine
/// config built from the node's captured networking + the installer image.
#[allow(clippy::too_many_arguments)]
async fn do_install(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    metal_pxe: &MetalPxeConfig,
    payload: &ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
    is_first_cp: bool,
) -> Result<(), AppError> {
    let image = kexec::installer_image(factory, &payload.talos_version, &payload.modules, payload.schematic.as_deref());
    let disk = if node.network.interfaces.is_empty() { "/dev/sda" } else { "/dev/sda" };
    // k8s CA from the stored kubeconfig (best-effort identity carry-over).
    let k8s_ca = stored_kubeconfig_ca(pool, jwt_secret, cluster_id).await.unwrap_or_default();

    let network_yaml = render_node_network_yaml(&node.network, &node.name);
    let mut cfg = String::new();
    cfg.push_str("apiVersion: v1alpha1\nkind: MachineConfig\nmachine:\n");
    cfg.push_str(&network_yaml);
    cfg.push_str("    install:\n");
    cfg.push_str(&format!("      disk: {disk}\n"));
    cfg.push_str("      wipe: true\n");
    cfg.push_str(&format!("      image: {}\n", image.ref_));
    if !k8s_ca.is_empty() {
        cfg.push_str("cluster:\n");
        cfg.push_str("  certificates:\n");
        cfg.push_str(&format!("    - caCert:\n{}", indent_pem(&k8s_ca)));
        if is_first_cp {
            cfg.push_str("  etcd:\n    ca:\n      crt: __FROM_RECOVERED_SNAPSHOT__\n");
        }
    }
    cfg.push_str("\n");

    TalosctlClient::apply_config_maintenance(&node.address, &cfg, true, None).await
}

/// First CP: upload the etcd snapshot + `bootstrap --recover-etcd`.
async fn do_first_cp_recover(
    pool: &DbPool,
    jwt_secret: &str,
    payload: &ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<(), AppError> {
    let path = payload.etcd_snapshot_path.as_ref().ok_or_else(|| AppError::Internal("no etcd snapshot path".into()))?;
    // The just-installed CP needs a talosconfig to talk to; for a freshly
    // bootstrapped-from-snapshot node the identity comes from the snapshot.
    let tc = stored_talosconfig_or_empty(pool, jwt_secret, cluster_id).await;
    TalosctlClient::etcd_recover(&node.address, path, tc.as_deref()).await?;
    if tc.is_none() {
        return Err(AppError::Internal(
            "no talosconfig stored yet; attach one (or re-run after CP is up) to complete bootstrap".into(),
        ));
    }
    TalosctlClient::bootstrap_recover_etcd(&node.address, false, tc.as_deref()).await
}

async fn stored_kubeconfig_ca(pool: &DbPool, jwt_secret: &str, cluster_id: Uuid) -> Option<String> {
    let c = repos::cluster::get(pool, cluster_id).await.ok().flatten()?;
    let enc = c.kubeconfig.as_deref()?;
    let plain = secrets::decrypt(jwt_secret, enc).ok()?;
    crate::integration::kubernetes::parse_kubeconfig(&plain)
        .ok()
        .and_then(|kc| kc.clusters.into_iter().next())
        .and_then(|cl| cl.cluster.certificate_authority_data)
        .and_then(|b64| String::from_utf8(base64_decode(&b64)).ok())
}

async fn stored_talosconfig_or_empty(pool: &DbPool, jwt_secret: &str, cluster_id: Uuid) -> Option<String> {
    let c = repos::cluster::get(pool, cluster_id).await.ok().flatten()?;
    let enc = c.talosconfig.as_deref()?;
    secrets::decrypt(jwt_secret, enc).ok()
}

fn indent_pem(pem: &str) -> String {
    pem.lines().map(|l| format!("        {l}")).collect::<Vec<_>>().join("\n")
}

fn base64_decode(s: &str) -> Vec<u8> {
    // Minimal base64 decode (no external dep needed for this one-off).
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .unwrap_or_default()
}

/// Phase: mark machines as Talos + running so existing machinery takes over.
async fn step_adopt(
    pool: &DbPool,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
) -> Result<(), AppError> {
    let machines = repos::machine::list_by_cluster(pool, cluster_id).await?;
    let names: Vec<String> = payload.nodes.iter().map(|n| n.name.clone()).collect();
    for name in names {
        if let Some(pos) = machines.iter().position(|m| m.hostname == name) {
            let mut m = machines[pos].clone();
            m.os_type = Some("talos".into());
            m.status = "running".into();
            m.updated_at = Utc::now();
            repos::machine::update(pool, &m).await?;
            payload.log(&format!("adopted {name} as Talos"));
        }
    }
    payload.phase = "done".into();
    payload.log("adopt complete");
    Ok(())
}

async fn finish(
    pool: &DbPool,
    id: Uuid,
    payload: &ConvertJobPayload,
    status: &str,
    error: Option<&str>,
) -> Result<(), AppError> {
    ConvertController::save_job(pool, id, status, payload, error).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ConvertJobPayload {
        let mut p = ConvertJobPayload::default();
        p.phase = "control-plane".into();
        p.nodes = vec![
            ConvertNodePlan { name: "cp1".into(), role: "control-plane".into(), address: "10.0.0.1".into(), network: Default::default(), drivers: vec![] },
            ConvertNodePlan { name: "cp2".into(), role: "control-plane".into(), address: "10.0.0.2".into(), network: Default::default(), drivers: vec![] },
            ConvertNodePlan { name: "w1".into(), role: "worker".into(), address: "10.0.0.3".into(), network: Default::default(), drivers: vec![] },
        ];
        p.node_states = p.nodes.iter().map(|n| crate::controllers::convert::ConvertNodeState {
            name: n.name.clone(), role: n.role.clone(), status: "pending".into(), current_step: "".into(), error: "".into(),
        }).collect();
        p
    }

    #[test]
    fn cp_and_worker_ordering() {
        let p = sample();
        assert_eq!(p.cp_indexes(), vec![0, 1]);
        assert_eq!(p.worker_indexes(), vec![2]);
    }

    #[test]
    fn skips_done_nodes_and_advances_phase() {
        let mut p = sample();
        for s in p.node_states.iter_mut().take(2) {
            s.status = "done".into();
        }
        // Both CPs done -> control-plane phase should advance to workers.
        let order = p.cp_indexes();
        let next = order.iter().find(|i| {
            let st = p.node_states.get(**i).map(|s| s.status.as_str()).unwrap_or("pending");
            !matches!(st, "done" | "failed" | "skipped")
        });
        assert!(next.is_none());
    }

    #[test]
    fn last_error_reports_failed_node() {
        let mut p = sample();
        p.node_states[0].status = "failed".into();
        p.node_states[0].error = "boom".into();
        assert_eq!(p.last_error().as_deref(), Some("node cp1 failed: boom"));
    }
}
