use chrono::{DateTime, Utc};
use uuid::Uuid;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

impl Default for User {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            email: String::new(),
            display_name: String::new(),
            role: "reader".to_string(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserRole {
    Admin,
    Operator,
    Reader,
}

impl Default for UserRole {
    fn default() -> Self {
        UserRole::Reader
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::Admin => write!(f, "admin"),
            UserRole::Operator => write!(f, "operator"),
            UserRole::Reader => write!(f, "reader"),
        }
    }
}
