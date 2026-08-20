//! `tcs delete <kind> <name>` — delete a K8s object.

use std::io::Write;

use clap::Args;

use super::client::Client;

#[derive(Args, Debug, Clone)]
pub struct DeleteArgs {
    pub kind: String,
    pub name: String,
    #[arg(short, long, alias = "ns")]
    pub namespace: Option<String>,
    /// Skip confirmation prompt.
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(client: &Client, cluster: &str, args: &DeleteArgs) -> super::super::client::CliResult<()> {
    if !args.force {
        print!("Delete {}/{} in cluster {cluster}? [y/N] ", args.kind, args.name);
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    let base = format!("/api/clusters/{cluster}/k8s");
    let ns = args
        .namespace
        .as_deref()
        .map(|n| format!("&ns={n}"))
        .unwrap_or_default();
    let res = client
        .delete_json(&format!("{base}/resource/{}?kind={}&{ns}", args.name, args.kind))
        .await?;
    println!("deleted {}", res.get("kind").and_then(|k| k.as_str()).unwrap_or(&args.kind));
    Ok(())
}
