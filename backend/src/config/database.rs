use std::fmt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseBackend {
    Sqlite,
    Postgres,
}

impl Default for DatabaseBackend {
    fn default() -> Self {
        DatabaseBackend::Sqlite
    }
}

impl fmt::Display for DatabaseBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatabaseBackend::Sqlite => write!(f, "sqlite"),
            DatabaseBackend::Postgres => write!(f, "postgres"),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub backend: DatabaseBackend,

    #[serde(default = "default_sqlite_path")]
    pub sqlite_path: String,

    #[serde(default)]
    pub postgres_url: String,

    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    #[serde(default = "default_connection_timeout")]
    pub connection_timeout: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            backend: DatabaseBackend::default(),
            sqlite_path: default_sqlite_path(),
            postgres_url: String::new(),
            max_connections: default_max_connections(),
            connection_timeout: default_connection_timeout(),
        }
    }
}

fn default_sqlite_path() -> String {
    "/var/lib/tcs/data.db".to_string()
}

fn default_max_connections() -> u32 {
    10
}

fn default_connection_timeout() -> u64 {
    30
}

impl DatabaseConfig {
    pub fn connection_string(&self) -> String {
        match self.backend {
            DatabaseBackend::Sqlite => format!("sqlite:{}", self.sqlite_path),
            DatabaseBackend::Postgres => self.postgres_url.clone(),
        }
    }
}
