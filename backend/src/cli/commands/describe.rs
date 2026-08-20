//! `tcs describe <kind> <name>` — fetch and pretty-print one object's YAML.

use super::client::Client;
use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct DescribeArgs {
    pub kind: String,
    pub name: String,
    #[arg(short, long, alias = "ns")]
    pub namespace: Option<String>,
}

pub async fn run(client: &Client, cluster: &str, args: &DescribeArgs) -> super::super::client::CliResult<()> {
    let base = format!("/api/clusters/{cluster}/k8s");
    let ns = args
        .namespace
        .as_deref()
        .map(|n| format!("&ns={n}"))
        .unwrap_or_default();
    let raw = client
        .get_json(&format!("{base}/resource/{}?kind={}&{ns}", args.name, args.kind))
        .await?;
    let yaml = serde_yaml::to_string(&raw).unwrap_or_else(|_| raw.to_string());
    println!("{yaml}");
    Ok(())
}
