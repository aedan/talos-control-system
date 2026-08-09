use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ConfigPatch {
    #[sqlx(rename = "id")]
    pub id: Uuid,
    #[sqlx(rename = "cluster_id")]
    pub cluster_id: Uuid,
    #[sqlx(rename = "machine_id")]
    pub machine_id: Option<Uuid>,
    #[sqlx(rename = "path")]
    pub path: String,
    #[sqlx(rename = "value")]
    pub value: String,
    #[sqlx(rename = "priority")]
    pub priority: i32,
    #[sqlx(rename = "created_at")]
    pub created_at: DateTime<Utc>,
    #[sqlx(rename = "updated_at")]
    pub updated_at: DateTime<Utc>,
}

impl ConfigPatch {
    pub fn new(cluster_id: Uuid, path: String, value: String, priority: i32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            cluster_id,
            machine_id: None,
            path,
            value,
            priority,
            created_at: now,
            updated_at: now,
        }
    }
}
