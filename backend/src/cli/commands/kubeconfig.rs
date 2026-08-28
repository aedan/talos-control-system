//! `tcs kubeconfig` / `tcs talosconfig` — print the cluster's stored
//! kubeconfig/talosconfig YAML.
//!
//! The credential is fetched **over the API** using the caller's TCS token
//! (audited, JWT-scoped, time-boxed). It is never read directly from the TCS
//! database — that would let anything with DB access exfiltrate cluster
//! credentials without authentication.
//!
//! Cluster resolution: an explicit `--cluster`/`TCS_CLUSTER`/saved cluster is
//! used when present. If none is set and the account can see exactly **one**
//! cluster, that cluster is used as the default (this is what lets the
//! zero-touch tool wrappers work with bare `kubectl`/`helm`/`talosctl` on a
//! single-cluster host). If there are several and none is selected, it errors.
//!
//! Auth: if the stored token is missing or expired (HTTP 401), this command
//! performs an interactive `tcs login` once and retries — mirroring the old
//! `tcs kubectl` behavior. Unattended scripts therefore require a human to
//! have logged in recently (within the JWT TTL).

use super::client::{CliError, CliResult, Client};
use super::require_cluster;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Kubeconfig,
    Talosconfig,
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Kubeconfig => "kubeconfig",
        Kind::Talosconfig => "talosconfig",
    }
}

pub async fn run(
    kind: Kind,
    server: Option<&str>,
    token: Option<&str>,
    cluster: Option<&str>,
) -> CliResult<()> {
    let client = Client::new(server, token)?;
    let id = resolve(&client, cluster).await?;
    let path = format!("/api/clusters/{id}/{}", kind_name(kind));

    match client.get_text(&path).await {
        Ok(text) => {
            print!("{text}");
            Ok(())
        }
        Err(err) if is_auth_error(&err) => {
            eprintln!("token is missing or expired — re-authenticating…");
            super::relogin(server).await?;
            let client = Client::new(server, token)?;
            let id = resolve(&client, cluster).await?;
            let path = format!("/api/clusters/{id}/{}", kind_name(kind));
            let text = client.get_text(&path).await?;
            print!("{text}");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

/// Resolve the canonical cluster UUID for the credential verbs.
///
/// Prefers an explicitly selected cluster; otherwise, if the account can see
/// exactly one cluster, uses it as the default. Returns the canonical UUID
/// plus the server/token so the caller can rebuild a fresh client.
async fn resolve(client: &Client, cluster: Option<&str>) -> CliResult<String> {
    let want = super::super::config::CliConfig::resolve_cluster(cluster);
    match want {
        Some(want) => require_cluster(client, Some(&want)).await,
        None => single_visible_cluster(client).await,
    }
}

/// Return the single visible cluster's UUID, or an error.
async fn single_visible_cluster(client: &Client) -> CliResult<String> {
    let res = client.get_json("/api/clusters").await?;
    let rows = res.as_array().cloned().unwrap_or_default();
    match rows.len() {
        0 => Err(CliError::Other("no clusters visible to this account".into())),
        1 => rows[0]
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or(CliError::Other("cluster row missing id".into())),
        n => Err(CliError::Other(format!(
            "{n} clusters visible; pass --cluster or set TCS_CLUSTER"
        ))),
    }
}

/// True when the error is a 401 / not-authenticated condition worth relogin.
fn is_auth_error(err: &CliError) -> bool {
    match err {
        CliError::NotAuthenticated => true,
        CliError::Server(msg) => msg.starts_with("401"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::client::CliError;
    use super::is_auth_error;

    #[test]
    fn detects_401_server_error() {
        assert!(is_auth_error(&CliError::Server("401: unauthorized".into())));
    }

    #[test]
    fn not_authenticated_is_auth_error() {
        assert!(is_auth_error(&CliError::NotAuthenticated));
    }

    #[test]
    fn other_server_errors_are_not() {
        assert!(!is_auth_error(&CliError::Server("404: not found".into())));
        assert!(!is_auth_error(&CliError::Server("500: boom".into())));
    }
}
