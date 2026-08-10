//! BMC power and boot control — Redfish primary, IPMI fallback.

mod ipmi;
mod redfish;

use crate::db::models::machine::Machine;
use crate::AppError;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
