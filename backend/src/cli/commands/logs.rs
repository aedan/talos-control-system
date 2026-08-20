//! `tcs logs <pod>` — print (and optionally follow) pod logs.

use super::client::Client;
use clap::Args;

#[derive(Args, Debug, Clone)]
pub struct LogsArgs {
    /// Pod name.
    pub pod: String,
    #[arg(short, long, alias = "ns")]
    pub namespace: Option<String>,
    #[arg(short, long)]
    pub container: Option<String>,
    /// Number of lines from the end (default 100 when not following).
    #[arg(short, long)]
    pub tail: Option<i64>,
    /// Show logs from the previous container instance.
    #[arg(long)]
    pub previous: bool,
    /// Follow the log stream.
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn run(client: &Client, cluster: &str, args: &LogsArgs) -> super::super::client::CliResult<()> {
    let ns = args.namespace.clone().unwrap_or_else(|| "default".to_string());
    let mut q = format!(
        "/api/clusters/{cluster}/k8s/logs?ns={ns}&name={}&follow={}",
        args.pod, args.follow
    );
    if let Some(c) = &args.container {
        q.push_str(&format!("&container={c}"));
    }
    if let Some(t) = args.tail {
        q.push_str(&format!("&tail={t}"));
    }
    if args.previous {
        q.push_str("&previous=true");
    }

    if args.follow {
        client
            .stream_sse(&q, |line| println!("{line}"))
            .await?;
    } else {
        let text = client.get_text(&q).await?;
        print!("{text}");
    }
    Ok(())
}
