//! `tcs apply -f <file>` — server-side apply a YAML manifest (one or more docs).

use std::io::Read;

use clap::Args;

use super::client::Client;

#[derive(Args, Debug, Clone)]
pub struct ApplyArgs {
    /// Manifest file (or `-` for stdin).
    #[arg(short, long)]
    pub file: String,
}

pub async fn run(client: &Client, cluster: &str, args: &ApplyArgs) -> super::super::client::CliResult<()> {
    let yaml = if args.file == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).map_err(super::super::client::CliError::Io)?;
        s
    } else {
        std::fs::read_to_string(&args.file).map_err(|e| super::super::client::CliError::Io(e))?
    };

    let res = client
        .post_json(
            &format!("/api/clusters/{cluster}/k8s/apply"),
            &serde_json::json!({ "manifest": yaml }),
        )
        .await?;

    let results = res.get("results").and_then(|r| r.as_array());
    match results {
        Some(items) => {
            for item in items {
                let kind = item.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
                let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                let ns = item.get("namespace").and_then(|n| n.as_str()).unwrap_or("");
                let status = item.get("status").and_then(|s| s.as_str()).unwrap_or("applied");
                let label = if ns.is_empty() {
                    format!("{kind}/{name}")
                } else {
                    format!("{kind}/{name} ({ns})")
                };
                println!("{status}: {label}");
            }
        }
        None => println!("{}", res),
    }
    Ok(())
}
