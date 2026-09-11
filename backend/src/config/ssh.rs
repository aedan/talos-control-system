//! SSH access to nodes for the in-place (non-Talos -> Talos) conversion path.
//!
//! TCS shells out to `ssh`/`scp` from the deployer host. The operator places
//! the trusted private key on the deployer (the nodes already accept it), so no
//! per-node credential store is needed. Env overrides: TCS_SSH_USER,
//! TCS_SSH_KEY_PATH, TCS_SSH_PORT.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SshConfig {
    /// Remote user for node commands (default root).
    pub user: String,
    /// Private key to use. Empty = OpenSSH default (~/.ssh/id_*, ssh-agent).
    pub key_path: String,
    /// Remote port (default 22).
    pub port: u16,
    /// Per-command timeout in seconds (default 60).
    pub timeout_secs: u64,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            user: "root".to_string(),
            key_path: String::new(),
            port: 22,
            timeout_secs: 60,
        }
    }
}
