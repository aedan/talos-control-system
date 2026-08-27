//! `tcs kubeconfig` / `tcs talosconfig` — print the cluster's stored
//! kubeconfig/talosconfig YAML.
//!
//! Two modes:
//!   * `--local`: read the TCS database directly on this host. No token, no
//!     HTTP — this is what the zero-touch tool wrappers use on the TCS box.
//!   * default: fetch over the API with the caller's token (remote use).

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

pub struct Args {
    pub local: bool,
}

pub async fn run(
    kind: Kind,
    args: &Args,
    server: Option<&str>,
    token: Option<&str>,
    cluster: Option<&str>,
) -> CliResult<()> {
    if args.local {
        return local_dump(kind, cluster).await;
    }
    let client = Client::new(server, token)?;
    let id = require_cluster(&client, cluster).await?;
    let path = format!("/api/clusters/{id}/{}", kind_name(kind));
    let text = client.get_text(&path).await?;
    print!("{text}");
    Ok(())
}

/// Read the config + DB directly from this host (must be the TCS box).
async fn local_dump(kind: Kind, cluster_flag: Option<&str>) -> CliResult<()> {
    load_local_secret_env();
    let config = crate::config::Config::load()
        .map_err(|e| CliError::Other(format!("local TCS config: {e}")))?;
    let pool = crate::db::init_pool(&config.database)
        .await
        .map_err(|e| CliError::Other(format!("local TCS database: {e}")))?;
    let clusters = crate::db::repos::cluster::list(&pool)
        .await
        .map_err(|e| CliError::Other(format!("local TCS database: {e}")))?;
    if clusters.is_empty() {
        return Err(CliError::Other("no clusters in the local TCS database".into()));
    }

    let c = match cluster_flag {
        Some(want) => {
            let matches: Vec<_> = clusters
                .iter()
                .filter(|c| {
                    c.id.to_string() == want || c.name == want || c.id.to_string().starts_with(want)
                })
                .cloned()
                .collect();
            match matches.len() {
                0 => return Err(CliError::Other(format!("no cluster matches '{want}' (local)"))),
                1 => matches.into_iter().next().unwrap(),
                _ => {
                    return Err(CliError::Other(format!(
                        "cluster '{want}' is ambiguous (local); pass a full UUID"
                    )))
                }
            }
        }
        None => {
            if clusters.len() == 1 {
                clusters.into_iter().next().unwrap()
            } else {
                return Err(CliError::Other(format!(
                    "no cluster selected: {} clusters in local DB, pass --cluster",
                    clusters.len()
                )));
            }
        }
    };

    let enc = match kind {
        Kind::Kubeconfig => c.kubeconfig,
        Kind::Talosconfig => c.talosconfig,
    }
    .ok_or_else(|| {
        CliError::Other(format!(
            "cluster '{}' has no {} attached",
            c.name,
            kind_name(kind)
        ))
    })?;

    let plain = crate::utils::secrets::decrypt(&config.auth.jwt_secret, &enc)
        .map_err(|e| CliError::Other(format!("decrypt {}: {e}", kind_name(kind))))?;
    print!("{plain}");
    Ok(())
}

/// Mirror systemd's `EnvironmentFile=-/etc/tcs/env`: if the JWT secret is not
/// already in the environment, pull it from the env file (root-readable, 0600).
/// This makes `--local` use exactly the secret the tcs.service process uses.
fn load_local_secret_env() {
    if let Ok(s) = std::env::var("TCS_AUTH_JWT_SECRET") {
        if !s.trim().is_empty() {
            return;
        }
    }
    let path = std::env::var("TCS_ENV_FILE").unwrap_or_else(|_| "/etc/tcs/env".into());
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k == "TCS_AUTH_JWT_SECRET" {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                if !v.is_empty() {
                    std::env::set_var("TCS_AUTH_JWT_SECRET", v);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_file_secret() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("env");
        std::fs::write(
            &p,
            "# comment\nTCS_AUTH_JWT_SECRET=abc123\nTCS_OTHER=x\n",
        )
        .unwrap();
        std::env::set_var("TCS_ENV_FILE", p);
        std::env::remove_var("TCS_AUTH_JWT_SECRET");
        load_local_secret_env();
        assert_eq!(std::env::var("TCS_AUTH_JWT_SECRET").unwrap(), "abc123");
        std::env::remove_var("TCS_AUTH_JWT_SECRET");
        std::env::remove_var("TCS_ENV_FILE");
    }
}
