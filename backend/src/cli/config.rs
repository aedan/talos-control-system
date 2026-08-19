//! CLI configuration: `~/.tcs/config` (JSON) + env + flag precedence.
//!
//! Precedence (highest first):
//!   * `--token` / `--server` / `--cluster` flags
//!   * `TCS_TOKEN` / `TCS_SERVER` / `TCS_CLUSTER` env vars
//!   * values stored in `~/.tcs/config`

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
}

impl CliConfig {
    /// Path to the config file (`$TCS_CONFIG` or `~/.tcs/config`).
    pub fn path() -> PathBuf {
        if let Ok(p) = std::env::var("TCS_CONFIG") {
            return PathBuf::from(p);
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".tcs").join("config")
    }

    pub fn load() -> Self {
        let p = Self::path();
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        // Config may contain a bearer token; keep it owner-only.
        std::fs::write(&p, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Resolve the server URL (flags > env > config > default).
    pub fn resolve_server(flag: Option<&str>) -> String {
        flag.map(|s| s.to_string())
            .or_else(|| std::env::var("TCS_SERVER").ok())
            .or_else(|| Self::load().server)
            .unwrap_or_else(|| "http://localhost:8081".to_string())
    }

    /// Resolve the bearer token (flags > env > config).
    pub fn resolve_token(flag: Option<&str>) -> Option<String> {
        flag.map(|s| s.to_string())
            .or_else(|| std::env::var("TCS_TOKEN").ok())
            .or_else(|| Self::load().token)
    }

    /// Resolve the default cluster id (flags > env > config).
    pub fn resolve_cluster(flag: Option<&str>) -> Option<String> {
        flag.map(|s| s.to_string())
            .or_else(|| std::env::var("TCS_CLUSTER").ok())
            .or_else(|| Self::load().cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn save_load_round_trip() {
        let _g = ENV_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("tcs-test-{}.json", std::process::id()));
        std::env::set_var("TCS_CONFIG", &path);
        let cfg = CliConfig {
            server: Some("http://x".into()),
            token: Some("tok".into()),
            cluster: Some("c1".into()),
        };
        cfg.save().unwrap();
        let loaded = CliConfig::load();
        assert_eq!(loaded.server.as_deref(), Some("http://x"));
        assert_eq!(loaded.token.as_deref(), Some("tok"));
        assert_eq!(loaded.cluster.as_deref(), Some("c1"));
        let _ = std::fs::remove_file(&path);
        std::env::remove_var("TCS_CONFIG");
    }

    #[test]
    fn resolve_server_flag_wins() {
        assert_eq!(CliConfig::resolve_server(Some("http://flag")), "http://flag");
    }

    #[test]
    fn resolve_server_from_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("tcs-test-env-{}.json", std::process::id()));
        std::env::set_var("TCS_CONFIG", &path);
        std::env::set_var("TCS_SERVER", "http://env");
        let got = CliConfig::resolve_server(None);
        std::env::remove_var("TCS_SERVER");
        std::env::remove_var("TCS_CONFIG");
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, "http://env");
    }
}
