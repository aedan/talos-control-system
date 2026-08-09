use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineClass {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub min_cpu: i32,
    pub min_memory: i64,
    pub min_disk: i64,
    pub arch: String,
    pub secure_boot: bool,
    pub allowed_roles: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MachineClass {
    pub fn new(
        name: String,
        description: String,
        min_cpu: i32,
        min_memory: i64,
        min_disk: i64,
        arch: String,
        secure_boot: bool,
        allowed_roles: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description,
            min_cpu,
            min_memory,
            min_disk,
            arch,
            secure_boot,
            allowed_roles,
            created_at: now,
            updated_at: now,
        }
    }
}

use sqlx::FromRow;

#[derive(Debug, FromRow)]
pub struct MachineClassRow {
    id: Uuid,
    name: String,
    description: String,
    min_cpu: i32,
    min_memory: i64,
    min_disk: i64,
    arch: String,
    secure_boot: bool,
    allowed_roles: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<MachineClassRow> for MachineClass {
    fn from(row: MachineClassRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            min_cpu: row.min_cpu,
            min_memory: row.min_memory,
            min_disk: row.min_disk,
            arch: row.arch,
            secure_boot: row.secure_boot,
            allowed_roles: serde_json::from_str(&row.allowed_roles).unwrap_or_default(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
