//! BMC power and boot control — Redfish primary, IPMI fallback.

mod ipmi;
mod redfish;

use crate::db::models::machine::Machine;
use crate::AppError;
use serde::{Deserialize, Serialize};

pub use ipmi::IpmiClient;
pub use redfish::RedfishClient;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    On,
    Off,
    Unknown,
}

impl PowerState {
    pub fn as_str(self) -> &'static str {
        match self {
            PowerState::On => "on",
            PowerState::Off => "off",
            PowerState::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootTarget {
    Pxe,
    Disk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmcProtocol {
    Redfish,
    Ipmi,
}

impl BmcProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            BmcProtocol::Redfish => "redfish",
            BmcProtocol::Ipmi => "ipmi",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BmcCredentials {
    pub address: String,
    pub username: String,
    pub password: String,
    pub redfish_path: String,
    pub tls_insecure: bool,
    pub preferred: String,
    pub timeout_secs: u64,
    pub ipmi_interface: String,
}

impl BmcCredentials {
    pub fn from_machine(
        machine: &Machine,
        password_plain: &str,
        timeout_secs: u64,
        ipmi_interface: &str,
    ) -> Result<Self, AppError> {
        if machine.bmc_address.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "Machine has no BMC address configured".into(),
            ));
        }
        if machine.bmc_username.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "Machine has no BMC username configured".into(),
            ));
        }
        if password_plain.is_empty() {
            return Err(AppError::InvalidInput(
                "Machine has no BMC password configured".into(),
            ));
        }
        Ok(Self {
            address: machine.bmc_address.trim().to_string(),
            username: machine.bmc_username.trim().to_string(),
            password: password_plain.to_string(),
            redfish_path: machine.bmc_redfish_path.clone(),
            tls_insecure: machine.bmc_tls_insecure,
            preferred: machine.bmc_type.clone(),
            timeout_secs,
            ipmi_interface: ipmi_interface.to_string(),
        })
    }
}

/// Resolve protocol and run power/boot ops.
pub struct BmcSession {
    protocol: BmcProtocol,
    redfish: Option<RedfishClient>,
    ipmi: Option<IpmiClient>,
}

impl BmcSession {
    pub async fn connect(creds: &BmcCredentials) -> Result<Self, AppError> {
        let preferred = creds.preferred.to_ascii_lowercase();
        match preferred.as_str() {
            "redfish" => {
                let rf = RedfishClient::connect(creds).await?;
                Ok(Self {
                    protocol: BmcProtocol::Redfish,
                    redfish: Some(rf),
                    ipmi: None,
                })
            }
            "ipmi" => {
                let ip = IpmiClient::new(creds)?;
                // quick probe
                let _ = ip.get_power_state().await?;
                Ok(Self {
                    protocol: BmcProtocol::Ipmi,
                    redfish: None,
                    ipmi: Some(ip),
                })
            }
            _ => {
                // auto: Redfish first
                match RedfishClient::connect(creds).await {
                    Ok(rf) => Ok(Self {
                        protocol: BmcProtocol::Redfish,
                        redfish: Some(rf),
                        ipmi: None,
                    }),
                    Err(rf_err) => {
                        tracing::debug!(error = %rf_err, "Redfish probe failed, trying IPMI");
                        let ip = IpmiClient::new(creds)?;
                        match ip.get_power_state().await {
                            Ok(_) => Ok(Self {
                                protocol: BmcProtocol::Ipmi,
                                redfish: None,
                                ipmi: Some(ip),
                            }),
                            Err(ip_err) => Err(AppError::Network(format!(
                                "BMC auto-detect failed: redfish={rf_err}; ipmi={ip_err}"
                            ))),
                        }
                    }
                }
            }
        }
    }

    pub fn protocol(&self) -> BmcProtocol {
        self.protocol
    }

    pub async fn get_power_state(&self) -> Result<PowerState, AppError> {
        if let Some(rf) = &self.redfish {
            return rf.get_power_state().await;
        }
        if let Some(ip) = &self.ipmi {
            return ip.get_power_state().await;
        }
        Err(AppError::Internal("No BMC client".into()))
    }

    pub async fn power(&self, action: &str) -> Result<(), AppError> {
        if let Some(rf) = &self.redfish {
            return rf.power(action).await;
        }
        if let Some(ip) = &self.ipmi {
            return ip.power(action).await;
        }
        Err(AppError::Internal("No BMC client".into()))
    }

    pub async fn set_boot(&self, target: BootTarget, once: bool) -> Result<(), AppError> {
        if let Some(rf) = &self.redfish {
            return rf.set_boot(target, once).await;
        }
        if let Some(ip) = &self.ipmi {
            return ip.set_boot(target, once).await;
        }
        Err(AppError::Internal("No BMC client".into()))
    }

    pub async fn mount_iso(&self, iso_url: &str, media: &str) -> Result<(), AppError> {
        if let Some(rf) = &self.redfish {
            return rf.mount_iso(iso_url, media).await;
        }
        Err(AppError::Internal("ISO mount only supported via Redfish".into()))
    }

    pub async fn unmount_iso(&self, media: &str) -> Result<(), AppError> {
        if let Some(rf) = &self.redfish {
            return rf.unmount_iso(media).await;
        }
        Err(AppError::Internal("ISO unmount only supported via Redfish".into()))
    }
}

/// Unified BMC control surface: either a direct on-network session or a relay
/// through a connected remote OOB agent. Call sites use this instead of
/// `BmcSession` so proxied machines are transparent.
pub enum BmcOps {
    Direct(BmcSession),
    Proxied {
        tunnel: std::sync::Arc<crate::network::tunnel::TunnelRegistry>,
        agent_id: String,
        creds: BmcCredentials,
    },
}

impl BmcOps {
    pub async fn power(&self, action: &str) -> Result<(), AppError> {
        match self {
            BmcOps::Direct(s) => s.power(action).await,
            BmcOps::Proxied { tunnel, agent_id, creds } => {
                tunnel.proxy_power(agent_id, creds, action).await
            }
        }
    }

    pub async fn set_boot(&self, target: BootTarget, once: bool) -> Result<(), AppError> {
        match self {
            BmcOps::Direct(s) => s.set_boot(target, once).await,
            BmcOps::Proxied { tunnel, agent_id, creds } => {
                tunnel.proxy_set_boot(agent_id, creds, target, once).await
            }
        }
    }

    pub async fn get_power_state(&self) -> Result<PowerState, AppError> {
        match self {
            BmcOps::Direct(s) => s.get_power_state().await,
            BmcOps::Proxied { tunnel, agent_id, creds } => {
                tunnel.proxy_get_power_state(agent_id, creds).await
            }
        }
    }

    pub async fn mount_iso(&self, iso_url: &str, media: &str) -> Result<(), AppError> {
        match self {
            BmcOps::Direct(s) => s.mount_iso(iso_url, media).await,
            BmcOps::Proxied { tunnel, agent_id, creds } => {
                tunnel.proxy_mount_iso(agent_id, creds, iso_url, media).await
            }
        }
    }

    pub async fn unmount_iso(&self, media: &str) -> Result<(), AppError> {
        match self {
            BmcOps::Direct(s) => s.unmount_iso(media).await,
            BmcOps::Proxied { tunnel, agent_id, creds } => {
                tunnel.proxy_unmount_iso(agent_id, creds, media).await
            }
        }
    }
}

/// Open BMC control for a machine, routing through its remote OOB agent when
/// `proxy_id` is set and the agent is connected, otherwise directly.
pub async fn open_bmc_ops(
    machine: &Machine,
    password_plain: &str,
    timeout_secs: u64,
    ipmi_interface: &str,
    tunnel: &std::sync::Arc<crate::network::tunnel::TunnelRegistry>,
) -> Result<BmcOps, AppError> {
    if let Some(agent_id) = machine
        .proxy_id
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        if tunnel.is_online(&agent_id) {
            let creds = BmcCredentials::from_machine(
                machine,
                password_plain,
                timeout_secs,
                ipmi_interface,
            )?;
            return Ok(BmcOps::Proxied {
                tunnel: std::sync::Arc::clone(tunnel),
                agent_id,
                creds,
            });
        }
        return Err(AppError::Network(format!(
            "OOB agent '{agent_id}' is not connected"
        )));
    }
    let creds = BmcCredentials::from_machine(machine, password_plain, timeout_secs, ipmi_interface)?;
    let sess = BmcSession::connect(&creds).await?;
    Ok(BmcOps::Direct(sess))
}
