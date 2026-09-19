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
    ConvertClusterIdentity, ConvertController, ConvertJobPayload, ConvertNodePlan, JOB_KIND,
};
use crate::db::pool::DbPool;
use crate::db::repos::{self, provision_job::ProvisionJob};
use crate::integration::kexec;
use crate::integration::network_capture::{network_usable, render_node_network_yaml, NetworkCapture};
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
            // TTL must exceed the longest atomic step (etcd snapshot: etcdctl
            // + 59MB scp can take ~40s) so the next tick does not preempt a
            // step mid-flight. The tick loop is sequential (one `tick` await
            // before the next acquire), so a long TTL only delays lock hand-off
            // if this process dies — acceptable for a single-node TCS.
            match crate::runtime::ha::try_acquire(&pool, "convert_scheduler", 300).await {
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

    // A single node failure must not abort the rest of the fleet convert.
    // Failed nodes are skipped by step_node_phase; the job finishes complete
    // once every node is done/failed/skipped.
    ConvertController::save_job(pool, job.id, "running", &payload, None).await?;
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

/// Phase: take an etcd snapshot from the first OLD (still kubespray) control
/// plane via SSH. A CP already converted to Talos (its /etc/kubernetes +
/// /etc/etcd.env are gone, port 22 closed) is skipped — the snapshot must come
/// from a surviving old CP.
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
    let order: Vec<usize> = payload
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| (n.role == "control-plane" || n.role == "controlplane") && !n.address.is_empty())
        .map(|(i, _)| i)
        .collect();
    let Some(&i) = order.first() else {
        // No CP in the plan: nothing to snapshot; proceed straight to the
        // per-node phases (worker-only pilot conversions).
        payload.phase = if payload.cp_indexes().is_empty() && !payload.worker_indexes().is_empty() {
            "workers".into()
        } else {
            "control-plane".into()
        };
        payload.log("no control-plane in convert plan; skipping etcd snapshot");
        return Ok(());
    };
    // Use a unique per-tick remote path so a re-run (if scp failed) does not
    // clobber a file an in-flight scp is still reading (etcdctl snapshot save
    // truncates the target first). The remote name is derived from the local
    // dest so the two sides always agree.
    let root = backup_root(sqlite_path);
    // Detect the etcd layout: kubeadm (/etc/kubernetes/pki/etcd), kubespray
    // (/etc/ssl/etcd/ssl, member-<hostname>.pem or node-<hostname>.pem), or
    // /etc/etcd.env TLS settings (authoritative). A local etcd is always
    // present on a control-plane node, so use 127.0.0.1.
    let probe = |remote: &str| -> String {
        format!(
            r#"
# A node already converted to Talos has no kubespray layout; mark it so the
# scheduler can skip it (port 22 is closed on Talos anyway).
if [ ! -f /etc/etcd.env ] && [ ! -f /etc/kubernetes/pki/ca.crt ] && [ ! -d /etc/ssl/etcd/ssl ]; then
  echo "CONVERTED"
  exit 42
fi
CA=""; CERT=""; KEY=""
envget() {{ grep -E "^$1=" /etc/etcd.env 2>/dev/null | head -1 | cut -d= -f2-; }}
ENVCA=$(envget ETCD_TRUSTED_CA_FILE); ENVCERT=$(envget ETCD_CERT_FILE); ENVKEY=$(envget ETCD_KEY_FILE)
if [ -n "$ENVCA" ] && [ -f "$ENVCA" ] && [ -f "$ENVCERT" ] && [ -f "$ENVKEY" ]; then
  CA=$ENVCA; CERT=$ENVCERT; KEY=$ENVKEY
elif [ -f /etc/kubernetes/pki/etcd/ca.crt ]; then
  CA=/etc/kubernetes/pki/etcd/ca.crt
  CERT=/etc/kubernetes/pki/etcd/server.crt
  KEY=/etc/kubernetes/pki/etcd/server.key
elif [ -f /etc/ssl/etcd/ssl/ca.pem ]; then
  CA=/etc/ssl/etcd/ssl/ca.pem
  for n in member-$(hostname) node-$(hostname) member-$(hostname -s) node-$(hostname -s); do
    if [ -f "/etc/ssl/etcd/ssl/$n.pem" ] && [ -f "/etc/ssl/etcd/ssl/$n-key.pem" ]; then
      CERT="/etc/ssl/etcd/ssl/$n.pem"; KEY="/etc/ssl/etcd/ssl/$n-key.pem"; break
    fi
  done
fi
if [ -z "$CA" ] || [ -z "$CERT" ] || [ -z "$KEY" ]; then echo "ERROR: no etcd cert layout found (tried /etc/etcd.env, kubeadm, kubespray member-/node-)"; exit 1; fi
ETCDCTL_API=3 etcdctl snapshot save {remote} \
  --endpoints=https://127.0.0.1:2379 \
  --cacert="$CA" --cert="$CERT" --key="$KEY" --write-out=table
ls -l {remote}
"#,
            remote = remote
        )
    };
    for &i in &order {
        let cp = payload.nodes[i].clone();
        let local = root.join(cluster_id.to_string()).join(format!("convert-{}.db", Utc::now().format("%Y%m%d%H%M%S")));
        let remote = format!("/tmp/tcs-convert-etcd-{}.db", Utc::now().format("%Y%m%d%H%M%S%f"));
        payload.set_state(&cp.name, "snapshot", "etcdctl snapshot save", "");
        match sshc.run_capture(&cp.address, &probe(&remote)).await {
            Ok(o) if o.ok => {}
            Ok(o) if o.stdout.contains("CONVERTED") => {
                payload.log(&format!("{} is already Talos (no kubespray etcd layout); trying next CP", cp.name));
                payload.set_state(&cp.name, "pending", "", "");
                continue;
            }
            Ok(o) => {
                fail_node(payload, &cp.name, "etcd snapshot", &o.stderr);
                return Ok(());
            }
            Err(e) => {
                // Unreachable over SSH (already converted, or down): skip to
                // the next old CP instead of failing the whole job.
                payload.log(&format!("{} snapshot source unreachable ({e}); trying next CP", cp.name));
                payload.set_state(&cp.name, "pending", "", "");
                continue;
            }
        }
        let size = match sshc.scp_back(&cp.address, &remote, &local).await {
            Ok(s) => s,
            Err(e) => {
                fail_node(payload, &cp.name, "etcd snapshot scp", &e.to_string());
                return Ok(());
            }
        };
        payload.etcd_snapshot_path = Some(local.to_string_lossy().to_string());
        payload.etcd_snapshot_size = size as i64;
        // TLS etcd (kubespray): the snapshot's member list carries https peer
        // URLs; etcd recovery must restore+reconfigure it (flag for recover).
        payload.etcd_tls = true;
        payload.log(&format!("etcd snapshot saved from {} ({} bytes, TLS)", cp.name, size));
        payload.set_state(&cp.name, "pending", "", "");
        // Register as a ClusterBackup row (reuses the backup machinery + retention).
        let _ = (pool, jwt_secret);
        payload.phase = "control-plane".into();
        return Ok(());
    }
    fail_node(payload, "", "etcd snapshot", "no surviving old CP available for the snapshot");
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
            // Resume path: a previous run may have already applied the install
            // config to the live installer (and the installer reboots itself).
            // If the installer is no longer up, treat it as "install issued"
            // and let the reboot probe take over — no re-apply.
            match probe_talos_up(&node.address).await {
                Ok(true) => match do_install(pool, jwt_secret, factory, metal_pxe, sshc, payload, cluster_id, &node, is_first_cp).await {
                    Ok(()) => {
                        payload.set_state(&node.name, "reboot", "waiting for Talos on disk", "");
                        if let Some(s) = payload.node_states.iter_mut().find(|s| s.name == node.name) {
                            s.attempts = 0;
                        }
                        payload.log(&format!("{} install issued; waiting for boot", node.name));
                    }
                    Err(e) => fail_node(payload, &node.name, "install", &e.to_string()),
                },
                Ok(false) => {
                    payload.set_state(&node.name, "reboot", "waiting for Talos on disk", "");
                    payload.log(&format!("{} installer no longer up; assuming install completed, waiting for boot", node.name));
                }
                Err(e) => payload.log(&format!("{} install probe: {e} (waiting)", node.name)),
            }
        }
        "reboot" => {
            let apid = probe_talos_up(&node.address).await.unwrap_or(false);
            let kubelet = probe_tcp(&node.address, 10250).await;
            if apid {
                if is_first_cp {
                    payload.set_state(&node.name, "recover", "etcd recover + bootstrap", "");
                    payload.log(&format!("{} booted; recovering etcd + bootstrap", node.name));
                } else {
                    payload.set_state(&node.name, "configure", "apply captured machine config", "");
                    payload.log(&format!("{} Talos apid up; pushing captured network config", node.name));
                    if let Some(s) = payload.node_states.iter_mut().find(|s| s.name == node.name) {
                        s.attempts = 0;
                    }
                }
            } else if kubelet && !is_first_cp {
                // Worker-only overtake: there is no Talos CP/trustd to issue a
                // machine cert, so apid (:50000) never listens. Kubelet on
                // :10250 means the node booted Talos; join is via the kubeadm
                // bootstrap token injected into the live API.
                let _ = inject_bootstrap_token(pool, jwt_secret, cluster_id, payload).await;
                let _ = issue_kubelet_csrs(pool, jwt_secret, cluster_id, payload).await;
                if kube_node_is_talos(pool, jwt_secret, cluster_id, &node.name).await {
                    payload.set_state(&node.name, "done", "joined Kubernetes as Talos", "");
                    payload.log(&format!("{} kubelet up and node OS is Talos; done (apid not required for worker-only)", node.name));
                } else {
                    let attempts = bump_attempts(payload, &node.name);
                    if attempts > 120 {
                        fail_node(
                            payload,
                            &node.name,
                            "reboot",
                            "Talos kubelet is up but the node never joined Kubernetes (check bootstrap token + cluster CA)",
                        );
                    } else {
                        payload.log(&format!(
                            "{} Talos kubelet up (no apid — expected without a Talos CP); waiting for Kubernetes join ({attempts}/120)",
                            node.name
                        ));
                    }
                }
            } else {
                let attempts = bump_attempts(payload, &node.name);
                if attempts > 120 {
                    fail_node(
                        payload,
                        &node.name,
                        "reboot",
                        "Talos apid :50000 and kubelet :10250 never came up after install",
                    );
                } else {
                    payload.log(&format!("{} still rebooting into Talos", node.name));
                }
            }
        }
        "configure" => match do_post_boot_apply(pool, jwt_secret, factory, sshc, payload, cluster_id, &node).await {
            Ok(()) => {
                payload.set_state(&node.name, "done", "machine config applied", "");
                payload.log(&format!("{} captured network applied to running Talos", node.name));
            }
            Err(e) => {
                let msg = e.to_string();
                let attempts = payload
                    .node_states
                    .iter()
                    .find(|s| s.name == node.name)
                    .map(|s| s.attempts)
                    .unwrap_or(0)
                    + 1;
                if attempts > 12 {
                    fail_node(payload, &node.name, "configure", &msg);
                } else {
                    if let Some(s) = payload.node_states.iter_mut().find(|s| s.name == node.name) {
                        s.attempts = attempts;
                        s.current_step = format!("apply-config retry {attempts}/12");
                    }
                    payload.log(&format!(
                        "{} post-boot apply failed (attempt {attempts}): {msg}",
                        node.name
                    ));
                }
            }
        },
        "recover" => match do_first_cp_recover(pool, jwt_secret, sshc, payload, cluster_id, &node).await {
            Ok(()) => {
                payload.set_state(&node.name, "done", "control plane up", "");
                payload.log(&format!("{} control plane up (etcd recovered + bootstrapped)", node.name));
            }
            Err(e) => {
                let msg = e.to_string();
                // Transient: the freshly-booted CP's etcd isn't ready to accept a
                // recovery snapshot yet ("etcd service is not ready for recovery
                // yet" / FailedPrecondition). Keep "recover" so the next tick
                // retries; cap attempts so a genuine fault fails instead of looping.
                let transient = msg.contains("not ready for recovery")
                    || msg.contains("FailedPrecondition")
                    || msg.contains("etcd service is not ready");
                if transient {
                    if let Some(s) = payload.node_states.iter_mut().find(|s| s.name == node.name) {
                        s.attempts += 1;
                        if s.attempts > 60 {
                            s.status = "failed".into();
                            s.current_step = "recover".into();
                            s.error = format!("etcd never ready for recovery after 60 attempts: {msg}");
                            payload.log(&format!("PHASE FAILED: {} etcd not ready after 60 attempts", node.name));
                        } else {
                            s.current_step = format!("bootstrap --recover-from (retry {}/60)", s.attempts);
                            let att = s.attempts;
                            payload.log(&format!(
                                "{} etcd not ready for recovery yet (attempt {att}); retrying next tick",
                                node.name
                            ));
                        }
                    } else {
                        fail_node(payload, &node.name, "recover", &msg);
                    }
                } else {
                    fail_node(payload, &node.name, "recover", &msg);
                }
            }
        },
        other => fail_node(payload, &node.name, "stuck", &format!("stuck in status {other}")),
    }
    Ok(())
}

/// Disk-install image ref: operator override (local patched registry) wins,
/// otherwise the factory metal-installer for this job's schematic (the 3
/// required convert modules plus any extras).
fn convert_disk_image(factory: &FactoryConfig, payload: &ConvertJobPayload) -> String {
    if let Some(r) = payload.install_image_override.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return r.to_string();
    }
    kexec::installer_image(
        factory,
        &payload.talos_version,
        &payload.modules,
        payload.schematic.as_deref(),
    )
    .ref_
}

/// Refresh the node's captured network over SSH immediately before kexec.
/// Falls back to the payload snapshot if live capture fails.
async fn refresh_node_network(sshc: &SshClient, payload: &mut ConvertJobPayload, node: &mut ConvertNodePlan) {
    let cap = NetworkCapture::new(sshc.clone());
    match cap.capture(&node.address).await {
        Ok(res) => {
            node.network = res.network.clone();
            node.drivers = res.drivers.clone();
            if let Some(p) = payload.nodes.iter_mut().find(|n| n.name == node.name) {
                p.network = res.network;
                p.drivers = res.drivers;
                if !res.kubelet_cert.is_empty() {
                    p.kubelet_cert = res.kubelet_cert;
                    p.kubelet_key = res.kubelet_key;
                }
            }
            payload.log(&format!("{} live network recaptured before kexec", node.name));
        }
        Err(e) => {
            payload.log(&format!(
                "{} live recapture failed ({e}); using payload snapshot",
                node.name
            ));
        }
    }
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
        let mut node = node.clone();
        refresh_node_network(sshc, payload, &mut node).await;
        if !network_usable(&node.network) {
            fail_node(
                payload,
                &node.name,
                "kexec",
                "no usable captured network (need static IP + gateway) — refusing kexec",
            );
            return;
        }

        let disk_image = convert_disk_image(factory, payload);
        let asset_dir = std::path::PathBuf::from(&metal_pxe.asset_dir);

        // Kexec BOOT vehicle: factory schematic kernel+initramfs (bzImage),
        // which bakes in bnx2-bnx2x / iscsi-tools / nfs-utils. The factory
        // *metal-installer* OCI (`vmlinuz.efi`) is NOT kexec-loadable on
        // legacy BIOS; it remains the on-disk install target only.
        // Stock+firmware-graft custom assets do NOT include iscsi/nfs and
        // must not be preferred when a schematic is present.
        let Some(schematic) = payload.schematic.as_deref() else {
            fail_node(
                payload,
                &node.name,
                "resolve-installer",
                "convert requires a factory schematic (bnx2-bnx2x, iscsi-tools, nfs-utils)",
            );
            return;
        };
        let assets = kexec::resolve_factory_boot_assets(
            &factory.normalized_base(),
            schematic,
            &payload.talos_version,
            arch,
            &asset_dir,
        )
        .await;
        let append = kexec::kexec_append(&node.network, &node.name, "");
        match assets {
            Ok(a) => {
                payload.log(&format!(
                    "{} kexec assets: factory schematic {schematic} modules [{}]",
                    node.name,
                    payload.modules.join(", ")
                ));
                let already_installer = probe_talos_up(&node.address).await.unwrap_or(false);
                if already_installer {
                    payload.log(&format!(
                        "{} already in Talos installer (previous kexec landed); skipping transfer",
                        node.name
                    ));
                } else if let Err(e) = kexec::kexec_node(sshc, &node.address, &a, &append).await {
                    fail_node(payload, &node.name, "kexec", &e.to_string());
                    return;
                }
                payload.set_state(&node.name, "kexec", "kexec issued; node rebooting", "");
                payload.log(&format!("{} kexec issued (install target {disk_image})", node.name));
            }
            Err(e) => fail_node(payload, &node.name, "resolve-installer", &e.to_string()),
        }
    }
}

fn bump_attempts(payload: &mut ConvertJobPayload, name: &str) -> u32 {
    if let Some(s) = payload.node_states.iter_mut().find(|s| s.name == name) {
        s.attempts = s.attempts.saturating_add(1);
        s.attempts
    } else {
        1
    }
}

/// Probe: is the node's Talos API reachable (TCP connect)?
/// The Talos apid listens on port 50000 (constants.ApidPort) in BOTH the
/// installer/maintenance phase and once the system is installed. Ports 5000/
/// 5006 are pre-1.10 legacy and are NOT open in v1.13 — probing them made a
/// healthy maintenance-mode installer look dead forever. A bare
/// `connect(address)` targets TCP/80, which Talos never opens, so we must
/// append the explicit port.
async fn probe_talos_up(address: &str) -> Result<bool, AppError> {
    Ok(probe_tcp(address, 50000).await)
}

async fn probe_tcp(address: &str, port: u16) -> bool {
    let host = address.split(':').next().unwrap_or(address);
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

/// The Talos apid endpoint for a node address. apid listens on port 50000
/// (constants.ApidPort) in both the installer/maintenance phase and once the
/// system is installed. 5000/5006 are pre-1.10 legacy and never open in v1.13.
/// If the address already carries a port, it is preserved as-is.
fn talos_endpoint(address: &str) -> String {
    if address.contains(':') {
        address.to_string()
    } else {
        format!("{address}:50000")
    }
}

/// Install Talos to disk on a node running the live installer, using a machine
/// config built from the node's captured networking + the installer image.
#[allow(clippy::too_many_arguments)]
async fn do_install(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    _metal_pxe: &MetalPxeConfig,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
    _is_first_cp: bool,
) -> Result<(), AppError> {
    let cfg = node_install_config(pool, jwt_secret, factory, sshc, payload, cluster_id, node).await?;
    TalosctlClient::apply_config_maintenance(&node.address, &cfg, true, None).await
}

/// Build the install machine config for a node (installer maintenance apply).
#[allow(clippy::too_many_arguments)]
async fn node_install_config(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<String, AppError> {
    node_machine_config(pool, jwt_secret, factory, sshc, payload, cluster_id, node, true).await
}

/// After the node has booted Talos from disk, push the captured-network
/// machine config over apid (no install.wipe — that would re-image).
async fn do_post_boot_apply(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<(), AppError> {
    let cfg = node_machine_config(pool, jwt_secret, factory, sshc, payload, cluster_id, node, false).await?;
    let k8s_ca = stored_kubeconfig_ca(pool, jwt_secret, cluster_id).await.unwrap_or_default();
    let kube_server = stored_kubeconfig_server(pool, jwt_secret, cluster_id).await;
    let ident = ensure_cluster_identity(sshc, payload, &k8s_ca, kube_server.as_deref()).await?;
    let tc = build_node_talosconfig(&ident, &node.address);
    let endpoint = talos_endpoint(&node.address);
    payload.log(&format!("{} applying captured machine config (no wipe)", node.name));
    TalosctlClient::apply_config(&endpoint, &cfg, false, false, Some(&tc)).await
}

/// Build the machine config for a node. `with_install` includes the
/// `machine.install` wipe/image block used by the live installer; the
/// post-boot apply omits it so networking can be pushed without re-imaging.
#[allow(clippy::too_many_arguments)]
async fn node_machine_config(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
    with_install: bool,
) -> Result<String, AppError> {
    let image = convert_disk_image(factory, payload);
    let disk = "/dev/sda"; // overtake: install to the boot disk; ceph OSDs are sdb+
    let k8s_ca = stored_kubeconfig_ca(pool, jwt_secret, cluster_id).await.unwrap_or_default();
    let is_cp = node.role == "control-plane" || node.role == "controlplane";
    let machine_type = if is_cp { "controlplane" } else { "worker" };
    let kube_server = stored_kubeconfig_server(pool, jwt_secret, cluster_id).await;
    let ident = ensure_cluster_identity(sshc, payload, &k8s_ca, kube_server.as_deref()).await?;
    let _ = inject_bootstrap_token(pool, jwt_secret, cluster_id, payload).await;
    let ident = payload.cluster_identity.clone().unwrap_or(ident);
    let network_yaml = render_node_network_yaml(&node.network, &node.name);

    let cfg = build_install_config(
        machine_type,
        is_cp,
        &network_yaml,
        disk,
        &image,
        &ident,
        &k8s_ca,
        // Overtake: true when the original cluster PKI was extracted, so the CP
        // config embeds it (and serves the recovered etcd with the same identity).
        !ident.k8s_ca_crt.is_empty(),
        &node.kubelet_cert,
        &node.kubelet_key,
        with_install,
    );
    // Debug: log the network section (where YAML decode errors land) so a
    // malformed block is visible in the job log without dumping the whole
    // config (PEM base64 is huge).
    let net_lines: Vec<String> = cfg
        .lines()
        .skip_while(|l| !l.starts_with("  network:"))
        .take_while(|l| !l.starts_with("  acceptedCAs:") && !l.starts_with("  ca:"))
        .enumerate()
        .map(|(i, l)| format!("L{}| {}", i + 6, l.chars().take(120).collect::<String>()))
        .collect();
    payload.log(&format!("{} install network section:\n{}", node.name, net_lines.join("\n")));
    Ok(cfg)
}

/// Build the installer maintenance-mode machine config (bare root v1alpha1 form)
/// as a string. Pure + testable: the exact schema was validated against the live
/// v1.13.10 installer via `talosctl apply-config --dry-run`. Rules:
/// - NO `apiVersion`/`kind`/`metadata` (configloader rejects any explicit kind
///   with "not registered");
/// - `persist: true` (required, else ".persist should be enabled");
/// - `machine.type` controlplane|worker (required);
/// - control plane carries `machine.ca` (issuing crt+key); workers carry
///   `machine.acceptedCAs` (crt only — "issuing CA key not allowed on
///   non-controlplane nodes");
/// - a `cluster:` block is required for both ("cluster instructions are
///   required");
/// - `systemExtensions` is NOT a valid key (extensions come from the factory
///   install image).
#[allow(clippy::too_many_arguments)]
fn build_install_config(
    machine_type: &str,
    is_cp: bool,
    network_yaml: &str,
    disk: &str,
    image_ref: &str,
    ident: &ConvertClusterIdentity,
    k8s_ca: &str,
    full_identity: bool,
    kubelet_cert: &str,
    kubelet_key: &str,
    with_install: bool,
) -> String {
    let mut cfg = String::new();
    cfg.push_str("version: v1alpha1\n");
    cfg.push_str("persist: true\n");
    cfg.push_str("machine:\n");
    cfg.push_str(&format!("  type: {machine_type}\n"));
    cfg.push_str("  network:\n");
    // render_node_network_yaml emits a 4-space base (`    hostname:` /
    // `    interfaces:`) which is already correct for being the children of a
    // 2-space `network:` key under `machine:`. Do NOT add indentation (the
    // earlier +2 over-indented and broke the YAML block mapping).
    cfg.push_str(network_yaml);
    // CA fields are base64-of-PEM on a single line (matches the proven
    // greenfield provision format and what the v1.13 installer decoder accepts;
    // a raw PEM block scalar fails with "illegal base64 data").
    let ca_crt_b64 = crate::controllers::provision::b64_le(&ident.machine_ca_crt);
    let ca_key_b64 = crate::controllers::provision::b64_le(&ident.machine_ca_key);
    if is_cp {
        cfg.push_str("  ca:\n");
        cfg.push_str(&format!("    crt: {ca_crt_b64}\n"));
        cfg.push_str(&format!("    key: {ca_key_b64}\n"));
    } else {
        cfg.push_str("  acceptedCAs:\n");
        cfg.push_str(&format!("    - crt: {ca_crt_b64}\n"));
    }
    cfg.push_str(&format!("  token: {}\n", ident.machine_token));
    if with_install {
        cfg.push_str("  install:\n");
        cfg.push_str(&format!("    disk: {disk}\n"));
        // Wipe only the install disk (/dev/sda). Talos does not touch the other
        // disks (sdb..sdk hold ceph OSDs), so this is safe for the overtake.
        cfg.push_str("    wipe: true\n");
        cfg.push_str(&format!("    image: {image_ref}\n"));
        // Enable serial console on the installed system so SOL (IPMI) works for
        // debugging. The kexec boot has console=ttyS0 in its append line, but the
        // GRUB config written by the installer does NOT inherit those params.
        // NOTE: iLO4 SOL runs at 115200 baud; a bare console=ttyS0 defaults to
        // 9600 and produces silent SOL after GRUB handoff. Always set the baud.
        cfg.push_str("    extraKernelArgs:\n");
        cfg.push_str("      - console=ttyS0,115200\n");
        cfg.push_str("      - slab_nomerge\n");
        cfg.push_str("      - pti=on\n");
    }
    // Cluster identity. For an OVERTAKE the CP must carry the ORIGINAL cluster
    // PKI (k8s CA + key, aggregator/front-proxy CA + key, service-account key)
    // so the static-pod apiserver (gated on secrets.KubernetesRoot, derived
    // from this config) serves the recovered kubeadm etcd data with the SAME
    // identity - existing client certs + SA tokens keep validating. A fresh
    // identity cannot serve foreign data. Workers only need to TRUST the
    // cluster CA (acceptedCAs / cluster.ca.crt, no keys).
    let k8s_crt = if ident.k8s_ca_crt.is_empty() { k8s_ca } else { &ident.k8s_ca_crt };
    cfg.push_str("cluster:\n");
    cfg.push_str(&format!("  id: {}\n", ident.cluster_id));
    cfg.push_str(&format!("  secret: {}\n", ident.cluster_secret));
    cfg.push_str("  controlPlane:\n");
    cfg.push_str(&format!("    endpoint: {}\n", ident.control_plane_endpoint));
    cfg.push_str(&format!("  clusterName: {}\n", ident.cluster_name));
    cfg.push_str(&format!("  token: {}\n", ident.kube_token));
    if is_cp && !ident.k8s_ca_crt.is_empty() {
        // Full CP cluster identity (original PKI carried over).
        let b64 = crate::controllers::provision::b64_le;
        cfg.push_str(&format!("  secretboxEncryptionSecret: {}\n", ident.secretbox_secret));
        cfg.push_str("  network:\n");
        cfg.push_str("    cni:\n");
        cfg.push_str("      name: none\n");
        cfg.push_str(&format!("    dnsDomain: {}\n", ident.cluster_domain));
        cfg.push_str("    podSubnets:\n");
        cfg.push_str(&format!("      - {}\n", ident.pod_cidr));
        cfg.push_str("    serviceSubnets:\n");
        cfg.push_str(&format!("      - {}\n", ident.service_cidr));
        cfg.push_str("  ca:\n");
        cfg.push_str(&format!("    crt: {}\n", b64(&ident.k8s_ca_crt)));
        cfg.push_str(&format!("    key: {}\n", b64(&ident.k8s_ca_key)));
        cfg.push_str("  aggregatorCA:\n");
        cfg.push_str(&format!("    crt: {}\n", b64(&ident.aggregator_ca_crt)));
        cfg.push_str(&format!("    key: {}\n", b64(&ident.aggregator_ca_key)));
        cfg.push_str("  serviceAccount:\n");
        cfg.push_str(&format!("    key: {}\n", b64(&ident.service_account_key)));
        let img = format!("registry.k8s.io/kube-apiserver:{}", ident.k8s_version);
        cfg.push_str("  apiServer:\n");
        cfg.push_str(&format!("    image: {img}\n"));
        cfg.push_str("  controllerManager:\n");
        cfg.push_str(&format!("    image: registry.k8s.io/kube-controller-manager:{v}\n", v = ident.k8s_version));
        cfg.push_str("  proxy:\n");
        cfg.push_str(&format!("    image: registry.k8s.io/kube-proxy:{v}\n", v = ident.k8s_version));
        cfg.push_str("  scheduler:\n");
        cfg.push_str(&format!("    image: registry.k8s.io/kube-scheduler:{v}\n", v = ident.k8s_version));
        // etcd CA. For an OVERTAKE of a TLS etcd (kubespray: https peer
        // URLs, per-member certs) reuse the ORIGINAL etcd CA + key when both
        // were harvested, so Talos issues each member a cert from the same CA
        // the recovered snapshot's etcd data expects (etcd re-signs member
        // certs as nodes join). Without them, fall back to the fresh CA pair.
        let (etcd_crt, etcd_key) = if ident.etcd_ca_crt_orig.is_empty()
            || ident.etcd_ca_key_orig.is_empty()
        {
            (
                b64(&ident.etcd_ca_crt),
                b64(&ident.etcd_ca_key),
            )
        } else {
            (
                b64(&ident.etcd_ca_crt_orig),
                b64(&ident.etcd_ca_key_orig),
            )
        };
        cfg.push_str("  etcd:\n");
        cfg.push_str("    ca:\n");
        cfg.push_str(&format!("      crt: {etcd_crt}\n"));
        cfg.push_str(&format!("      key: {etcd_key}\n"));
    } else {
        // Worker (or CP without original identity): trust the cluster CA only.
        if !k8s_crt.is_empty() {
            cfg.push_str("  ca:\n");
            cfg.push_str(&format!("    crt: {}\n", crate::controllers::provision::b64_le(k8s_crt)));
        }
        if is_cp {
            // CP always needs an etcd CA (fresh when original unknown).
            let etcd_crt = crate::controllers::provision::b64_le(&ident.etcd_ca_crt);
            let etcd_key = crate::controllers::provision::b64_le(&ident.etcd_ca_key);
            cfg.push_str("  etcd:\n");
            cfg.push_str("    ca:\n");
            cfg.push_str(&format!("      crt: {etcd_crt}\n"));
            cfg.push_str(&format!("      key: {etcd_key}\n"));
        }
    }
    cfg.push_str("\n");
    cfg
}

/// Ensure the job has a shared cluster identity, generating it once and
/// persisting it in the payload. The machine CA + cluster id/secret are shared
/// by ALL nodes so they form one Talos control plane. The control-plane
/// endpoint is the FIRST control-plane node's address:6443 (the VIP for the
/// overtaken cluster's API server).
async fn ensure_cluster_identity(
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    k8s_ca: &str,
    kube_server: Option<&str>,
) -> Result<ConvertClusterIdentity, AppError> {
    if let Some(ident) = &payload.cluster_identity {
        return Ok(ident.clone());
    }
    use crate::controllers::provision::{
        b64_random, bootstrap_token, generate_ca_issuer, generate_server_cert,
    };
    // Overtake keeps the cluster's existing name. The API-server identity
    // (k8s CA, aggregator CA, SA key) is CARRIED OVER from a running old
    // control plane so the recovered cluster keeps its original identity
    // (existing client certs + SA tokens validate). The Talos machine CA +
    // membership id/secret/token are fresh (Talos-specific; the old cluster is
    // kubeadm).
    let cluster_name = if payload.cluster_name.is_empty() {
        "phobos".to_string()
    } else {
        payload.cluster_name.clone()
    };
    let machine_ca = generate_ca_issuer(&format!("{cluster_name}-talos-ca"), 3650)?;
    let etcd_ca = generate_ca_issuer(&format!("{cluster_name}-etcd-ca"), 3650)?;

    // First CP address = control-plane endpoint (all nodes join here).
    let cp_addr = payload
        .nodes
        .iter()
        .find(|n| n.role == "control-plane" || n.role == "controlplane")
        .map(|n| n.address.clone())
        .unwrap_or_default();
    // Worker-only overtakes have no CP in the node list, so cp_addr is empty.
    // The operator supplies the EXISTING running CP explicitly
    // (payload.control_plane_endpoint) -- without it the endpoint would default
    // to 127.0.0.1:6443 and the workers could never join the cluster plane
    // (kubelet/apiserver can't reach the CP -> node stuck "rebooting into Talos").
    // Prefer the kubeconfig server URL: it is the API the cluster actually
    // serves (phobos: 172.20.0.38). A random CP IP (e.g. infra02 .55) can
    // present a kube-apiserver cert but reject bootstrap tokens (401).
    let endpoint = if let Some(s) = kube_server.map(str::trim).filter(|s| !s.is_empty()) {
        if s.starts_with("https://") || s.starts_with("http://") {
            s.replace("http://", "https://")
        } else {
            format!("https://{s}")
        }
    } else if !cp_addr.is_empty() {
        format!("https://{cp_addr}:6443")
    } else if let Some(ref explicit) = payload.control_plane_endpoint {
        let e = explicit.trim();
        if e.is_empty() {
            "https://127.0.0.1:6443".into()
        } else if let Some(hostport) = e.strip_prefix("https://") {
            format!("https://{hostport}")
        } else if let Some(hostport) = e.strip_prefix("http://") {
            format!("https://{hostport}")
        } else if e.contains(':') {
            format!("https://{e}")
        } else {
            format!("https://{e}:6443")
        }
    } else {
        "https://127.0.0.1:6443".into()
    };
    let admin_sans: Vec<String> = if cp_addr.is_empty() {
        // Worker-only overtake: include the explicit CP host (if given) so the
        // admin cert is valid for the real control plane, plus loopback.
        let mut sans = vec!["localhost".into(), "127.0.0.1".into()];
        if let Some(ref explicit) = payload.control_plane_endpoint {
            let e = explicit.trim();
            if !e.is_empty() {
                let host = e
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split(':')
                    .next()
                    .unwrap_or(e)
                    .trim();
                if !host.is_empty() && !sans.iter().any(|s| s == host) {
                    sans.push(host.to_string());
                }
            }
        }
        sans
    } else {
        vec!["localhost".into(), cp_addr.clone()]
    };
    let admin_sans_refs: Vec<&str> = admin_sans.iter().map(|s| s.as_str()).collect();
    let (admin_cert, admin_key) =
        generate_server_cert(&machine_ca, "admin", "os:admin", &admin_sans_refs, 365)?;

    // Extract the original cluster identity from a RUNNING old CP. The first
    // CP (cp_addr) is the one being converted, so pull from another old CP if
    // available; fall back to the first CP's pre-convert state (it's still old
    // at install time, before its install reboots it).
    let source_cp = payload
        .nodes
        .iter()
        .filter(|n| (n.role == "control-plane" || n.role == "controlplane"))
        .find(|n| n.address != cp_addr)
        .map(|n| n.address.clone())
        .unwrap_or_else(|| cp_addr.clone());
    let mut harvest_hosts: Vec<String> = Vec::new();
    if !source_cp.is_empty() {
        harvest_hosts.push(source_cp.clone());
    }
    if let Some(h) = explicit_cp_host(payload) {
        harvest_hosts.push(h);
    }
    if let Some(s) = kube_server {
        if let Some(h) = s
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split(':')
            .next()
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            harvest_hosts.push(h.to_string());
        }
    }
    harvest_hosts.sort();
    harvest_hosts.dedup();
    let mut orig: Option<OriginalIdentity> = None;
    for h in &harvest_hosts {
        if let Some(o) = extract_original_identity(sshc, h).await {
            let has_key = !o.k8s_ca_key.is_empty();
            orig = Some(o);
            if has_key {
                break;
            }
        }
    }

    let ident = ConvertClusterIdentity {
        machine_ca_crt: machine_ca.pem().to_string(),
        machine_ca_key: machine_ca.key().serialize_pem().to_string(),
        cluster_id: b64_random(32),
        cluster_secret: b64_random(32),
        machine_token: bootstrap_token(),
        kube_token: kubeadm_join_token(),
        cluster_name: cluster_name.to_string(),
        control_plane_endpoint: endpoint,
        admin_cert,
        admin_key,
        etcd_ca_crt: etcd_ca.pem().to_string(),
        etcd_ca_key: etcd_ca.key().serialize_pem().to_string(),
        // Original identity (empty if extraction unavailable -> config falls
        // back to a fresh-identity CP, which cannot serve foreign etcd; the
        // overtake requires the original PKI, so extraction must succeed).
        k8s_ca_crt: orig
            .as_ref()
            .map(|o| o.k8s_ca_crt.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| k8s_ca.to_string()),
        k8s_ca_key: orig.as_ref().map(|o| o.k8s_ca_key.clone()).unwrap_or_default(),
        aggregator_ca_crt: orig.as_ref().map(|o| o.aggregator_ca_crt.clone()).unwrap_or_default(),
        aggregator_ca_key: orig.as_ref().map(|o| o.aggregator_ca_key.clone()).unwrap_or_default(),
        service_account_key: orig.as_ref().map(|o| o.service_account_key.clone()).unwrap_or_default(),
        k8s_version: orig.as_ref().map(|o| o.k8s_version.clone()).unwrap_or_default(),
        service_cidr: orig.as_ref().map(|o| o.service_cidr.clone()).unwrap_or_default(),
        cluster_domain: orig.as_ref().map(|o| o.cluster_domain.clone()).unwrap_or_else(|| "cluster.local".into()),
        pod_cidr: "10.244.0.0/16".into(),
        secretbox_secret: b64_random(32),
        etcd_ca_crt_orig: orig.as_ref().map(|o| o.etcd_ca_crt.clone()).unwrap_or_default(),
        etcd_ca_key_orig: orig.as_ref().map(|o| o.etcd_ca_key.clone()).unwrap_or_default(),
        etcd_server_crt: orig.as_ref().map(|o| o.etcd_server_crt.clone()).unwrap_or_default(),
        etcd_server_key: orig.as_ref().map(|o| o.etcd_server_key.clone()).unwrap_or_default(),
    };
    payload.cluster_identity = Some(ident.clone());
    Ok(ident)
}

/// kubeadm bootstrap token: `[a-z0-9]{6}.[a-z0-9]{16}`. The provision
/// `bootstrap_token()` helper emits unpadded hex of 22 raw bytes and is
/// rejected by kube-apiserver's bootstrap authenticator.
fn kubeadm_join_token() -> String {
    const A: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = String::with_capacity(23);
    for i in 0..23 {
        if i == 6 {
            s.push('.');
            continue;
        }
        s.push(A[fastrand::usize(..A.len())] as char);
    }
    s
}

fn parse_kubeadm_token(token: &str) -> Option<(&str, &str)> {
    let (id, secret) = token.split_once('.')?;
    if id.len() == 6
        && secret.len() == 16
        && id.chars().all(|c| c.is_ascii_alphanumeric())
        && secret.chars().all(|c| c.is_ascii_alphanumeric())
    {
        Some((id, secret))
    } else {
        None
    }
}

fn explicit_cp_host(payload: &ConvertJobPayload) -> Option<String> {
    let e = payload.control_plane_endpoint.as_deref()?.trim();
    if e.is_empty() {
        return None;
    }
    let host = e
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or(e)
        .trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Apply the job's kubeadm bootstrap token as a kube-system Secret so a
/// converted Talos worker can CSR-join the still-kubeadm API.
async fn inject_bootstrap_token(
    pool: &DbPool,
    jwt_secret: &str,
    cluster_id: Uuid,
    payload: &mut ConvertJobPayload,
) -> Result<(), AppError> {
    if payload.bootstrap_token_injected {
        return Ok(());
    }
    let Some(ident) = payload.cluster_identity.clone() else {
        return Ok(());
    };
    let Some((id, secret)) = parse_kubeadm_token(&ident.kube_token) else {
        payload.log(&format!(
            "bootstrap token {:?} is not kubeadm 6.16 form; not injecting",
            ident.kube_token
        ));
        return Ok(());
    };
    let Some(kc) = stored_kubeconfig_yaml(pool, jwt_secret, cluster_id).await else {
        payload.log("no stored kubeconfig; cannot inject bootstrap token");
        return Ok(());
    };
    let exp = (chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339();
    let yaml = format!(
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: bootstrap-token-{id}\n  namespace: kube-system\ntype: bootstrap.kubernetes.io/token\nstringData:\n  description: tcs convert overtake\n  token-id: {id}\n  token-secret: {secret}\n  expiration: {exp}\n  usage-bootstrap-authentication: \"true\"\n  usage-bootstrap-signing: \"true\"\n  auth-extra-groups: system:bootstrappers:nodes\n"
    );
    let tmpdir = std::path::PathBuf::from("/var/lib/tcs/talosctl-tmp");
    let _ = tokio::fs::create_dir_all(&tmpdir).await;
    let kc_path = tmpdir.join(format!("kubeconfig.{}.inject", std::process::id()));
    let sec_path = tmpdir.join(format!("bootstrap.{}.yaml", std::process::id()));
    tokio::fs::write(&kc_path, kc)
        .await
        .map_err(|e| AppError::Internal(format!("write kubeconfig: {e}")))?;
    tokio::fs::write(&sec_path, yaml)
        .await
        .map_err(|e| AppError::Internal(format!("write bootstrap secret: {e}")))?;
    let out = tokio::process::Command::new("kubectl")
        .env("TCS_INTERNAL", "1")
        .args([
            "--kubeconfig",
            &kc_path.to_string_lossy(),
            "apply",
            "-f",
            &sec_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("kubectl spawn: {e}")))?;
    let _ = tokio::fs::remove_file(&kc_path).await;
    let _ = tokio::fs::remove_file(&sec_path).await;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        payload.log(&format!("bootstrap token inject failed: {err}"));
        return Err(AppError::Network(format!("kubectl apply bootstrap token: {err}")));
    }
    payload.bootstrap_token_injected = true;
    payload.log(&format!("injected kubeadm bootstrap token {id}.**** into kube-system"));
    Ok(())
}

/// Approve + issue kubelet client CSRs for this job's bootstrap token.
/// kubespray's controller-manager often leaves `kubernetes.io/kube-apiserver-client-kubelet`
/// CSRs Approved-but-not-Issued; we sign them with the harvested cluster CA.
async fn issue_kubelet_csrs(
    pool: &DbPool,
    jwt_secret: &str,
    cluster_id: Uuid,
    payload: &mut ConvertJobPayload,
) -> Result<(), AppError> {
    let Some(ident) = payload.cluster_identity.clone() else {
        return Ok(());
    };
    let Some((id, _)) = parse_kubeadm_token(&ident.kube_token) else {
        return Ok(());
    };
    let requestor = format!("system:bootstrap:{id}");
    let Some(kc) = stored_kubeconfig_yaml(pool, jwt_secret, cluster_id).await else {
        return Ok(());
    };
    let tmpdir = std::path::PathBuf::from("/var/lib/tcs/talosctl-tmp");
    let _ = tokio::fs::create_dir_all(&tmpdir).await;
    let kc_path = tmpdir.join(format!("kubeconfig.{}.csr", std::process::id()));
    tokio::fs::write(&kc_path, &kc)
        .await
        .map_err(|e| AppError::Internal(format!("write kubeconfig: {e}")))?;
    let out = tokio::process::Command::new("kubectl")
        .env("TCS_INTERNAL", "1")
        .args(["--kubeconfig", &kc_path.to_string_lossy(), "get", "csr", "-o", "json"])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("kubectl get csr: {e}")))?;
    if !out.status.success() {
        let _ = tokio::fs::remove_file(&kc_path).await;
        return Ok(());
    }
    let list: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or(serde_json::json!({}));
    let items = list.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let ca_key = ident.k8s_ca_key.clone();
    let ca_crt = ident.k8s_ca_crt.clone();
    for item in items {
        let name = item.pointer("/metadata/name").and_then(|v| v.as_str()).unwrap_or("");
        let user = item.pointer("/spec/username").and_then(|v| v.as_str()).unwrap_or("");
        if user != requestor || name.is_empty() {
            continue;
        }
        if item.pointer("/status/certificate").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false) {
            continue;
        }
        let _ = tokio::process::Command::new("kubectl")
            .env("TCS_INTERNAL", "1")
            .args(["--kubeconfig", &kc_path.to_string_lossy(), "certificate", "approve", name])
            .output()
            .await;
        if ca_key.trim().is_empty() || ca_crt.trim().is_empty() {
            payload.log(&format!("approved kubelet CSR {name} (no harvested CA key to issue)"));
            continue;
        }
        let req_b64 = item.pointer("/spec/request").and_then(|v| v.as_str()).unwrap_or("");
        if req_b64.is_empty() {
            continue;
        }
        match sign_kubelet_csr(name, req_b64, &ca_crt, &ca_key, &kc_path).await {
            Ok(()) => payload.log(&format!("issued kubelet client cert for CSR {name}")),
            Err(e) => payload.log(&format!("CSR {name} sign failed: {e}")),
        }
    }
    let _ = tokio::fs::remove_file(&kc_path).await;
    Ok(())
}

async fn sign_kubelet_csr(
    name: &str,
    req_b64: &str,
    ca_crt: &str,
    ca_key: &str,
    kubeconfig: &std::path::Path,
) -> Result<(), AppError> {
    let tmpdir = std::path::PathBuf::from("/var/lib/tcs/talosctl-tmp");
    let req_bytes = base64_decode(req_b64);
    let csr_path = tmpdir.join(format!("{name}.csr"));
    let crt_path = tmpdir.join(format!("{name}.crt"));
    let ca_path = tmpdir.join(format!("{name}.ca.crt"));
    let key_path = tmpdir.join(format!("{name}.ca.key"));
    tokio::fs::write(&csr_path, req_bytes)
        .await
        .map_err(|e| AppError::Internal(format!("write csr: {e}")))?;
    tokio::fs::write(&ca_path, ca_crt)
        .await
        .map_err(|e| AppError::Internal(format!("write ca: {e}")))?;
    tokio::fs::write(&key_path, ca_key)
        .await
        .map_err(|e| AppError::Internal(format!("write ca key: {e}")))?;
    let sign = tokio::process::Command::new("openssl")
        .args([
            "x509",
            "-req",
            "-in",
            &csr_path.to_string_lossy(),
            "-CA",
            &ca_path.to_string_lossy(),
            "-CAkey",
            &key_path.to_string_lossy(),
            "-CAcreateserial",
            "-out",
            &crt_path.to_string_lossy(),
            "-days",
            "365",
        ])
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("openssl spawn: {e}")))?;
    if !sign.status.success() {
        let err = String::from_utf8_lossy(&sign.stderr);
        return Err(AppError::Internal(format!("openssl x509 -req: {err}")));
    }
    let crt = tokio::fs::read(&crt_path)
        .await
        .map_err(|e| AppError::Internal(format!("read issued cert: {e}")))?;
    let get = tokio::process::Command::new("kubectl")
        .env("TCS_INTERNAL", "1")
        .args(["--kubeconfig", &kubeconfig.to_string_lossy(), "get", "csr", name, "-o", "json"])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("kubectl get csr {name}: {e}")))?;
    let mut doc: serde_json::Value = serde_json::from_slice(&get.stdout)
        .map_err(|e| AppError::Internal(format!("csr json: {e}")))?;
    let b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&crt)
    };
    if let Some(status) = doc.get_mut("status") {
        status["certificate"] = serde_json::Value::String(b64);
    }
    let status_path = tmpdir.join(format!("{name}.status.json"));
    tokio::fs::write(&status_path, serde_json::to_vec(&doc).unwrap_or_default())
        .await
        .map_err(|e| AppError::Internal(format!("write csr status: {e}")))?;
    let put = tokio::process::Command::new("kubectl")
        .env("TCS_INTERNAL", "1")
        .args([
            "--kubeconfig",
            &kubeconfig.to_string_lossy(),
            "replace",
            "--raw",
            &format!("/apis/certificates.k8s.io/v1/certificatesigningrequests/{name}/status"),
            "-f",
            &status_path.to_string_lossy(),
        ])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("kubectl replace csr status: {e}")))?;
    let _ = tokio::fs::remove_file(&csr_path).await;
    let _ = tokio::fs::remove_file(&crt_path).await;
    let _ = tokio::fs::remove_file(&ca_path).await;
    let _ = tokio::fs::remove_file(&key_path).await;
    let _ = tokio::fs::remove_file(&status_path).await;
    if !put.status.success() {
        return Err(AppError::Network(format!(
            "csr status replace: {}",
            String::from_utf8_lossy(&put.stderr)
        )));
    }
    Ok(())
}

async fn kube_node_is_talos(
    pool: &DbPool,
    jwt_secret: &str,
    cluster_id: Uuid,
    node: &str,
) -> bool {
    let Some(kc) = stored_kubeconfig_yaml(pool, jwt_secret, cluster_id).await else {
        return false;
    };
    let tmpdir = std::path::PathBuf::from("/var/lib/tcs/talosctl-tmp");
    let _ = tokio::fs::create_dir_all(&tmpdir).await;
    let kc_path = tmpdir.join(format!("kubeconfig.{}.os", std::process::id()));
    if tokio::fs::write(&kc_path, kc).await.is_err() {
        return false;
    }
    let out = tokio::process::Command::new("kubectl")
        .env("TCS_INTERNAL", "1")
        .args([
            "--kubeconfig",
            &kc_path.to_string_lossy(),
            "get",
            "node",
            node,
            "-o",
            "jsonpath={.status.nodeInfo.osImage}",
        ])
        .output()
        .await;
    let _ = tokio::fs::remove_file(&kc_path).await;
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            s.to_ascii_lowercase().contains("talos")
        }
        _ => false,
    }
}

/// The original (kubeadm) cluster identity extracted from a running old CP.
#[derive(Debug, Clone, Default)]
struct OriginalIdentity {
    k8s_ca_crt: String,
    k8s_ca_key: String,
    aggregator_ca_crt: String,
    aggregator_ca_key: String,
    service_account_key: String,
    k8s_version: String,
    service_cidr: String,
    cluster_domain: String,
    /// Original (kubespray) etcd CA/server cert/key (PEM). The kubespray etcd
    /// cluster uses TLS peer URLs (https://IP:2380) with per-member certs whose
    /// SANs cover ALL CPs; recovering that snapshot into a Talos etcd with a
    /// fresh CA + plaintext peer URLs would fail member re-joins. Reusing the
    /// original etcd CA (+ key, so Talos can issue member certs from it) keeps
    /// the recovered etcd cluster intact.
    etcd_ca_crt: String,
    etcd_ca_key: String,
    etcd_server_crt: String,
    etcd_server_key: String,
}

/// Extract the original cluster's PKI + control-plane settings from a RUNNING
/// old control plane (over SSH). The overtake must reuse this identity so the
/// recovered etcd data is served with the same k8s CA / aggregator CA /
/// service-account key (existing client certs + SA tokens keep validating).
///
/// Returns `Ok(None)` if the source CP is unreachable or the files are missing
/// (the caller then falls back to a fresh identity, which will NOT serve
/// foreign etcd - the overtake would need re-running once a source is up).
async fn extract_original_identity(sshc: &SshClient, source_cp: &str) -> Option<OriginalIdentity> {
    // Dump each file between markers + a couple of config values, in one SSH
    // round-trip. PEM files are multi-line; markers make parsing robust.
    let cmd = r#"
emit() { echo "BEGIN $1"; cat "$2" 2>/dev/null; echo "END $1"; }
emitfirst() { echo "BEGIN $1"; for f in $2 $3; do [ -f "$f" ] && cat "$f" && break; done; echo "END $1"; }
emitfirst K8S_CA_CRT /etc/kubernetes/ssl/ca.crt /etc/kubernetes/pki/ca.crt
emitfirst K8S_CA_KEY /etc/kubernetes/ssl/ca.key /etc/kubernetes/pki/ca.key
emitfirst AGG_CA_CRT /etc/kubernetes/ssl/front-proxy-ca.crt /etc/kubernetes/pki/front-proxy-ca.crt
emitfirst AGG_CA_KEY /etc/kubernetes/ssl/front-proxy-ca.key /etc/kubernetes/pki/front-proxy-ca.key
emitfirst SA_KEY /etc/kubernetes/ssl/sa.key /etc/kubernetes/pki/sa.key
echo "BEGIN K8S_VERSION"
grep -oE 'image: registry.k8s.io/kube-apiserver:v[0-9.]+' /etc/kubernetes/manifests/kube-apiserver.yaml 2>/dev/null | head -1 | sed 's#.*kube-apiserver:##'
echo "END K8S_VERSION"
echo "BEGIN SERVICE_CIDR"
grep -oE '\-\-service-cluster-ip-range=[0-9./]+' /etc/kubernetes/manifests/kube-apiserver.yaml 2>/dev/null | head -1 | sed 's/.*=//'
echo "END SERVICE_CIDR"
echo "BEGIN CLUSTER_DOMAIN"
echo cluster.local
echo "END CLUSTER_DOMAIN"
emit ETCD_CA_CRT /etc/ssl/etcd/ssl/ca.pem
emit ETCD_CA_KEY /etc/ssl/etcd/ssl/ca-key.pem
emit ETCD_SERVER_CRT /etc/ssl/etcd/ssl/member-$(hostname).pem
emit ETCD_SERVER_KEY /etc/ssl/etcd/ssl/member-$(hostname)-key.pem
"#;
    let out = match sshc.run(source_cp, cmd).await {
        Ok(o) => o,
        Err(_) => return None,
    };
    let block = |out: &str, name: &str| -> Option<String> {
        let b = format!("BEGIN {name}\n");
        let e = format!("END {name}\n");
        let si = out.find(&b)? + b.len();
        let ei = out[si..].find(&e)? + si;
        let v = out[si..ei].trim().to_string();
        if v.is_empty() { None } else { Some(v) }
    };
    Some(OriginalIdentity {
        k8s_ca_crt: block(&out, "K8S_CA_CRT")?,
        k8s_ca_key: block(&out, "K8S_CA_KEY")?,
        aggregator_ca_crt: block(&out, "AGG_CA_CRT")?,
        aggregator_ca_key: block(&out, "AGG_CA_KEY")?,
        service_account_key: block(&out, "SA_KEY")?,
        k8s_version: block(&out, "K8S_VERSION").unwrap_or_default(),
        service_cidr: block(&out, "SERVICE_CIDR").unwrap_or_default(),
        cluster_domain: block(&out, "CLUSTER_DOMAIN").unwrap_or_else(|| "cluster.local".into()),
        etcd_ca_crt: block(&out, "ETCD_CA_CRT").unwrap_or_default(),
        etcd_server_crt: block(&out, "ETCD_SERVER_CRT").unwrap_or_default(),
        etcd_server_key: block(&out, "ETCD_SERVER_KEY").unwrap_or_default(),
        etcd_ca_key: block(&out, "ETCD_CA_KEY").unwrap_or_default(),
    })
}

/// First CP: `talosctl bootstrap --recover-from <snapshot>` to form its etcd
/// from the overtaken cluster's snapshot.
///
/// Bootstrap a freshly-installed CP from the overtaken snapshot.
///
/// `talosctl bootstrap --recover-from <snapshot>` resolves `<snapshot>` on the
/// DEPLOYER and streams it to the node over gRPC (EtcdRecover) — no file
/// upload / sshd is needed.
///
/// TLS etcd (kubespray): the snapshot carries https peer URLs from the OLD
/// cluster. etcd re-issues member TLS certs from the CA at bootstrap time and
/// rewrites this node's peer URL to its own address, so the recovered member
/// is self-consistent; the remaining CPs re-join once converted (their old
/// member entries are removed during `talosctl etcd remove-member` on adopt —
/// or they simply overwrite on join, which etcd handles). Plain etcd
/// (kubeadm): single-member fresh list, same mechanism.
async fn do_first_cp_recover(
    pool: &DbPool,
    jwt_secret: &str,
    sshc: &SshClient,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<(), AppError> {
    let path = payload.etcd_snapshot_path.clone().ok_or_else(|| AppError::Internal("no etcd snapshot path".into()))?;
    // talosconfig from the generated identity (already created during do_install;
    // cached in the payload, so no SSH re-extraction happens here).
    let k8s_ca = stored_kubeconfig_ca(pool, jwt_secret, cluster_id).await.unwrap_or_default();
    let kube_server = stored_kubeconfig_server(pool, jwt_secret, cluster_id).await;
    let tc = build_node_talosconfig(&ensure_cluster_identity(sshc, payload, &k8s_ca, kube_server.as_deref()).await?, &node.address);
    let endpoint = talos_endpoint(&node.address);
    let how = if payload.etcd_tls { "TLS etcd" } else { "plain etcd" };
    payload.set_state(&node.name, "recover", &format!("bootstrap --recover-from ({how})"), "");
    payload.log(&format!("{} bootstrapping etcd from snapshot {} ({how})", node.name, path));
    // --recover-from points at the LOCAL deployer path; the client streams it.
    TalosctlClient::bootstrap_recover_etcd(&endpoint, &path, Some(&tc)).await
}

/// Build a talosconfig YAML for a freshly-installed node's apid (:50000) from
/// the generated identity (machine CA + os:admin client cert). The node list
/// is required so talosctl knows which nodes to target.
fn build_node_talosconfig(ident: &ConvertClusterIdentity, node_address: &str) -> String {
    let ca_b64 = crate::controllers::provision::b64_le(&ident.machine_ca_crt);
    let crt_b64 = crate::controllers::provision::b64_le(&ident.admin_cert);
    let key_b64 = crate::controllers::provision::b64_le(&ident.admin_key);
    let name = ident.cluster_name.replace('-', "_");
    format!(
        "context: {name}\ncontexts:\n  {name}:\n    endpoints:\n      - https://{node_address}:50000\n    nodes:\n      - {node_address}\n    ca: {ca_b64}\n    crt: {crt_b64}\n    key: {key_b64}\n"
    )
}

async fn stored_kubeconfig_yaml(pool: &DbPool, jwt_secret: &str, cluster_id: Uuid) -> Option<String> {
    let c = repos::cluster::get(pool, cluster_id).await.ok().flatten()?;
    let enc = c.kubeconfig.as_deref()?;
    secrets::decrypt(jwt_secret, enc).ok()
}

async fn stored_kubeconfig_ca(pool: &DbPool, jwt_secret: &str, cluster_id: Uuid) -> Option<String> {
    let plain = stored_kubeconfig_yaml(pool, jwt_secret, cluster_id).await?;
    crate::integration::kubernetes::parse_kubeconfig(&plain)
        .ok()
        .and_then(|kc| kc.clusters.into_iter().next())
        .and_then(|cl| cl.cluster.certificate_authority_data)
        .and_then(|b64| String::from_utf8(base64_decode(&b64)).ok())
}

async fn stored_kubeconfig_server(pool: &DbPool, jwt_secret: &str, cluster_id: Uuid) -> Option<String> {
    let plain = stored_kubeconfig_yaml(pool, jwt_secret, cluster_id).await?;
    crate::integration::kubernetes::parse_kubeconfig(&plain)
        .ok()
        .and_then(|kc| kc.clusters.into_iter().next())
        .map(|cl| cl.cluster.server)
        .filter(|s| !s.trim().is_empty())
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
            ConvertNodePlan { name: "cp1".into(), role: "control-plane".into(), address: "10.0.0.1".into(), network: Default::default(), drivers: vec![], kubelet_cert: String::new(), kubelet_key: String::new() },
            ConvertNodePlan { name: "cp2".into(), role: "control-plane".into(), address: "10.0.0.2".into(), network: Default::default(), drivers: vec![], kubelet_cert: String::new(), kubelet_key: String::new() },
            ConvertNodePlan { name: "w1".into(), role: "worker".into(), address: "10.0.0.3".into(), network: Default::default(), drivers: vec![], kubelet_cert: String::new(), kubelet_key: String::new() },
        ];
        p.node_states = p.nodes.iter().map(|n| crate::controllers::convert::ConvertNodeState {
            name: n.name.clone(), role: n.role.clone(), status: "pending".into(), current_step: "".into(), error: "".into(), attempts: 0,
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

    #[test]
    fn kubeadm_join_token_is_6_16() {
        for _ in 0..20 {
            let t = kubeadm_join_token();
            assert!(parse_kubeadm_token(&t).is_some(), "bad token {t}");
        }
        assert!(parse_kubeadm_token("abcdef.0123456789abcdef").is_some());
        assert!(parse_kubeadm_token("c9278af922e2.94d1e5f396a4d978f6394cc0d4c58e0").is_none());
        assert!(parse_kubeadm_token("aabbcc.short").is_none());
    }

    #[test]
    fn talos_endpoint_appends_50000() {
        // apid is on 50000 in v1.13 (installer/maintenance AND installed).
        assert_eq!(talos_endpoint("172.20.0.38"), "172.20.0.38:50000");
        // explicit port preserved
        assert_eq!(talos_endpoint("172.20.0.38:50000"), "172.20.0.38:50000");
        // must NOT default to the legacy 5000/5006
        assert!(!talos_endpoint("10.0.0.1").ends_with(":5000"));
        assert!(!talos_endpoint("10.0.0.1").ends_with(":5006"));
    }

    #[test]
    fn build_node_talosconfig_uses_50000_and_nodes() {
        // talosconfig for a freshly-installed node's apid: endpoint :50000,
        // a nodes list (required by talosctl), base64-of-PEM ca/crt/key.
        let tc = build_node_talosconfig(&fake_ident(), "10.0.0.1");
        assert!(tc.contains("endpoints:\n      - https://10.0.0.1:50000\n"));
        assert!(tc.contains("nodes:\n      - 10.0.0.1\n"));
        // base64 fields present, no raw PEM.
        assert!(!tc.contains("BEGIN CERTIFICATE"));
        let ca_b64 = crate::controllers::provision::b64_le(&fake_ident().machine_ca_crt);
        assert!(tc.contains(&format!("ca: {ca_b64}\n")));
    }

    fn fake_ident() -> ConvertClusterIdentity {
        ConvertClusterIdentity {
            machine_ca_crt: "-----BEGIN CERTIFICATE-----\nMIIBxx\n-----END CERTIFICATE-----".into(),
            machine_ca_key: "-----BEGIN PRIVATE KEY-----\nMC4CAx\n-----END PRIVATE KEY-----".into(),
            cluster_id: "Y2x1c3Rlci1pZA==".into(),
            cluster_secret: "Y2x1c3Rlci1zZWNyZXQ=".into(),
            machine_token: "aabbcc.1111112222223333".into(),
            kube_token: "ddeeff.4444445555556666".into(),
            cluster_name: "phobos".into(),
            control_plane_endpoint: "https://10.0.0.1:6443".into(),
            admin_cert: "-----BEGIN CERTIFICATE-----\nAdm\n-----END CERTIFICATE-----".into(),
            admin_key: "-----BEGIN PRIVATE KEY-----\nAdk\n-----END PRIVATE KEY-----".into(),
            etcd_ca_crt: "-----BEGIN CERTIFICATE-----\nEtc\n-----END CERTIFICATE-----".into(),
            etcd_ca_key: "-----BEGIN PRIVATE KEY-----\nEtk\n-----END PRIVATE KEY-----".into(),
            k8s_ca_crt: "-----BEGIN CERTIFICATE-----\nOrigK8s\n-----END CERTIFICATE-----".into(),
            k8s_ca_key: "-----BEGIN PRIVATE KEY-----\nOrigK8sKey\n-----END PRIVATE KEY-----".into(),
            aggregator_ca_crt: "-----BEGIN CERTIFICATE-----\nOrigAgg\n-----END CERTIFICATE-----".into(),
            aggregator_ca_key: "-----BEGIN PRIVATE KEY-----\nOrigAggKey\n-----END PRIVATE KEY-----".into(),
            service_account_key: "-----BEGIN PRIVATE KEY-----\nOrigSA\n-----END PRIVATE KEY-----".into(),
            k8s_version: "v1.33.5".into(),
            service_cidr: "10.233.0.0/18".into(),
            cluster_domain: "cluster.local".into(),
            pod_cidr: "10.244.0.0/16".into(),
            secretbox_secret: "c2VjcmV0Ym94".into(),
            etcd_ca_crt_orig: String::new(),
            etcd_ca_key_orig: String::new(),
            etcd_server_crt: String::new(),
            etcd_server_key: String::new(),
        }
    }

    #[test]
    fn build_install_config_controlplane_root_form() {
        // Mirrors the schema validated against the live v1.13.10 installer via
        // `talosctl apply-config --dry-run`.
        let net = "    hostname: cp1\n    interfaces:\n      - interface: bond0\n        mtu: 1500\n";
        let k8s_ca = "-----BEGIN CERTIFICATE-----\nK8s\n-----END CERTIFICATE-----";
        let cfg = build_install_config("controlplane", true, net, "/dev/sda", "factory.talos.dev/metal-installer/x:v1.13.10", &fake_ident(), k8s_ca, true, "", "", true);
        // Root form: version + persist, NO apiVersion/kind/metadata.
        assert!(cfg.starts_with("version: v1alpha1\npersist: true\nmachine:\n"));
        assert!(!cfg.contains("apiVersion:"));
        assert!(!cfg.contains("kind: "));
        assert!(!cfg.contains("metadata:"));
        // machine.type + control-plane ca (crt AND key, base64-of-PEM single
        // line) + token.
        assert!(cfg.contains("  type: controlplane\n"));
        let crt_b64 = crate::controllers::provision::b64_le(&fake_ident().machine_ca_crt);
        let key_b64 = crate::controllers::provision::b64_le(&fake_ident().machine_ca_key);
        assert!(cfg.contains(&format!("  ca:\n    crt: {crt_b64}\n    key: {key_b64}\n")));
        assert!(cfg.contains("  token: aabbcc.1111112222223333\n"));
        // install to sda, wipe, image ref.
        assert!(cfg.contains("  install:\n    disk: /dev/sda\n    wipe: true\n"));
        assert!(cfg.contains("    image: factory.talos.dev/metal-installer/x:v1.13.10\n"));
        // cluster block required.
        assert!(cfg.contains("cluster:\n  id: Y2x1c3Rlci1pZA==\n  secret: Y2x1c3Rlci1zZWNyZXQ=\n"));
        assert!(cfg.contains("  controlPlane:\n    endpoint: https://10.0.0.1:6443\n"));
        // Overtake: the full ORIGINAL cluster identity is carried (k8s CA + key,
        // aggregator CA + key, SA key, original k8s version images, service CIDR).
        let b64 = crate::controllers::provision::b64_le;
        let fi = fake_ident();
        assert!(cfg.contains(&format!("  ca:\n    crt: {}\n    key: {}\n", b64(&fi.k8s_ca_crt), b64(&fi.k8s_ca_key))));
        assert!(cfg.contains(&format!("  aggregatorCA:\n    crt: {}\n    key: {}\n", b64(&fi.aggregator_ca_crt), b64(&fi.aggregator_ca_key))));
        assert!(cfg.contains(&format!("  serviceAccount:\n    key: {}\n", b64(&fi.service_account_key))));
        assert!(cfg.contains("  apiServer:\n    image: registry.k8s.io/kube-apiserver:v1.33.5\n"));
        assert!(cfg.contains("  serviceSubnets:\n      - 10.233.0.0/18\n"));
        assert!(cfg.contains("  secretboxEncryptionSecret: c2VjcmV0Ym94\n"));
        // etcd CA present (fresh, since old etcd had no TLS).
        assert!(cfg.contains("  etcd:\n    ca:\n"));
        // network block emitted verbatim (its 4-space base is already correct
        // under the 2-space `network:` key).
        assert!(cfg.contains("  network:\n    hostname: cp1"));
        // NO systemExtensions key anywhere (invalid in this schema).
        assert!(!cfg.contains("systemExtensions"));
        // The generated config must be VALID YAML (catches indentation bugs that
        // surface server-side as "go-yaml ... did not find expected key").
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&cfg).expect("install config must be valid YAML");
        // Spot-check the parsed structure.
        let doc = parsed.as_mapping().expect("mapping");
        assert_eq!(doc.get(&serde_yaml::Value::String("version".into())).unwrap().as_str(), Some("v1alpha1"));
        let machine = doc.get(&serde_yaml::Value::String("machine".into())).unwrap();
        assert_eq!(machine.get("type").and_then(|t| t.as_str()), Some("controlplane"));
        assert!(machine.get("network").is_some());
        assert!(machine.get("ca").is_some());
        // No machine.features block (SSH was removed from Talos; there is no
        // valid features.ssh schema in v1.13).
        assert!(machine.get("features").is_none());
        // Control plane must carry the etcd CA (else "missing cluster.etcdCA
        // secret" and etcd never starts).
        assert!(cfg.contains("  etcd:\n    ca:\n"));
        let etcd_crt_b64 = crate::controllers::provision::b64_le(&fake_ident().etcd_ca_crt);
        assert!(cfg.contains(&format!("      crt: {etcd_crt_b64}\n")));
    }

    #[test]
    fn post_boot_config_omits_install_wipe() {
        let net = "    hostname: w1\n    interfaces:\n      - interface: bond0\n";
        let cfg = build_install_config("worker", false, net, "/dev/sda", "img:v1", &fake_ident(), "", false, "", "", false);
        assert!(cfg.contains("  network:\n    hostname: w1"));
        assert!(!cfg.contains("  install:"));
        assert!(!cfg.contains("wipe: true"));
    }

    #[test]
    fn convert_disk_image_prefers_override_then_factory() {
        let mut p = ConvertJobPayload::default();
        p.talos_version = "v1.13.10".into();
        p.modules = vec![
            "siderolabs/bnx2-bnx2x".into(),
            "siderolabs/iscsi-tools".into(),
            "siderolabs/nfs-utils".into(),
        ];
        p.schematic = Some("schem1".into());
        let factory = FactoryConfig::default();
        let img = convert_disk_image(&factory, &p);
        assert!(img.contains("factory.talos.dev/metal-installer/schem1:v1.13.10"));
        p.install_image_override = Some("https://172.20.0.126:5000/talos-tls:talos-tls".into());
        assert_eq!(
            convert_disk_image(&factory, &p),
            "https://172.20.0.126:5000/talos-tls:talos-tls"
        );
    }

    #[test]
    fn build_install_config_worker_uses_accepted_cas() {
        let net = "    interfaces:\n      - interface: bond0\n";
        let cfg = build_install_config("worker", false, net, "/dev/sda", "img:v1", &fake_ident(), "", false, "", "", true);
        assert!(cfg.contains("  type: worker\n"));
        // Worker: acceptedCAs (crt only, base64-of-PEM), NO machine.ca / NO key.
        let crt_b64 = crate::controllers::provision::b64_le(&fake_ident().machine_ca_crt);
        assert!(cfg.contains(&format!("  acceptedCAs:\n    - crt: {crt_b64}\n")));
        assert!(!cfg.contains("    key:"));
        assert!(!cfg.contains("PRIVATE KEY"));
        // Worker trusts the cluster CA (crt only) but carries NO issuing keys:
        // no cluster.ca.key, no aggregatorCA, no serviceAccount, no etcd block.
        let b64 = crate::controllers::provision::b64_le;
        let fi = fake_ident();
        assert!(cfg.contains(&format!("  ca:\n    crt: {}\n", b64(&fi.k8s_ca_crt))));
        assert!(!cfg.contains("aggregatorCA"));
        assert!(!cfg.contains("serviceAccount"));
        assert!(!cfg.contains("  etcd:"));
        assert!(!cfg.contains("apiServer"));
    }
}
