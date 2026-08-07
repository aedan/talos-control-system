use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSet {
    pub id: Uuid,
    pub cluster_id: Uuid,
    pub name: String,
    pub role: String,
    pub size: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Default for MachineSet {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            cluster_id: Uuid::nil(),
            name: String::new(),
            role: "worker".to_string(),
            size: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}
