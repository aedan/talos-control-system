//! `tcs attach <pod>` — attach to a pod container's stdin/stdout/stderr.

use std::io::{self, Write};

use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};

use super::client::Client;
use base64::Engine as _;

#[derive(Args, Debug, Clone)]
pub struct AttachArgs {
    /// Pod name.
    pub pod: String,
    #[arg(short, long, alias = "ns")]
    pub namespace: Option<String>,
    #[arg(short, long)]
    pub container: Option<String>,
    /// Use a TTY (interactive).
    #[arg(short = 't', long)]
    pub tty: bool,
}

pub async fn run(client: &Client, cluster: &str, args: &AttachArgs) -> super::super::client::CliResult<()> {
    let ns = args.namespace.clone().unwrap_or_else(|| "default".to_string());
    let mut q = format!(
        "/api/clusters/{cluster}/k8s/attach?ns={ns}&name={}&tty={}&token={}",
        args.pod, args.tty, client.token
    );
    if let Some(c) = &args.container {
        q.push_str(&format!("&container={c}"));
    }

    let (mut ws, _) = connect_async(client.absolute(&q))
        .await
        .map_err(|e| super::super::client::CliError::Other(e.to_string()))?;

    let mut out = io::stdout();
    let mut err = io::stderr();
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 4096];
    let mut exit_code: i32 = 0;

    loop {
        tokio::select! {
            r = stdin.read(&mut buf) => {
                match r {
                    Ok(0) => {
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
                                Some("exit") => {
                                    exit_code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0) as i32;
                                    break;
                                }
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
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}
