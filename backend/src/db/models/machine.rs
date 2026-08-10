use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    #[sqlx(rename = "id")]
    pub id: Uuid,
    #[sqlx(rename = "system_uuid")]
    pub system_uuid: String,
    #[sqlx(rename = "machine_type")]
    pub machine_type: String,
    #[sqlx(rename = "cluster_id")]
    pub cluster_id: Option<Uuid>,
    #[sqlx(rename = "status")]
    pub status: String,
    #[sqlx(rename = "talos_version")]
    pub talos_version: String,
    #[sqlx(rename = "secure_boot")]
    pub secure_boot: bool,
    #[sqlx(rename = "siderolink_connected")]
    pub siderolink_connected: bool,
    #[sqlx(rename = "address")]
    pub address: String,
    #[sqlx(rename = "install_disk")]
    pub install_disk: String,
    #[sqlx(rename = "mac_address")]
    pub mac_address: String,
    #[sqlx(rename = "hostname")]
    pub hostname: String,
    #[sqlx(rename = "bmc_address")]
    pub bmc_address: String,
    #[sqlx(rename = "bmc_username")]
    pub bmc_username: String,
    /// Encrypted BMC password; never serialize to API.
    #[sqlx(rename = "bmc_password_enc")]
    #[serde(skip_serializing)]
    pub bmc_password_enc: Option<String>,
    #[sqlx(rename = "bmc_type")]
    pub bmc_type: String,
    #[sqlx(rename = "bmc_redfish_path")]
    pub bmc_redfish_path: String,
    #[sqlx(rename = "bmc_tls_insecure")]
    pub bmc_tls_insecure: bool,
    #[sqlx(rename = "pxe_profile_id")]
    pub pxe_profile_id: Option<String>,
    #[sqlx(rename = "last_power_state")]
    pub last_power_state: String,
    #[sqlx(rename = "last_seen_at")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[sqlx(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    #[sqlx(rename = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

impl Machine {
    pub fn new(system_uuid: String, machine_type: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            system_uuid,
            machine_type,
            cluster_id: None,
            status: "pending".to_string(),
            talos_version: String::new(),
            secure_boot: false,
            siderolink_connected: false,
            address: String::new(),
            install_disk: String::new(),
            mac_address: String::new(),
            hostname: String::new(),
            bmc_address: String::new(),
            bmc_username: String::new(),
            bmc_password_enc: None,
            bmc_type: "auto".to_string(),
            bmc_redfish_path: String::new(),
            bmc_tls_insecure: true,
            pxe_profile_id: None,
            last_power_state: "unknown".to_string(),
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn update_status(&mut self, status: &str) {
        self.status = status.to_string();
        self.updated_at = Utc::now();
    }

    pub fn has_bmc(&self) -> bool {
        !self.bmc_address.trim().is_empty()
            && !self.bmc_username.trim().is_empty()
            && self
                .bmc_password_enc
                .as_ref()
                .map(|p| !p.is_empty())
                .unwrap_or(false)
    }
}
