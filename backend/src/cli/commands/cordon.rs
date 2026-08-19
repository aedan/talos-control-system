//! `tcs cordon <node>` / `tcs uncordon <node>` — set a node unschedulable/schedulable.

use clap::Args;

use super::client::Client;

#[derive(Args, Debug, Clone)]
pub struct CordonArgs {
    /// Node name.
    pub node: String,
}

pub async fn run(client: &Client, cluster: &str, args: &CordonArgs) -> super::super::client::CliResult<()> {
    let res = client
        .post_json(
            &format!("/api/clusters/{cluster}/k8s/cordon"),
            &serde_json::json!({ "name": args.node }),
        )
        .await?;
    println!("cordoned {}", res.get("node").and_then(|n| n.as_str()).unwrap_or(&args.node));
    Ok(())
}

#[derive(Args, Debug, Clone)]
pub struct UncordonArgs {
    /// Node name.
    pub node: String,
}

pub async fn run_uncordon(client: &Client, cluster: &str, args: &UncordonArgs) -> super::super::client::CliResult<()> {
    let res = client
        .post_json(
            &format!("/api/clusters/{cluster}/k8s/uncordon"),
            &serde_json::json!({ "name": args.node }),
        )
        .await?;
    println!("uncordoned {}", res.get("node").and_then(|n| n.as_str()).unwrap_or(&args.node));
    Ok(())
}
