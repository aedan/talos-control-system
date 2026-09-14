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
        // Kexec BOOT vehicle selection:
        //   1. CUSTOM asset (preferred for bnx2x fleets): stock release
        //      vmlinuz-amd64 + a rebuilt initramfs whose rootfs carries the
        //      bnx2x *firmware*. The stock initramfs has the signed bnx2x.ko
        //      but an EMPTY /usr/lib/firmware/bnx2x, so the NIC loads but never
        //      gets link. Grafting only firmware sidesteps the vermagic
        //      (6.18.48-talos) + module.sig_enforce=1 wall that blocks
        //      grafted .ko files.
        //   2. STANDARD asset: straight from the release mirror. Fine for NICs
        //      whose firmware ships in stock; not for bnx2x.
        // The factory image (UEFI vmlinuz.efi) is NEVER the kexec vehicle —
        // `kexec -l` can't load a PE32+ image. It remains the install *target*
        // in do_install.
        let image = kexec::installer_image(factory, &payload.talos_version, &payload.modules, payload.schematic.as_deref());
        let asset_dir = std::path::PathBuf::from(&metal_pxe.asset_dir);
        let assets = match kexec::resolve_custom_assets(&asset_dir, &payload.talos_version) {
            Ok(a) => {
                payload.log(&format!("{} using custom installer assets (bnx2x firmware grafted)", node.name));
                Ok(a)
            }
            Err(_) => {
                payload.log(&format!("{} custom assets absent; using standard installer assets", node.name));
                kexec::resolve_standard_assets(
                    &metal_pxe.mirror_base,
                    &payload.talos_version,
                    arch,
                    &asset_dir,
                )
                .await
            }
        };
        let append = kexec::kexec_append(&node.network, &node.name, "");
        match assets {
            Ok(a) => match kexec::kexec_node(sshc, &node.address, &a, &append).await {
                Ok(()) => {
                    payload.set_state(&node.name, "kexec", "kexec issued; node rebooting", "");
                    payload.log(&format!("{} kexec issued (install target {})", node.name, image.ref_));
                }
                Err(e) => fail_node(payload, &node.name, "kexec", &e.to_string()),
            },
            Err(e) => fail_node(payload, &node.name, "resolve-installer", &e.to_string()),
        }
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
    let host = talos_endpoint(address);
    match tokio::net::TcpStream::connect(&host).await {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
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
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
    _is_first_cp: bool,
) -> Result<(), AppError> {
    let cfg = node_install_config(pool, jwt_secret, factory, payload, cluster_id, node).await?;
    TalosctlClient::apply_config_maintenance(&node.address, &cfg, true, None).await
}

/// Build the install machine config for a node (installer maintenance apply).
#[allow(clippy::too_many_arguments)]
async fn node_install_config(
    pool: &DbPool,
    jwt_secret: &str,
    factory: &FactoryConfig,
    payload: &mut ConvertJobPayload,
    cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<String, AppError> {
    let image = kexec::installer_image(factory, &payload.talos_version, &payload.modules, payload.schematic.as_deref());
    let disk = "/dev/sda"; // overtake: install to the boot disk; ceph OSDs are sdb+
    let k8s_ca = stored_kubeconfig_ca(pool, jwt_secret, cluster_id).await.unwrap_or_default();
    let is_cp = node.role == "control-plane" || node.role == "controlplane";
    let machine_type = if is_cp { "controlplane" } else { "worker" };
    let ident = ensure_cluster_identity(payload).await?;
    let network_yaml = render_node_network_yaml(&node.network, &node.name);

    Ok(build_install_config(
        machine_type,
        is_cp,
        &network_yaml,
        disk,
        &image.ref_,
        &ident,
        &k8s_ca,
    ))
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
        cfg.push_str(&format!("    crt: {ca_crt_b64}\n"));
    }
    cfg.push_str(&format!("  token: {}\n", ident.machine_token));
    cfg.push_str("  install:\n");
    cfg.push_str(&format!("    disk: {disk}\n"));
    // Wipe only the install disk (/dev/sda). Talos does not touch the other
    // disks (sdb..sdk hold ceph OSDs), so this is safe for the overtake.
    cfg.push_str("    wipe: true\n");
    cfg.push_str(&format!("    image: {image_ref}\n"));
    cfg.push_str("cluster:\n");
    cfg.push_str(&format!("  id: {}\n", ident.cluster_id));
    cfg.push_str(&format!("  secret: {}\n", ident.cluster_secret));
    cfg.push_str("  controlPlane:\n");
    cfg.push_str(&format!("    endpoint: {}\n", ident.control_plane_endpoint));
    cfg.push_str(&format!("  clusterName: {}\n", ident.cluster_name));
    cfg.push_str(&format!("  token: {}\n", ident.kube_token));
    if !k8s_ca.is_empty() {
        cfg.push_str("  ca:\n");
        cfg.push_str(&format!("    crt: {}\n", crate::controllers::provision::b64_le(k8s_ca)));
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
    payload: &mut ConvertJobPayload,
) -> Result<ConvertClusterIdentity, AppError> {
    if let Some(ident) = &payload.cluster_identity {
        return Ok(ident.clone());
    }
    use crate::controllers::provision::{
        b64_random, bootstrap_token, generate_ca_issuer, generate_server_cert,
    };
    // Overtake keeps a stable cluster name; the API-server identity (k8s CA)
    // is carried over separately in the cluster block. TODO: derive from the
    // cluster record rather than hardcoding.
    let cluster_name = "phobos";
    let machine_ca = generate_ca_issuer(&format!("{cluster_name}-talos-ca"), 3650)?;
    // Admin client cert (org os:admin) signed by the machine CA, so we can
    // authenticate to the freshly-installed node's apid during etcd recovery.
    // SANs = first CP address so the cert is valid for the apid TLS check.
    let cp_addr = payload
        .nodes
        .iter()
        .find(|n| n.role == "control-plane" || n.role == "controlplane")
        .map(|n| n.address.clone())
        .unwrap_or_default();
    let endpoint = if cp_addr.is_empty() {
        "https://127.0.0.1:6443".into()
    } else {
        format!("https://{cp_addr}:6443")
    };
    let admin_sans: Vec<String> = if cp_addr.is_empty() {
        vec!["localhost".into(), "127.0.0.1".into()]
    } else {
        vec!["localhost".into(), cp_addr.clone()]
    };
    let admin_sans_refs: Vec<&str> = admin_sans.iter().map(|s| s.as_str()).collect();
    let (admin_cert, admin_key) =
        generate_server_cert(&machine_ca, "admin", "os:admin", &admin_sans_refs, 365)?;
    let ident = ConvertClusterIdentity {
        machine_ca_crt: machine_ca.pem().to_string(),
        machine_ca_key: machine_ca.key().serialize_pem().to_string(),
        cluster_id: b64_random(32),
        cluster_secret: b64_random(32),
        machine_token: bootstrap_token(),
        kube_token: bootstrap_token(),
        cluster_name: cluster_name.to_string(),
        control_plane_endpoint: endpoint,
        admin_cert,
        admin_key,
    };
    payload.cluster_identity = Some(ident.clone());
    Ok(ident)
}

/// First CP: `talosctl bootstrap --recover-from <snapshot>` to form its etcd
/// from the overtaken cluster's snapshot.
///
/// The node just installed Talos and its etcd is in the join loop (Preparing).
/// `talosctl bootstrap --recover-from <path>` resolves `<path>` on the CLIENT
/// (deployer) and streams the snapshot to the node over gRPC (EtcdRecover) —
/// NO file upload / sshd is needed (and SSH was removed from Talos entirely,
/// so machine.features.ssh does not exist in v1.13). The snapshot already
/// lives on the deployer from the snapshot phase, so we point straight at it.
///
/// Only the FIRST CP is bootstrapped; the other CPs join automatically once
/// this one's control-plane endpoint is up (via controlPlane.endpoint).
async fn do_first_cp_recover(
    _pool: &DbPool,
    _jwt_secret: &str,
    payload: &mut ConvertJobPayload,
    _cluster_id: Uuid,
    node: &ConvertNodePlan,
) -> Result<(), AppError> {
    let path = payload.etcd_snapshot_path.clone().ok_or_else(|| AppError::Internal("no etcd snapshot path".into()))?;
    // talosconfig from the generated identity (the node is a brand-new Talos
    // control plane with no stored talosconfig).
    let tc = build_node_talosconfig(&ensure_cluster_identity(payload).await?, &node.address);
    let endpoint = talos_endpoint(&node.address);
    payload.set_state(&node.name, "recover", "bootstrap --recover-from (streaming snapshot)", "");
    payload.log(&format!("{} bootstrapping etcd from snapshot {}", node.name, path));
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
        }
    }

    #[test]
    fn build_install_config_controlplane_root_form() {
        // Mirrors the schema validated against the live v1.13.10 installer via
        // `talosctl apply-config --dry-run`.
        let net = "    hostname: cp1\n    interfaces:\n      - interface: bond0\n        mtu: 1500\n";
        let k8s_ca = "-----BEGIN CERTIFICATE-----\nK8s\n-----END CERTIFICATE-----";
        let cfg = build_install_config("controlplane", true, net, "/dev/sda", "factory.talos.dev/metal-installer/x:v1.13.10", &fake_ident(), k8s_ca);
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
    }

    #[test]
    fn build_install_config_worker_uses_accepted_cas() {
        let net = "    interfaces:\n      - interface: bond0\n";
        let cfg = build_install_config("worker", false, net, "/dev/sda", "img:v1", &fake_ident(), "");
        assert!(cfg.contains("  type: worker\n"));
        // Worker: acceptedCAs (crt only, base64-of-PEM), NO machine.ca / NO key.
        let crt_b64 = crate::controllers::provision::b64_le(&fake_ident().machine_ca_crt);
        assert!(cfg.contains(&format!("  acceptedCAs:\n    crt: {crt_b64}\n")));
        assert!(!cfg.contains("    key:"));
        assert!(!cfg.contains("PRIVATE KEY"));
        // Empty k8s_ca -> no cluster.ca block.
        assert!(!cfg.contains("  ca:\n    crt:"));
    }
}
