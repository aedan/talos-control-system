//! `tcs kubeconfig` / `tcs talosconfig` — print the cluster's stored
//! kubeconfig/talosconfig YAML.
//!
//! The credential is fetched **over the API** using the caller's TCS token
//! (audited, JWT-scoped, time-boxed). It is never read directly from the TCS
//! database — that would let anything with DB access exfiltrate cluster
//! credentials without authentication.
//!
//! If the stored token is missing or has expired (HTTP 401), this command
//! performs an interactive `tcs login` once and retries, mirroring the old
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
    let id = require_cluster(&client, cluster).await?;
    let path = format!("/api/clusters/{id}/{}", kind_name(kind));

    match client.get_text(&path).await {
        Ok(text) => {
            print!("{text}");
            Ok(())
        }
        Err(err) if is_auth_error(&err) => {
            eprintln!(
                "{} token is missing or expired — re-authenticating…",
                kind_name(kind)
            );
            super::relogin(server).await?;
            // Re-resolve server/token after login rewrote the config.
            let client = Client::new(server, token)?;
            let id = require_cluster(&client, cluster).await?;
            let path = format!("/api/clusters/{id}/{}", kind_name(kind));
            let text = client.get_text(&path).await?;
            print!("{text}");
            Ok(())
        }
        Err(err) => Err(err),
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
    use super::is_auth_error;
    use super::super::client::CliError;

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
