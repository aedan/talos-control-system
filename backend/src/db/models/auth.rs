use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub password_hash: Option<String>,
    pub auth_provider: String,
    pub ldap_dn: Option<String>,
    pub password_needs_change: bool,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Default for User {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            email: String::new(),
            display_name: String::new(),
            role: "reader".to_string(),
            is_active: true,
            password_hash: None,
            auth_provider: "local".to_string(),
            ldap_dn: None,
            password_needs_change: false,
            last_login: None,
            created_at: now,
            updated_at: now,
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
