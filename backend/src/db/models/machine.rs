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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn update_status(&mut self, status: &str) {
        self.status = status.to_string();
        self.updated_at = Utc::now();
    }
}
