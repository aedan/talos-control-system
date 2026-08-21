//! The `tcs` CLI: a thin authenticated client over the TCS REST API.
//!
//! Invoked as `tcs <verb> ...` (the same binary that runs the server with no
//! args). Authentication comes from `--token`/`TCS_TOKEN`/`~/.tcs/config`.

pub mod client;
pub mod commands;
pub mod config;
pub mod output;

use clap::{Parser, Subcommand};

use client::{CliError, CliResult, Client};
use serde_json::Value;
use commands::{
    apply, attach, cordon, delete, describe, drain, exec, get, logs, scale,
};

/// Talos Control System CLI.
#[derive(Parser, Debug)]
#[command(name = "tcs", version, about = "Talos Control System CLI")]
pub struct Cli {
    /// TCS server URL (default: local server config, else http://localhost:8081).
    #[arg(short, long, global = true)]
    pub server: Option<String>,
    /// Bearer token (overrides TCS_TOKEN and config).
    #[arg(short, long, global = true)]
    pub token: Option<String>,
    /// Default cluster (name or UUID; overrides TCS_CLUSTER and config).
    #[arg(short, long, global = true)]
    pub cluster: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the TCS control-plane server (used by the systemd unit).
    Serve,
    /// Authenticate and store a token in ~/.tcs/config.
    ///
    /// With no arguments, prompts for email and password interactively.
    Login {
        /// Email address (or use --email / interactive prompt).
        #[arg(value_name = "EMAIL")]
        email: Option<String>,
        /// Password (or use --password / interactive prompt).
        #[arg(value_name = "PASSWORD")]
        password: Option<String>,
        /// Email address (alternative to the positional EMAIL).
        #[arg(long = "email")]
        flag_email: Option<String>,
        /// Password (alternative to the positional PASSWORD).
        #[arg(long = "password")]
        flag_password: Option<String>,
    },
    /// List clusters.
    Clusters,
    /// Get or list K8s objects (arbitrary kinds).
    Get(get::GetArgs),
    /// Describe a single object (prints YAML).
    Describe(describe::DescribeArgs),
    /// Print pod logs.
    Logs(logs::LogsArgs),
    /// Run a command in a pod.
    Exec(exec::ExecArgs),
    /// Attach to a pod container.
    Attach(attach::AttachArgs),
    /// Delete a K8s object.
    Delete(delete::DeleteArgs),
    /// Scale a deployment.
    Scale(scale::ScaleArgs),
    /// Cordon a node.
    Cordon(cordon::CordonArgs),
    /// Uncordon a node.
    Uncordon(cordon::UncordonArgs),
    /// Drain a node.
    Drain(drain::DrainArgs),
    /// Apply a YAML manifest.
    Apply(apply::ApplyArgs),
}

/// Run the CLI (called from `main` when a verb is present).
pub async fn run_cli(cli: Cli) -> CliResult<()> {
    let Some(command) = cli.command else {
        return Err(CliError::Other("no command given".into()));
    };
    match command {
        Command::Login { email, password, flag_email, flag_password } => {
            do_login(
                &cli.server,
                flag_email.or(email),
                flag_password.or(password),
            )
            .await
        }
        Command::Clusters => {
            let client = Client::new(cli.server.as_deref(), cli.token.as_deref())?;
            do_clusters(&client).await
        }
        _ => {
            let client = Client::new(cli.server.as_deref(), cli.token.as_deref())?;
            let cluster = commands::require_cluster(&client, cli.cluster.as_deref()).await?;
            match &command {
                Command::Get(a) => get::run(&client, &cluster, a).await,
                Command::Describe(a) => describe::run(&client, &cluster, a).await,
                Command::Logs(a) => logs::run(&client, &cluster, a).await,
                Command::Exec(a) => exec::run(&client, &cluster, a).await,
                Command::Attach(a) => attach::run(&client, &cluster, a).await,
                Command::Delete(a) => delete::run(&client, &cluster, a).await,
                Command::Scale(a) => scale::run(&client, &cluster, a).await,
                Command::Cordon(a) => cordon::run(&client, &cluster, a).await,
                Command::Uncordon(a) => cordon::run_uncordon(&client, &cluster, a).await,
                Command::Drain(a) => drain::run(&client, &cluster, a).await,
                Command::Apply(a) => apply::run(&client, &cluster, a).await,
                Command::Login { .. } | Command::Clusters | Command::Serve => unreachable!(),
            }
        }
    }
}

async fn do_login(
    server_flag: &Option<String>,
    email: Option<String>,
    password: Option<String>,
) -> CliResult<()> {
    let email = match email {
        Some(e) if !e.trim().is_empty() => e,
        _ => prompt_line("Email: ")?,
    };
    let password = match password {
        Some(p) if !p.trim().is_empty() => p,
        _ => prompt_password("Password: ")?,
    };
    let server = config::CliConfig::resolve_server(server_flag.as_deref());
    let http = reqwest::Client::new();
    let res = http
        .post(format!("{}/api/auth/login", server.trim_end_matches('/')))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .map_err(CliError::Network)?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        let msg = body.get("error").and_then(|e| e.as_str()).unwrap_or("login failed");
        return Err(CliError::Server(format!("{status}: {msg}")));
    }
    let token = body
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| CliError::Other("login response missing token".into()))?
        .to_string();

    let mut cfg = config::CliConfig::load();
    cfg.token = Some(token.clone());
    cfg.server = Some(server.clone());
    cfg.save().map_err(CliError::Io)?;
    println!("logged in; token saved to {}", config::CliConfig::path().display());
    Ok(())
}

async fn do_clusters(client: &Client) -> CliResult<()> {
    let res = client.get_json("/api/clusters").await?;
    let rows: Vec<Value> = res
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut out = serde_json::json!({ "columns": ["ID", "NAME", "STATUS", "KUBECONFIG"], "rows": Vec::<Vec<String>>::new() });
    for c in &rows {
        let row = vec![
            c.get("id").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
            c.get("name").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
            c.get("status").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
            if c.get("hasKubeconfig").and_then(|v| v.as_bool()).unwrap_or(false) {
                "yes".to_string()
            } else {
                "no".to_string()
            },
        ];
        out["rows"].as_array_mut().unwrap().push(row.into());
    }
    println!("{}", output::render(&out, output::Format::Table));
    Ok(())
}

/// Prompt for a line of input (visible), reading from stdin.
fn prompt_line(label: &str) -> CliResult<String> {
    use std::io::{Read, Write};
    print!("{label}");
    std::io::stdout().flush().map_err(CliError::Io)?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(CliError::Io)?;
    let line = line.trim().to_string();
    if line.is_empty() {
        return Err(CliError::Other("no input provided".into()));
    }
    Ok(line)
}

/// Prompt for a password (hidden), reading from the tty.
fn prompt_password(label: &str) -> CliResult<String> {
    print!("{label}");
    std::io::Write::flush(&mut std::io::stdout()).map_err(CliError::Io)?;
    let pw = rpassword::read_password().map_err(CliError::Io)?;
    println!();
    if pw.trim().is_empty() {
        return Err(CliError::Other("no input provided".into()));
    }
    Ok(pw)
}
