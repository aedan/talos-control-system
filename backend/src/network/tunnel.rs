//! Remote OOB proxy tunnel registry.
//!
//! A small native agent on the operator's remote desktop dials OUT to TCS over
//! a WebSocket (`GET /api/proxy/tunnel?token=...`), authenticated by a
//! pre-shared join token. TCS is a pure relay: BMC operations are framed as
//! JSON and sent to the agent, which executes them against the local Redfish
//! endpoint and returns the result. This module owns the in-memory map of
//! connected agents and the request/response correlation.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::integration::bmc::{BmcCredentials, BootTarget, PowerState};
use crate::AppError;

/// Default per-operation timeout for a proxied BMC call.
pub const OP_TIMEOUT: Duration = Duration::from_secs(60);

/// A single BMC operation to relay to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BmcOp {
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iso_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
    pub creds: BmcCredentials,
}

/// Result of a proxied BMC operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BmcOpResult {
    pub op_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_state: Option<String>,
}

struct AgentSlot {
    tx: mpsc::Sender<String>,
    pending: HashMap<String, oneshot::Sender<BmcOpResult>>,
    caps: Vec<String>,
    label: Option<String>,
    connected_at: Instant,
}

/// In-memory registry of connected OOB agents, keyed by agent id (the token
/// suffix). Cloned `Arc<TunnelRegistry>` is shared across handlers and the
/// metal scheduler.
#[derive(Default)]
pub struct TunnelRegistry {
    agents: DashMap<String, AgentSlot>,
}

impl TunnelRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
        }
    }

    /// Register (or replace) a connected agent. Returns the (sender, receiver)
    /// pair: the sender is stored for `send_op` and the receiver is owned by
    /// the agent's WebSocket pump task to push framed JSON out.
    pub fn upsert(
        &self,
        agent_id: &str,
        caps: Vec<String>,
        label: Option<String>,
    ) -> (mpsc::Sender<String>, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel::<String>(64);
        self.agents.insert(
            agent_id.to_string(),
            AgentSlot {
                tx: tx.clone(),
                pending: HashMap::new(),
                caps,
                label,
                connected_at: Instant::now(),
            },
        );
        (tx, rx)
    }

    /// Remove an agent and fail any in-flight operations.
    pub fn disconnect(&self, agent_id: &str) {
        if let Some((_key, mut slot)) = self.agents.remove(agent_id) {
            for (_op_id, rx) in slot.pending.drain() {
                let _ = rx.send(BmcOpResult {
                    op_id: String::new(),
                    ok: false,
                    error: Some("agent disconnected".into()),
                    power_state: None,
                });
            }
        }
    }

    pub fn is_online(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    /// Deliver an agent's response, resolving the matching pending operation.
    pub fn deliver(&self, agent_id: &str, result: BmcOpResult) {
        if let Some(mut slot) = self.agents.get_mut(agent_id) {
            if let Some(rx) = slot.pending.remove(&result.op_id) {
                let _ = rx.send(result);
            }
        }
    }

    /// Send an operation with an explicit timeout.
    async fn send_op_timeout(
        &self,
        agent_id: &str,
        op: BmcOp,
        timeout: Duration,
    ) -> Result<BmcOpResult, AppError> {
        let (op_id, rx) = {
            let mut slot = self
                .agents
                .get_mut(agent_id)
                .ok_or_else(|| AppError::Network(format!("OOB agent '{agent_id}' is not connected")))?;
            let op_id = uuid::Uuid::new_v4().simple().to_string();
            let (tx, rx) = oneshot::channel::<BmcOpResult>();
            slot.pending.insert(op_id.clone(), tx);
            (op_id, rx)
        };
        let frame = serde_json::json!({ "type": "bmc.op", "opId": op_id, "op": op });
        let frame = frame.to_string();
        let tx = self
            .agents
            .get(agent_id)
            .map(|s| s.tx.clone())
            .ok_or_else(|| AppError::Network(format!("OOB agent '{agent_id}' is not connected")))?;
        if tx.send(frame).await.is_err() {
            let _ = self.agents.get_mut(agent_id).map(|mut s| s.pending.remove(&op_id));
            return Err(AppError::Network(format!(
                "OOB agent '{agent_id}' connection closed"
            )));
        }
        let result = tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| AppError::Network(format!("OOB op timed out after {timeout:?}")))?
            .map_err(|_| AppError::Network("OOB agent dropped before responding".into()))?;
        if !result.ok {
            return Err(AppError::Network(
                result.error.unwrap_or_else(|| "OOB agent reported failure".into()),
            ));
        }
        Ok(result)
    }

    /// Snapshot of connected agents for the admin UI.
    pub fn online_list(&self) -> Vec<serde_json::Value> {
        self.agents
            .iter()
            .map(|e| {
                serde_json::json!({
                    "agentId": e.key(),
                    "label": e.value().label,
                    "caps": e.value().caps,
                    "connectedForSecs": e.value().connected_at.elapsed().as_secs(),
                })
            })
            .collect()
    }
}

// ─── High-level proxied BMC operations ────────────────────────────────────

impl TunnelRegistry {
    fn op_timeout(creds: &BmcCredentials) -> Duration {
        Duration::from_secs(creds.timeout_secs.max(1)).max(OP_TIMEOUT)
    }

    async fn power_state_of(&self, agent_id: &str, creds: &BmcCredentials) -> Result<PowerState, AppError> {
        let r = self
            .send_op_timeout(
                agent_id,
                BmcOp {
                    op: "get_power_state".into(),
                    action: None,
                    target: None,
                    once: None,
                    iso_url: None,
                    media: None,
                    creds: creds.clone(),
                },
                Self::op_timeout(creds),
            )
            .await?;
        Ok(match r.power_state.as_deref() {
            Some("on") => PowerState::On,
            Some("off") => PowerState::Off,
            _ => PowerState::Unknown,
        })
    }

    pub async fn proxy_power(
        &self,
        agent_id: &str,
        creds: &BmcCredentials,
        action: &str,
    ) -> Result<(), AppError> {
        self.send_op_timeout(
            agent_id,
            BmcOp {
                op: "power".into(),
                action: Some(action.into()),
                target: None,
                once: None,
                iso_url: None,
                media: None,
                creds: creds.clone(),
            },
            Self::op_timeout(creds),
        )
        .await?;
        Ok(())
    }

    pub async fn proxy_set_boot(
        &self,
        agent_id: &str,
        creds: &BmcCredentials,
        target: BootTarget,
        once: bool,
    ) -> Result<(), AppError> {
        self.send_op_timeout(
            agent_id,
            BmcOp {
                op: "set_boot".into(),
                action: None,
                target: Some(match target {
                    BootTarget::Pxe => "pxe".into(),
                    BootTarget::Disk => "disk".into(),
                }),
                once: Some(once),
                iso_url: None,
                media: None,
                creds: creds.clone(),
            },
            Self::op_timeout(creds),
        )
        .await?;
        Ok(())
    }

    pub async fn proxy_get_power_state(
        &self,
        agent_id: &str,
        creds: &BmcCredentials,
    ) -> Result<PowerState, AppError> {
        self.power_state_of(agent_id, creds).await
    }

    pub async fn proxy_mount_iso(
        &self,
        agent_id: &str,
        creds: &BmcCredentials,
        iso_url: &str,
        media: &str,
    ) -> Result<(), AppError> {
        self.send_op_timeout(
            agent_id,
            BmcOp {
                op: "mount_iso".into(),
                action: None,
                target: None,
                once: None,
                iso_url: Some(iso_url.into()),
                media: Some(media.into()),
                creds: creds.clone(),
            },
            Self::op_timeout(creds),
        )
        .await?;
        Ok(())
    }

    pub async fn proxy_unmount_iso(
        &self,
        agent_id: &str,
        creds: &BmcCredentials,
        media: &str,
    ) -> Result<(), AppError> {
        self.send_op_timeout(
            agent_id,
            BmcOp {
                op: "unmount_iso".into(),
                action: None,
                target: None,
                once: None,
                iso_url: None,
                media: Some(media.into()),
                creds: creds.clone(),
            },
            Self::op_timeout(creds),
        )
        .await?;
        Ok(())
    }
}

/// Derive a stable agent id from a join token: the hex suffix after `pxj_`.
pub fn agent_id_from_token(token: &str) -> String {
    token
        .strip_prefix("pxj_")
        .unwrap_or(token)
        .to_ascii_lowercase()
}

/// Clone-friendly handle so the registry can be shared cheaply.
pub type TunnelHandle = Arc<TunnelRegistry>;
