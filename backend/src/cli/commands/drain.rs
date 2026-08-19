//! `tcs drain <node>` — cordon a node and evict its pods.

use std::io::Write;

use clap::Args;

use super::client::Client;

#[derive(Args, Debug, Clone)]
pub struct DrainArgs {
    /// Node name.
    pub node: String,
    /// Skip confirmation prompt and force-evict.
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(client: &Client, cluster: &str, args: &DrainArgs) -> super::super::client::CliResult<()> {
    if !args.force {
        print!("Drain node {} in cluster {cluster}? [y/N] ", args.node);
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    let res = client
        .post_json(
            &format!("/api/clusters/{cluster}/k8s/drain"),
            &serde_json::json!({ "name": args.node, "force": args.force }),
        )
        .await?;
    let evicted = res.get("evicted").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
    let skipped = res.get("skipped").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
    let errors = res.get("errors").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
    println!("drained {}: {evicted} evicted, {skipped} skipped, {errors} errors", args.node);
    if let Some(errs) = res.get("errors").and_then(|e| e.as_array()) {
        for e in errs {
            if let Some(s) = e.as_str() {
                eprintln!("  {s}");
            }
        }
    }
    Ok(())
}
