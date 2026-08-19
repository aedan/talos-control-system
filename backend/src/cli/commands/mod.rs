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
pub mod logs;
pub mod scale;

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
pub fn require_cluster(client: &Client, cluster_flag: Option<&str>) -> super::client::CliResult<String> {
    let _ = client;
    let c = super::config::CliConfig::resolve_cluster(cluster_flag)
        .ok_or(super::client::CliError::NoCluster)?;
    Ok(c)
}
