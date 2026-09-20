//! In-place (non-Talos -> Talos) conversion: controller + job payload.
//!
//! The controller handles the API surface (preview / start / status / cancel)
//! and the read-only capture sweep. The actual node conversion (etcd snapshot,
//! kexec, install, recover, join, adopt) is driven by `runtime::convert_scheduler`
//! off a resumable `provision_jobs` row (kind = "convert").

use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Config;
use crate::controllers::cluster::ClusterController;
use crate::db::pool::DbPool;
use crate::db::repos::{self};
use crate::integration::network_capture::{network_usable, NetworkCapture, NodeNetworkCapture};
use crate::integration::ssh::SshClient;
use crate::AppError;

pub const JOB_KIND: &str = "convert";

/// Deduped, trimmed operator module list. Empty is allowed: start() then
/// fills from captured-driver recommendations, or kexec uses the stock
/// installer when nothing applies.
pub fn normalize_convert_modules(mods: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in mods {
        let t = m.trim();
        if t.is_empty() {
            continue;
        }
        if !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}

// ── payload (persisted on the provision_job row) ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertNodePlan {
    pub name: String,
    pub role: String, // "control-plane" | "worker"
    pub address: String,
    pub network: NodeNetworkCapture,
    pub drivers: Vec<String>,
    /// Existing kubelet client cert (PEM). When set, the Talos kubelet uses
    /// this cert to authenticate to the apiserver (overtake: the node keeps
    /// its identity). Empty for fresh installs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kubelet_cert: String,
    /// Existing kubelet client key (PEM).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kubelet_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertNodeState {
    pub name: String,
    pub role: String,
    pub status: String, // pending|snapshot|kexec|install|reboot|join|recover|done|failed|skipped
    pub current_step: String,
    pub error: String,
    /// Retry counter for transient steps (etcd-recovery "not ready yet");
    /// bounds the retry loop so a genuine fault fails instead of looping.
    #[serde(default)]
    pub attempts: u32,
    /// After `talosctl install`, :50000 is still the *installer* until it
    /// reboots. Set once we have observed apid down so the next apid-up is
    /// the installed OS (needed before etcd recover).
    #[serde(default)]
    pub installer_gone: bool,
}

fn default_cluster_name() -> String {
    "phobos".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertJobPayload {
    pub talos_version: String,
    /// Target cluster name (from the cluster record; defaults to "phobos" for
    /// payloads saved before this field existed).
    #[serde(default = "default_cluster_name")]
    pub cluster_name: String,
    /// True when the original (kubespray) etcd already speaks TLS — detected by
    /// the snapshot probe. When true, etcd recovery must run `snapshot restore`
    /// (member list / TLS peer URLs are stale); the fresh-install bootstrap
    /// cannot use `--recover-from` directly in that case.
    #[serde(default)]
    pub etcd_tls: bool,
    pub modules: Vec<String>,
    pub schematic: Option<String>,
    /// Ordered conversion plan: control-plane first, then workers.
    pub nodes: Vec<ConvertNodePlan>,
    pub phase: String, // snapshot|control-plane|workers|adopt|done
    pub current_index: usize,
    pub node_states: Vec<ConvertNodeState>,
    pub etcd_snapshot_path: Option<String>,
    pub etcd_snapshot_size: i64,
    /// Talos cluster identity generated ONCE at job start and shared by every
    /// node (the machine CA + cluster secrets must be identical across all CPs
    /// and workers so they join the same Talos control plane). Populated lazily
    /// on the first tick that needs it, then persisted in the payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_identity: Option<ConvertClusterIdentity>,
    /// Optional override for the install image ref (bypasses the factory
    /// schematic). Use when the standard factory image lacks required NIC
    /// firmware (e.g. bnx2x) and a locally-patched image is available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_image_override: Option<String>,
    /// Explicit control-plane endpoint supplied at job start, for worker-only
    /// overtakes (no CP in the node list). Used by ensure_cluster_identity when
    /// deriving the join endpoint; empty when a CP node is present in the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_plane_endpoint: Option<String>,
    /// True after the kubeadm bootstrap token from clusterIdentity.kubeToken
    /// has been applied as a kube-system Secret (worker-only overtake).
    #[serde(default)]
    pub bootstrap_token_injected: bool,
    pub steps_log: Vec<String>,
}

/// Shared Talos identity for an overtake. Generated once per convert job.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertClusterIdentity {
    /// Machine CA cert (PEM, base64 lines ready for YAML block scalar).
    pub machine_ca_crt: String,
    /// Machine CA key (PEM).
    pub machine_ca_key: String,
    /// cluster.id (b64).
    pub cluster_id: String,
    /// cluster.secret (b64).
    pub cluster_secret: String,
    /// Machine join/bootstrap token.
    pub machine_token: String,
    /// Kubecontrolplane join token (cluster.token).
    pub kube_token: String,
    /// Cluster name.
    pub cluster_name: String,
    /// controlPlane.endpoint (https://<first-cp-ip>:6443).
    pub control_plane_endpoint: String,
    /// Admin client cert (PEM) signed by the machine CA, org os:admin. Used to
    /// build a talosconfig for talking to the freshly-installed node's apid
    /// during etcd recovery.
    pub admin_cert: String,
    /// Admin client key (PEM) matching `admin_cert`.
    pub admin_key: String,
    /// etcd CA cert (PEM). Talos etcd is always TLS; a fresh CA is generated
    /// (the overtaken kubeadm etcd has no TLS, so there is no CA to match).
    /// Required in the control-plane config as cluster.etcd.ca, else the
    /// RootEtcdController fails with "missing cluster.etcdCA secret".
    pub etcd_ca_crt: String,
    /// etcd CA key (PEM).
    pub etcd_ca_key: String,
    // --- Original cluster identity (extracted from a running old CP). The
    // apiserver/scheduler/controller-manager are static pods gated on
    // secrets.KubernetesRoot, which is derived from the machine config's
    // cluster identity. For an OVERTAKE the recovered kubeadm etcd data must be
    // served with the ORIGINAL identity (k8s CA, aggregator/front-proxy CA,
    // service-account key) so existing client certs + SA tokens validate. A
    // fresh identity cannot serve foreign data. ---
    /// Original k8s/apiserver CA (PEM) - becomes cluster.ca.crt.
    pub k8s_ca_crt: String,
    /// Original k8s CA key (PEM) - becomes cluster.ca.key.
    pub k8s_ca_key: String,
    /// Original front-proxy/aggregator CA (PEM) - cluster.aggregatorCA.crt.
    pub aggregator_ca_crt: String,
    /// Original front-proxy CA key (PEM) - cluster.aggregatorCA.key.
    pub aggregator_ca_key: String,
    /// Original service-account signing key (PEM) - cluster.serviceAccount.key.
    pub service_account_key: String,
    /// k8s control-plane component image tag, e.g. "v1.33.5" (from the running
    /// kube-apiserver manifest).
    pub k8s_version: String,
    /// Service cluster IP range, e.g. "10.233.0.0/18".
    pub service_cidr: String,
    /// Cluster domain, e.g. "cluster.local".
    pub cluster_domain: String,
    /// Pod CIDR (for the Talos cluster.network; default 10.244.0.0/16).
    pub pod_cidr: String,
    /// secretbox encryption secret (fresh 32-byte b64 - encrypts the stored
    /// cluster secrets on the node; a fresh value is fine since it only
    /// protects at-rest storage of the CAs we provide).
    pub secretbox_secret: String,
    /// Original (kubespray) etcd CA (PEM). When set, Talos etcd uses it so the
    /// recovered snapshot's peer URLs + member TLS stay compatible across the
    /// conversion; empty -> Talos issues a fresh etcd CA.
    pub etcd_ca_crt_orig: String,
    /// Original etcd CA key (PEM; ca-key.pem lives on kubespray CPs).
    pub etcd_ca_key_orig: String,
    /// Original etcd server cert (PEM; SANs cover all CP hostnames + IPs).
    pub etcd_server_crt: String,
    /// Original etcd server key (PEM).
    pub etcd_server_key: String,
}

impl ConvertJobPayload {
    pub fn log(&mut self, line: &str) {
        self.steps_log.push(format!("{} {line}", Utc::now().to_rfc3339()));
        if self.steps_log.len() > 200 {
            let drain = self.steps_log.len() - 200;
            self.steps_log.drain(0..drain);
        }
    }
    fn state_mut(&mut self, name: &str) -> Option<&mut ConvertNodeState> {
        self.node_states.iter_mut().find(|s| s.name == name)
    }
    pub fn set_state(&mut self, name: &str, status: &str, step: &str, error: &str) {
        if let Some(s) = self.state_mut(name) {
            s.status = status.to_string();
            s.current_step = step.to_string();
            s.error = error.to_string();
        }
    }
}

// ── API DTOs (match frontend/src/lib/api/convert.ts) ──────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewNode {
    pub name: String,
    pub role: String,
    pub os_type: String,
    pub os_image: String,
    pub ssh_ok: bool,
    pub ssh_error: String,
    pub drivers: Vec<String>,
    pub recommended_modules: Vec<String>,
    pub network: NodeNetworkCapture,
    /// Existing kubelet client cert (PEM) — captured from the node so the
    /// overtake preserves the node's k8s identity.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kubelet_cert: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kubelet_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEtcd {
    pub cp_nodes: Vec<String>,
    pub embedded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertPreview {
    pub cluster_name: String,
    pub talos_version: String,
    pub kubernetes_version: String,
    pub nodes: Vec<PreviewNode>,
    pub etcd: PreviewEtcd,
    pub can_convert: bool,
    pub blockers: Vec<String>,
    /// kubeconfig cluster.server (e.g. https://172.20.0.38:6443). The
    /// operator should use this as the control-plane endpoint; a random CP
    /// IP may 401 bootstrap tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubeconfig_server: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEtcdSnapshot {
    pub taken: bool,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertStatusDto {
    pub job_id: String,
    pub status: String,
    pub phase: String,
    pub etcd_snapshot: StatusEtcdSnapshot,
    pub nodes: Vec<ConvertNodeState>,
    pub steps_log: Vec<String>,
}

// ── controller ─────────────────────────────────────────────────────────────

pub struct ConvertController {
    pool: DbPool,
    pub jwt_secret: Arc<str>,
    ssh: SshClient,
    pub factory: crate::config::FactoryConfig,
    pub metal_pxe: crate::config::MetalPxeConfig,
    _cluster: std::marker::PhantomData<ClusterController>,
}

impl ConvertController {
    pub fn new(pool: DbPool, config: &Config) -> Self {
        Self {
            pool,
            jwt_secret: Arc::from(config.auth.jwt_secret.as_str()),
            ssh: SshClient::new(config.ssh.clone()),
            factory: config.factory.clone(),
            metal_pxe: config.metal.pxe.clone(),
            _cluster: std::marker::PhantomData,
        }
    }

    /// Read-only capture sweep: SSH to every node, gather networking + drivers,
    /// recommend modules, report blockers. Does not mutate the cluster.
    pub async fn preview(
        &self,
        cluster_id: Uuid,
        talos_version: String,
        modules: Vec<String>,
    ) -> Result<ConvertPreview, AppError> {
        let cluster = repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound("cluster not found".into()))?;
        let machines = repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let capture = NetworkCapture::new(self.ssh.clone());

        let mut nodes = Vec::new();
        let mut blockers = Vec::new();
        let kubeconfig_server = cluster.kubeconfig.as_deref().and_then(|enc| {
            crate::utils::secrets::decrypt(&self.jwt_secret, enc)
                .ok()
                .and_then(|plain| crate::integration::kubernetes::parse_kubeconfig(&plain).ok())
                .and_then(|kc| kc.clusters.into_iter().next())
                .map(|cl| cl.cluster.server)
                .filter(|s| !s.trim().is_empty())
        });
        let mut cp_nodes = Vec::new();

        for m in &machines {
            if m.address.trim().is_empty() {
                nodes.push(PreviewNode {
                    name: m.hostname.clone(),
                    role: m.machine_type.clone(),
                    os_type: m.os_type.clone().unwrap_or_default(),
                    os_image: String::new(),
                    ssh_ok: false,
                    ssh_error: "no node address".into(),
                    drivers: vec![],
                    recommended_modules: vec![],
                    network: NodeNetworkCapture::default(),
                    kubelet_cert: String::new(),
                    kubelet_key: String::new(),
                });
                blockers.push(format!("node {}: no address recorded", m.hostname));
                continue;
            }
            match capture.capture(&m.address).await {
                Ok(res) => {
                    // Always recommend from *this node's* captured drivers so
                    // the wizard can pre-select 10Gb NIC / iSCSI / NFS modules
                    // independently of what the operator already ticked.
                    let recommended = recommend_modules(&res.drivers);
                    if m.machine_type == "control-plane" || m.machine_type == "controlplane" {
                        cp_nodes.push(m.hostname.clone());
                    }
                    nodes.push(PreviewNode {
                        name: m.hostname.clone(),
                        role: m.machine_type.clone(),
                        os_type: m.os_type.clone().unwrap_or_else(|| "talos".into()),
                        os_image: os_image_for(m),
                        ssh_ok: true,
                        ssh_error: String::new(),
                        drivers: res.drivers.clone(),
                        recommended_modules: recommended,
                        network: res.network,
                        kubelet_cert: res.kubelet_cert,
                        kubelet_key: res.kubelet_key,
                    });
                }
                Err(e) => {
                    // Unreachable nodes are skipped in the picker, not a hard
                    // blocker — convert the SSH-ok subset.
                    nodes.push(PreviewNode {
                        name: m.hostname.clone(),
                        role: m.machine_type.clone(),
                        os_type: m.os_type.clone().unwrap_or_else(|| "talos".into()),
                        os_image: os_image_for(m),
                        ssh_ok: false,
                        ssh_error: e.to_string(),
                        drivers: vec![],
                        recommended_modules: vec![],
                        network: NodeNetworkCapture::default(),
                        kubelet_cert: String::new(),
                        kubelet_key: String::new(),
                    });
                }
            }
        }

        let ssh_ok = nodes.iter().filter(|n| n.ssh_ok).count();
        if ssh_ok == 0 {
            blockers.push("no SSH-reachable nodes — convert needs SSH to recapture network and kexec".into());
        }
        let can_convert = ssh_ok >= 1;

        Ok(ConvertPreview {
            cluster_name: cluster.name,
            talos_version,
            kubernetes_version: cluster.control_plane_version,
            nodes,
            etcd: PreviewEtcd {
                cp_nodes,
                embedded: true, // kubeadm-style embedded etcd assumed for non-Talos
            },
            can_convert,
            blockers,
            kubeconfig_server,
        })
    }

    /// Create a resumable convert job. `nodes` are in the desired conversion
    /// order (control-plane first); each carries its captured network + drivers.
    pub async fn start(
        &self,
        user: &str,
        cluster_id: Uuid,
        body: StartBody,
    ) -> Result<Uuid, AppError> {
        let cluster = repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound("cluster not found".into()))?;
        let machines = repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut plans = Vec::new();
        let by_name: std::collections::HashMap<String, &crate::db::models::machine::Machine> =
            machines.iter().map(|m| (m.hostname.clone(), m)).collect();

        // Always SSH-capture each selected node at start so the job payload
        // holds live bonds/IPs/DNS. The wizard may send a preview snapshot;
        // we do not trust it as the kexec/machine-config source of truth.
        let capture = NetworkCapture::new(self.ssh.clone());
        for n in &body.nodes {
            let Some(m) = by_name.get(&n.name) else { continue };
            if m.address.trim().is_empty() {
                return Err(AppError::InvalidInput(format!(
                    "node {}: no address recorded, cannot capture network",
                    n.name
                )));
            }
            let (network, drivers, kubelet_cert, kubelet_key) = match capture.capture(&m.address).await {
                Ok(res) => {
                    let drivers = if n.drivers.is_empty() {
                        res.drivers
                    } else {
                        n.drivers.clone()
                    };
                    (res.network, drivers, res.kubelet_cert, res.kubelet_key)
                }
                Err(e) => {
                    return Err(AppError::InvalidInput(format!(
                        "node {}: SSH network capture failed before kexec ({e})",
                        n.name
                    )));
                }
            };
            if !network_usable(&network) {
                return Err(AppError::InvalidInput(format!(
                    "node {}: captured network has no static IP/gateway (refusing kexec)",
                    n.name
                )));
            }
            plans.push(ConvertNodePlan {
                name: n.name.clone(),
                role: n.role.clone(),
                address: m.address.clone(),
                network,
                drivers,
                kubelet_cert,
                kubelet_key,
            });
        }
        let _ = by_name;

        let mut modules = normalize_convert_modules(&body.modules);
        if modules.is_empty() {
            for p in &plans {
                for m in recommend_modules(&p.drivers) {
                    if !modules.iter().any(|x| x == &m) {
                        modules.push(m);
                    }
                }
            }
        }
        let schematic = if modules.is_empty() {
            None
        } else {
            Some(
                crate::integration::image_factory::ImageFactoryClient::new(&self.factory.normalized_base())
                    .create_schematic(&modules)
                    .await?,
            )
        };

        let now = Utc::now();
        let payload = ConvertJobPayload {
            talos_version: body.talos_version,
            cluster_name: cluster.name.clone(),
            etcd_tls: false,
            modules,
            schematic,
            nodes: plans,
            phase: "snapshot".into(),
            current_index: 0,
            node_states: body
                .nodes
                .iter()
                .map(|n| ConvertNodeState {
                    name: n.name.clone(),
                    role: n.role.clone(),
                    status: "pending".into(),
                    current_step: String::new(),
                    error: String::new(),
                    attempts: 0,
                    installer_gone: false,
                })
                .collect(),
            etcd_snapshot_path: None,
            etcd_snapshot_size: 0,
            cluster_identity: None,
            install_image_override: body.install_image.clone(),
            control_plane_endpoint: body.control_plane_endpoint.clone(),
            bootstrap_token_injected: false,
            steps_log: vec![format!("{now} convert job created ({} nodes)", body.nodes.len())],
        };

        let id = Uuid::new_v4();
        let job = repos::provision_job::ProvisionJob {
            id,
            cluster_id: Some(cluster_id),
            kind: JOB_KIND.into(),
            status: "running".into(),
            desired_workers: 0,
            payload: Some(serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into())),
            error: None,
            created_by: Some(user.to_string()),
            created_at: now,
            updated_at: now,
        };
        repos::provision_job::create(&self.pool, &job).await?;
        crate::utils::audit::log_action(
            &self.pool,
            user,
            "start_convert",
            &id.to_string(),
            &cluster_id.to_string(),
        )
        .await;
        Ok(id)
    }

    /// Latest convert job for the cluster, as a status DTO (or NotFound).
    pub async fn status(&self, cluster_id: Uuid) -> Result<ConvertStatusDto, AppError> {
        let job = self
            .pool
            .fetch_optional_as::<repos::provision_job::ProvisionJob>(
                "SELECT * FROM provision_jobs WHERE cluster_id = ? AND kind = ? ORDER BY created_at DESC LIMIT 1",
                &[
                    crate::db::pool::SqlVal::Uuid(cluster_id),
                    crate::db::pool::SqlVal::text(JOB_KIND),
                ],
            )
            .await?
            .ok_or_else(|| AppError::NotFound("no convert job for this cluster".into()))?;
        let payload: ConvertJobPayload = serde_json::from_str(
            job.payload.as_deref().unwrap_or("{}"),
        )
        .unwrap_or_default();
        Ok(ConvertStatusDto {
            job_id: job.id.to_string(),
            status: job.status,
            phase: payload.phase,
            etcd_snapshot: StatusEtcdSnapshot {
                taken: payload.etcd_snapshot_path.is_some(),
                size_bytes: payload.etcd_snapshot_size,
            },
            nodes: payload.node_states,
            steps_log: payload.steps_log,
        })
    }

    pub async fn cancel(&self, user: &str, cluster_id: Uuid) -> Result<(), AppError> {
        let job = self
            .pool
            .fetch_optional_as::<repos::provision_job::ProvisionJob>(
                "SELECT * FROM provision_jobs WHERE cluster_id = ? AND kind = ? ORDER BY created_at DESC LIMIT 1",
                &[
                    crate::db::pool::SqlVal::Uuid(cluster_id),
                    crate::db::pool::SqlVal::text(JOB_KIND),
                ],
            )
            .await?
            .ok_or_else(|| AppError::NotFound("no convert job for this cluster".into()))?;
        if job.status == "running" {
            repos::provision_job::update_status(&self.pool, job.id, "cancelled", Some("cancelled by user"), None).await?;
            crate::utils::audit::log_action(&self.pool, user, "cancel_convert", &job.id.to_string(), &cluster_id.to_string()).await;
        }
        Ok(())
    }

    /// Active convert jobs (driven by the scheduler).
    pub fn active_jobs(pool: &DbPool) -> impl std::future::Future<Output = Result<Vec<repos::provision_job::ProvisionJob>, AppError>> + '_ {
        async move {
            pool.fetch_all_as(
                "SELECT * FROM provision_jobs WHERE kind = ? AND status = 'running' ORDER BY created_at ASC",
                &[crate::db::pool::SqlVal::text(JOB_KIND)],
            )
            .await
        }
    }

    /// Persist a payload + status back onto the job row.
    pub async fn save_job(pool: &DbPool, id: Uuid, status: &str, payload: &ConvertJobPayload, error: Option<&str>) -> Result<(), AppError> {
        repos::provision_job::update_status(
            pool,
            id,
            status,
            error,
            Some(&serde_json::to_string(payload).unwrap_or_else(|_| "{}".into())),
        )
        .await
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBody {
    pub talos_version: String,
    #[serde(default)]
    pub modules: Vec<String>,
    pub nodes: Vec<NodeIn>,
    /// Override the install image ref (e.g. a locally-patched image with
    /// NIC firmware baked in).
    #[serde(default)]
    pub install_image: Option<String>,
    /// Explicit control-plane endpoint for worker-only overtakes, e.g.
    /// "172.20.0.55" or "https://172.20.0.55:6443". Required when the node
    /// list contains no control-plane node (a worker-only convert); otherwise
    /// the endpoint defaults to 127.0.0.1:6443 and the workers can never join
    /// the running cluster plane.
    #[serde(default)]
    pub control_plane_endpoint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeIn {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub network: NodeNetworkCapture,
    #[serde(default)]
    pub drivers: Vec<String>,
    #[serde(default)]
    pub kubelet_cert: String,
    #[serde(default)]
    pub kubelet_key: String,
}

fn os_image_for(m: &crate::db::models::machine::Machine) -> String {
    m.os_type.clone().unwrap_or_default()
}

/// Map active kernel drivers to known Image Factory module names (best-effort
/// pre-selection; the operator confirms/edits in the wizard).
pub fn recommend_modules(drivers: &[String]) -> Vec<String> {
    let map: &[(&str, &str)] = &[
        ("bnx2x", "siderolabs/bnx2-bnx2x"),
        ("bnx2", "siderolabs/bnx2-bnx2x"),
        ("be2net", "siderolabs/be2net"),
        ("mlx5_core", "siderolabs/mlx5"),
        ("mlx5", "siderolabs/mlx5"),
        ("i40e", "siderolabs/i40e"),
        ("igb", "siderolabs/igb"),
        ("mpt3sas", "siderolabs/mpt3sas"),
        ("mpt2sas", "siderolabs/mpt2sas"),
        ("aardvark", "siderolabs/aardvark"),
        ("isci", "siderolabs/isci"),
        ("nvme", "siderolabs/nvme"),
        ("iscsi_tcp", "siderolabs/iscsi-tools"),
        ("libiscsi", "siderolabs/iscsi-tools"),
        ("nfsv4", "siderolabs/nfs-utils"),
        ("nfs", "siderolabs/nfs-utils"),
        ("fcoe", "siderolabs/fcoe"),
        ("libfc", "siderolabs/fcoe"),
    ];
    let mut out = Vec::new();
    for d in drivers {
        let dl = d.to_lowercase();
        for (drv, module) in map {
            if dl == *drv && !out.iter().any(|o: &String| o == module) {
                out.push(module.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommend_maps_known_drivers() {
        let m = recommend_modules(&["bnx2x".into(), "bonding".into(), "nfsv4".into()]);
        assert!(m.contains(&"siderolabs/bnx2-bnx2x".to_string()));
        assert!(m.contains(&"siderolabs/nfs-utils".to_string()));
        // bonding has no factory module -> not recommended
        assert!(!m.iter().any(|x| x.contains("bonding")));
    }

    #[test]
    fn recommend_dedups() {
        let m = recommend_modules(&["bnx2x".into(), "bnx2".into()]);
        assert_eq!(m.iter().filter(|x| **x == "siderolabs/bnx2-bnx2x").count(), 1);
    }

    #[test]
    fn payload_log_is_bounded() {
        let mut p = ConvertJobPayload::default();
        for i in 0..300 {
            p.log(&format!("step {i}"));
        }
        assert!(p.steps_log.len() <= 200);
    }

    #[test]
    fn normalize_dedups_and_keeps_operator_order() {
        let m = normalize_convert_modules(&[
            " siderolabs/bnx2-bnx2x ".into(),
            "siderolabs/iscsi-tools".into(),
            "siderolabs/bnx2-bnx2x".into(),
            "".into(),
        ]);
        assert_eq!(
            m,
            vec![
                "siderolabs/bnx2-bnx2x".to_string(),
                "siderolabs/iscsi-tools".to_string(),
            ]
        );
        assert!(normalize_convert_modules(&[]).is_empty());
    }

    #[test]
    fn recommend_iscsi_and_nfs_from_storage_drivers() {
        let m = recommend_modules(&["iscsi_tcp".into(), "nfs".into()]);
        assert!(m.contains(&"siderolabs/iscsi-tools".to_string()));
        assert!(m.contains(&"siderolabs/nfs-utils".to_string()));
    }

    #[test]
    fn start_body_deserializes_camelcase() {
        let b: StartBody = serde_json::from_str(
            r#"{"talosVersion":"v1.13.7","modules":["siderolabs/bnx2-bnx2x"],"nodes":[{"name":"n1","role":"control-plane"}]}"#,
        )
        .unwrap();
        assert_eq!(b.talos_version, "v1.13.7");
        assert_eq!(b.nodes.len(), 1);
        assert_eq!(b.nodes[0].role, "control-plane");
    }
}
