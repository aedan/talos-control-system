//! `tcs scale <deployment> --replicas=N` — scale a deployment.

use clap::Args;

use super::client::Client;

#[derive(Args, Debug, Clone)]
pub struct ScaleArgs {
    /// Deployment name.
    pub deployment: String,
    #[arg(short = 'n', long = "namespace", alias = "ns")]
    pub namespace: Option<String>,
    /// Target replica count.
    #[arg(short = 'r', long = "replicas")]
    pub replicas: u32,
}

pub async fn run(client: &Client, cluster: &str, args: &ScaleArgs) -> super::super::client::CliResult<()> {
    let ns = args.namespace.clone().unwrap_or_else(|| "default".to_string());
    let res = client
        .post_json(
            &format!("/api/clusters/{cluster}/k8s/scale"),
            &serde_json::json!({ "ns": ns, "name": args.deployment, "replicas": args.replicas }),
        )
        .await?;
    println!(
        "scaled {}/{} to {} replicas",
        ns,
        args.deployment,
        res.get("replicas").and_then(|r| r.as_u64()).unwrap_or(args.replicas as u64)
    );
    Ok(())
}
