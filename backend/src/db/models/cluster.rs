use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
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
    /// Encrypted talosconfig YAML. Never serialize to API as-is.
    #[serde(skip_serializing)]
    #[sqlx(rename = "talosconfig")]
    pub talosconfig: Option<String>,
    /// Encrypted kubeconfig YAML.
    #[serde(skip_serializing)]
    #[sqlx(rename = "kubeconfig")]
    pub kubeconfig: Option<String>,
    #[sqlx(rename = "backup_retention")]
    pub backup_retention: Option<i32>,
    /// If set (hours > 0), automatic etcd snapshots on this interval.
    #[sqlx(rename = "backup_schedule_hours")]
    pub backup_schedule_hours: Option<i32>,
    #[sqlx(rename = "last_auto_backup_at")]
    pub last_auto_backup_at: Option<DateTime<Utc>>,
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
            talosconfig: None,
            kubeconfig: None,
            backup_retention: None,
            backup_schedule_hours: None,
            last_auto_backup_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    pub fn has_talos_credentials(&self) -> bool {
        self.talosconfig
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn has_kubeconfig(&self) -> bool {
        self.kubeconfig
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    pub fn update_status(&mut self, status: &str) {
        self.status = status.to_string();
        self.updated_at = Utc::now();
    }
}
