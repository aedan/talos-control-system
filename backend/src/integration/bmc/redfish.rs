//! Redfish BMC client (HTTPS + basic auth).

use reqwest::header::CONTENT_TYPE;
use serde_json::Value;
use std::time::Duration;

use super::{BootTarget, BmcCredentials, PowerState};
use crate::AppError;

pub struct RedfishClient {
    http: reqwest::Client,
    base: String,
    system_path: String,
    username: String,
    password: String,
}

impl RedfishClient {
    pub async fn connect(creds: &BmcCredentials) -> Result<Self, AppError> {
        let host = creds
            .address
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let base = format!("https://{host}");

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(creds.timeout_secs.max(5)))
            .danger_accept_invalid_certs(creds.tls_insecure)
            .user_agent("tcs-bmc/0.3")
            .build()
            .map_err(|e| AppError::Network(format!("Redfish client build: {e}")))?;

        let systems_url = format!("{base}/redfish/v1/Systems");
        let resp = http
            .get(&systems_url)
            .basic_auth(&creds.username, Some(&creds.password))
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish Systems: {e}")))?;

        if !resp.status().is_success() {
            return Err(AppError::Network(format!(
                "Redfish Systems HTTP {}",
                resp.status()
            )));
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Network(format!("Redfish Systems JSON: {e}")))?;

        let system_path = if !creds.redfish_path.trim().is_empty() {
            let p = creds.redfish_path.trim();
            if p.starts_with('/') {
                p.to_string()
            } else {
                format!("/{p}")
            }
        } else {
            body.get("Members")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first())
                .and_then(|m| m.get("@odata.id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| AppError::Network("Redfish: no Systems members found".into()))?
        };

        Ok(Self {
            http,
            base,
            system_path,
            username: creds.username.clone(),
            password: creds.password.clone(),
        })
    }

    fn system_url(&self) -> String {
        if self.system_path.starts_with("http") {
            self.system_path.clone()
        } else {
            format!("{}{}", self.base, self.system_path)
        }
    }

    pub async fn get_power_state(&self) -> Result<PowerState, AppError> {
        let resp = self
            .http
            .get(self.system_url())
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish get system: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Network(format!(
                "Redfish get system HTTP {}",
                resp.status()
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Network(format!("Redfish system JSON: {e}")))?;
        let state = body
            .get("PowerState")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        Ok(match state {
            "On" => PowerState::On,
            "Off" => PowerState::Off,
            _ => PowerState::Unknown,
        })
    }

    pub async fn power(&self, action: &str) -> Result<(), AppError> {
        let reset_type = match action {
            "on" => "On",
            "off" => "ForceOff",
            "reset" | "cycle" => "ForceRestart",
            "graceful_shutdown" => "GracefulShutdown",
            other => {
                return Err(AppError::InvalidInput(format!(
                    "Unknown power action: {other}"
                )))
            }
        };
        let url = format!("{}/Actions/ComputerSystem.Reset", self.system_url());
        let body = serde_json::json!({ "ResetType": reset_type });
        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish power: {e}")))?;
        let code = resp.status().as_u16();
        if resp.status().is_success() || code == 204 || code == 202 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(AppError::Network(format!(
            "Redfish power HTTP {code}: {text}"
        )))
    }

    pub async fn mount_iso(&self, iso_url: &str, media: &str) -> Result<(), AppError> {
        let vmedia_path = format!("{}/VirtualMedia", self.system_url());
        let resp = self
            .http
            .get(&vmedia_path)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish get VirtualMedia: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Network(format!(
                "Redfish get VirtualMedia HTTP {}",
                resp.status()
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Network(format!("Redfish VirtualMedia JSON: {e}")))?;
        let members = body.get("Members")
            .and_then(|m| m.as_array())
            .ok_or_else(|| AppError::Network("No VirtualMedia members found".into()))?;
        let mut target_member: Option<&Value> = None;
        let mut target_odata: Option<String> = None;
        for m in members {
            let name = m.get("Name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let odata = m.get("@odata.id")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            if name.to_lowercase() == media.to_lowercase() {
                target_member = Some(m);
                target_odata = Some(odata.to_string());
                break;
            }
        }
        if target_member.is_none() {
            return Err(AppError::Network(format!(
                "VirtualMedia '{media}' not found. Available: {:?}",
                members.iter().filter_map(|m| m.get("Name").and_then(|n| n.as_str())).collect::<Vec<_>>()
            )));
        }
        let detail_path = target_odata.unwrap();
        let insert_url = format!("{}/VirtualMedia.InsertMedia",
            if detail_path.starts_with("http") {
                detail_path
            } else {
                format!("{}{}", self.base, detail_path)
            }
        );
        let body = serde_json::json!({
            "Image": iso_url,
            "TransferMethod": "Download",
            "TransferProtocolType": "HTTP"
        });
        let resp = self
            .http
            .post(&insert_url)
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish mount ISO: {e}")))?;
        let code = resp.status().as_u16();
        if resp.status().is_success() || code == 204 || code == 202 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(AppError::Network(format!(
            "Redfish mount ISO HTTP {code}: {text}"
        )))
    }

    pub async fn unmount_iso(&self, media: &str) -> Result<(), AppError> {
        let vmedia_path = format!("{}/VirtualMedia", self.system_url());
        let resp = self
            .http
            .get(&vmedia_path)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish get VirtualMedia: {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::Network(format!(
                "Redfish get VirtualMedia HTTP {}",
                resp.status()
            )));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Network(format!("Redfish VirtualMedia JSON: {e}")))?;
        let members = body.get("Members")
            .and_then(|m| m.as_array())
            .ok_or_else(|| AppError::Network("No VirtualMedia members found".into()))?;
        let mut target_odata: Option<String> = None;
        for m in members {
            let name = m.get("Name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let odata = m.get("@odata.id")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            if name.to_lowercase() == media.to_lowercase() {
                target_odata = Some(odata.to_string());
                break;
            }
        }
        let detail_path = target_odata.ok_or_else(|| AppError::Network(format!(
            "VirtualMedia '{media}' not found"
        )))?;
        let eject_url = format!("{}/VirtualMedia.EjectMedia",
            if detail_path.starts_with("http") {
                detail_path
            } else {
                format!("{}{}", self.base, detail_path)
            }
        );
        let resp = self
            .http
            .post(&eject_url)
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, "application/json")
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish unmount ISO: {e}")))?;
        let code = resp.status().as_u16();
        if resp.status().is_success() || code == 204 || code == 202 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(AppError::Network(format!(
            "Redfish unmount ISO HTTP {code}: {text}"
        )))
    }

    pub async fn set_boot(&self, target: BootTarget, once: bool) -> Result<(), AppError> {
        let boot_target = match target {
            BootTarget::Pxe => "Pxe",
            BootTarget::Disk => "Hdd",
        };
        let enabled = if once { "Once" } else { "Continuous" };
        let body = serde_json::json!({
            "BootSourceOverrideTarget": boot_target,
            "BootSourceOverrideEnabled": enabled,
            "BootSourceOverrideMode": "Legacy"
        });
        let resp = self
            .http
            .patch(self.system_url())
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, "application/json")
            .header("OData-Version", "4.0")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("Redfish set boot: {e}")))?;
        let code = resp.status().as_u16();
        if resp.status().is_success() || code == 204 {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        Err(AppError::Network(format!(
            "Redfish set boot HTTP {code}: {text}"
        )))
    }
}
