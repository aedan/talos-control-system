use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Cluster {
    #[sqlx(rename = "id")]
    pub id: Uuid,
    #[sqlx(rename = "name")]
    pub name: String,
    #[sqlx(rename = "control_plane_version")]
    pub control_plane_version: String,
    #[sqlx(rename = "talos_version")]
    pub talos_version: String,
    #[sqlx(rename = "status")]
    pub status: String,
    #[sqlx(rename = "control_plane_size")]
    pub control_plane_size: i32,
    #[sqlx(rename = "worker_size")]
    pub worker_size: i32,
    #[sqlx(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    #[sqlx(rename = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

impl Cluster {
    pub fn new(name: String, control_plane_version: String, talos_version: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            control_plane_version,
            talos_version,
            status: "unknown".to_string(),
            control_plane_size: 1,
            worker_size: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn update_status(&mut self, status: &str) {
        self.status = status.to_string();
        self.updated_at = Utc::now();
    }
}