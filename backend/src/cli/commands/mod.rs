//! `tcs` CLI commands. Each command is a thin client over the TCS REST API.
//!
//! Wave 1 agents (C1/C2/C3) fill in the real logic; these stubs compile and
//! establish the shared entry-point contract:
//!
//!   `pub async fn run(client: &Client, cluster: &str, args: &Args) -> CliResult<()>`

pub mod apply;
pub mod attach;
pub mod cordon;
pub mod delete;
pub mod describe;
pub mod drain;
pub mod exec;
pub mod get;
pub mod kubeconfig;
pub mod logs;
pub mod scale;
pub mod tool;

use super::client::Client;
use super::output::Format;

// Re-export so command files can refer to `super::client` / `super::output`.
pub use super::client;
pub use super::output;

/// Common flags shared by most read commands.
#[derive(Debug, Clone)]
pub struct CommonArgs {
    pub format: Format,
    pub all_namespaces: bool,
}

/// Resolve the target cluster id from `--cluster`/env/config, erroring if absent.
///
/// Accepts a full UUID, a unique UUID prefix, or a cluster name, and returns the
/// canonical UUID (the server routes are keyed by UUID).
pub async fn require_cluster(client: &Client, cluster_flag: Option<&str>) -> super::client::CliResult<String> {
    let want = super::config::CliConfig::resolve_cluster(cluster_flag)
        .ok_or(super::client::CliError::NoCluster)?;

    // Fast path: already a full UUID.
    if let Ok(uuid) = uuid::Uuid::parse_str(&want) {
        return Ok(uuid.to_string());
    }

    // Otherwise, look it up by name or unique UUID prefix.
    let res = client.get_json("/api/clusters").await?;
    let rows = res
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut matches: Vec<(String, String)> = Vec::new();
    for c in &rows {
        let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if id == want || name == want {
            matches.push((id, name));
        } else if id.starts_with(&want) {
            matches.push((id, name));
        }
    }

    match matches.len() {
        0 => Err(super::client::CliError::Other(format!(
            "no cluster matches '{want}' (try `tcs clusters`)"
        ))),
        1 => Ok(matches.remove(0).0),
        _ => Err(super::client::CliError::Other(format!(
            "cluster '{want}' is ambiguous: {}",
            matches
                .iter()
                .map(|(id, name)| format!("{name} ({id})"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}
