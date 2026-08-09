use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ClusterBackup {
    #[sqlx(rename = "id")]
    pub id: Uuid,
    #[sqlx(rename = "cluster_id")]
    pub cluster_id: Uuid,
    #[sqlx(rename = "name")]
    pub name: String,
    #[sqlx(rename = "status")]
    pub status: String,
    #[sqlx(rename = "file_path")]
    pub file_path: Option<String>,
    #[sqlx(rename = "size_bytes")]
    pub size_bytes: i64,
    #[sqlx(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    #[sqlx(rename = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

impl ClusterBackup {
    pub fn new(cluster_id: Uuid, name: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            name,
            status: "completed".to_string(),
            file_path: Some(format!("/backups/{}/{}.snapshot", cluster_id, Uuid::new_v4())),
            size_bytes: 0,
            created_at: now,
            updated_at: now,
        }
    }
}
