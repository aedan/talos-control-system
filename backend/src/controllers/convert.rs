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
use crate::integration::network_capture::{NetworkCapture, NodeNetworkCapture};
use crate::integration::ssh::SshClient;
use crate::AppError;

pub const JOB_KIND: &str = "convert";

// ── payload (persisted on the provision_job row) ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertNodePlan {
    pub name: String,
    pub role: String, // "control-plane" | "worker"
    pub address: String,
    pub network: NodeNetworkCapture,
    pub drivers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertNodeState {
    pub name: String,
    pub role: String,
    pub status: String, // pending|snapshot|kexec|install|reboot|join|recover|done|failed|skipped
    pub current_step: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConvertJobPayload {
    pub talos_version: String,
    pub modules: Vec<String>,
    pub schematic: Option<String>,
    /// Ordered conversion plan: control-plane first, then workers.
    pub nodes: Vec<ConvertNodePlan>,
    pub phase: String, // snapshot|control-plane|workers|adopt|done
    pub current_index: usize,
    pub node_states: Vec<ConvertNodeState>,
    pub etcd_snapshot_path: Option<String>,
    pub etcd_snapshot_size: i64,
    pub steps_log: Vec<String>,
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
                });
                blockers.push(format!("node {}: no address recorded", m.hostname));
                continue;
            }
            match capture.capture(&m.address).await {
                Ok(res) => {
                    let recommended = if modules.is_empty() {
                        recommend_modules(&res.drivers)
                    } else {
                        modules.clone()
                    };
                    if m.machine_type == "control-plane" || m.machine_type == "controlplane" {
                        cp_nodes.push(m.hostname.clone());
                    }
                    nodes.push(PreviewNode {
                        name: m.hostname.clone(),
                        role: m.machine_type.clone(),
                        os_type: m.os_type.clone().unwrap_or_else(|| "baremetal".into()),
                        os_image: os_image_for(m),
                        ssh_ok: true,
                        ssh_error: String::new(),
                        drivers: res.drivers.clone(),
                        recommended_modules: recommended,
                        network: res.network,
                    });
                }
                Err(e) => {
                    blockers.push(format!("node {}: ssh unreachable ({})", m.hostname, e));
                    nodes.push(PreviewNode {
                        name: m.hostname.clone(),
                        role: m.machine_type.clone(),
                        os_type: m.os_type.clone().unwrap_or_else(|| "baremetal".into()),
                        os_image: os_image_for(m),
                        ssh_ok: false,
                        ssh_error: e.to_string(),
                        drivers: vec![],
                        recommended_modules: vec![],
                        network: NodeNetworkCapture::default(),
                    });
                }
            }
        }

        let can_convert = blockers.is_empty() && !nodes.is_empty() && cp_nodes.iter().count() >= 1;

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
        let machines = repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        let mut plans = Vec::new();
        let mut by_name: std::collections::HashMap<String, &crate::db::models::machine::Machine> =
            machines.iter().map(|m| (m.hostname.clone(), m)).collect();

        // Control-plane first, then workers, preserving the requested order.
        let ordered: Vec<&crate::db::models::machine::Machine> = body
            .nodes
            .iter()
            .filter_map(|n| by_name.get(&n.name))
            .cloned()
            .collect();

        for n in &body.nodes {
            let Some(m) = by_name.get(&n.name) else { continue };
            plans.push(ConvertNodePlan {
                name: n.name.clone(),
                role: n.role.clone(),
                address: m.address.clone(),
                network: n.network.clone(),
                drivers: n.drivers.clone(),
            });
        }
        let _ = ordered; // (ordering is driven by body.nodes order)

        let schematic = if body.modules.is_empty() {
            None
        } else {
            Some(crate::integration::image_factory::ImageFactoryClient::new(&self.factory.normalized_base())
                .create_schematic(&body.modules)
                .await?)
        };

        let now = Utc::now();
        let payload = ConvertJobPayload {
            talos_version: body.talos_version,
            modules: body.modules.clone(),
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
                })
                .collect(),
            etcd_snapshot_path: None,
            etcd_snapshot_size: 0,
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
