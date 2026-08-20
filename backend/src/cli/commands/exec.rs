//! `tcs exec <pod> -- <command...>` — run a command in a pod container.
//!
//! Bridges local stdin/stdout to the TCS exec WebSocket. The kubeconfig never
//! touches the CLI; the server proxies the exec stream.

use std::io::{self, Write};

use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};

use super::client::Client;
use base64::Engine as _;

#[derive(Args, Debug, Clone)]
pub struct ExecArgs {
    /// Pod name.
    pub pod: String,
    #[arg(short, long, alias = "ns")]
    pub namespace: Option<String>,
    #[arg(short, long)]
    pub container: Option<String>,
    /// Use a TTY (interactive shell).
    #[arg(short = 't', long)]
    pub tty: bool,
    /// Command to run (after `--`).
    #[arg(last = true)]
    pub command: Vec<String>,
}

pub async fn run(client: &Client, cluster: &str, args: &ExecArgs) -> super::super::client::CliResult<()> {
    let ns = args.namespace.clone().unwrap_or_else(|| "default".to_string());
    let cmd = if args.command.is_empty() {
        vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()]
    } else {
        args.command.clone()
    };

    let mut q = format!(
        "/api/clusters/{cluster}/k8s/exec?ns={ns}&name={}&tty={}&token={}",
        args.pod,
        args.tty,
        client.token
    );
    if let Some(c) = &args.container {
        q.push_str(&format!("&container={c}"));
    }
    q.push_str(&format!("&command={}", urlencoding::encode(&serde_json::to_string(&cmd).unwrap_or_default())));

    let (mut ws, _) = connect_async(client.absolute(&q))
        .await
        .map_err(|e| super::super::client::CliError::Other(e.to_string()))?;

    let mut out = io::stdout();
    let mut err = io::stderr();
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 4096];

    loop {
        tokio::select! {
            // Read local stdin and forward to the pod.
            r = stdin.read(&mut buf) => {
                match r {
                    Ok(0) => {
                        // EOF: close the stdin side and keep draining output.
                        let _ = ws.send(WsMsg::Close(None)).await;
                        break;
                    }
                    Ok(k) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..k]);
                        let msg = WsMsg::Text(serde_json::json!({ "type": "stdin", "data": b64 }).to_string());
                        if ws.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Receive pod output.
            maybe = ws.next() => {
                match maybe {
                    Some(Ok(WsMsg::Text(text))) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            match v.get("type").and_then(|t| t.as_str()) {
                                Some("stdout") => {
                                    if let Some(d) = v.get("data").and_then(|d| d.as_str()) {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d) {
                                            let _ = out.write_all(&bytes);
                                            let _ = out.flush();
                                        }
                                    }
                                }
                                Some("stderr") => {
                                    if let Some(d) = v.get("data").and_then(|d| d.as_str()) {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d) {
                                            let _ = err.write_all(&bytes);
                                            let _ = err.flush();
                                        }
                                    }
                                }
                                Some("exit") => break,
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(WsMsg::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
        }
    }

    let _ = ws.close(None).await;
    Ok(())
}
