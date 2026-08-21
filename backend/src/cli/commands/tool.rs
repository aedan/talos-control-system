//! `tcs kubectl|helm|talosctl <args...>` — passthrough to the real binaries,
//! executed server-side by TCS using the cluster's stored credentials.
//!
//! The CLI never sees the kubeconfig/talosconfig; it only ships argv (and, when
//! stdin is piped, its bytes) to the server and renders the output. Interactive
//! commands (TTY detected) are bridged over the tool PTY WebSocket.

use std::io::{self, IsTerminal, Read, Write};

use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};

use super::client::{CliError, CliResult, Client};
use base64::Engine as _;

/// Decide whether the given argv looks interactive (needs a PTY).
fn is_interactive(args: &[String]) -> bool {
    let has_tty_flag = args.iter().any(|a| {
        a == "-it"
            || a == "-ti"
            || a == "-i"
            || a == "-t"
            || a == "--tty"
            || a == "--interactive"
    });
    let is_exec_like = args.iter().any(|a| a == "exec" || a == "attach");
    is_exec_like && has_tty_flag
}

/// Run the given tool (kubectl/helm/talosctl) with raw args on the server.
/// `args` is everything after the tool name, passed verbatim.
pub async fn run(client: &Client, cluster: &str, tool: &str, args: &[String]) -> CliResult<()> {
    let argv: Vec<String> = std::iter::once(tool.to_string()).chain(args.iter().cloned()).collect();

    if is_interactive(&argv) {
        return run_interactive(client, cluster, &argv).await;
    }
    run_oneshot(client, cluster, &argv).await
}

/// One-shot: POST argv (+piped stdin) and print stdout/stderr, exit with code.
async fn run_oneshot(client: &Client, cluster: &str, argv: &[String]) -> CliResult<()> {
    let mut body = serde_json::json!({ "argv": argv });
    // If local stdin is piped (not a TTY), forward its contents.
    if !io::stdin().is_terminal() {
        let mut s = String::new();
        if io::stdin().read_to_string(&mut s).is_ok() && !s.is_empty() {
            body["stdin"] = serde_json::json!(
                base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
            );
        }
    }
    let res = client
        .post_json_long(&format!("/api/clusters/{cluster}/tool"), &body)
        .await?;
    let stdout = res.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = res.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    let code = res.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
    if !stdout.is_empty() {
        print!("{stdout}");
        let _ = io::stdout().flush();
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
        let _ = io::stderr().flush();
    }
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Interactive: bridge local stdin/stdout to the tool PTY WebSocket.
async fn run_interactive(client: &Client, cluster: &str, argv: &[String]) -> CliResult<()> {
    let q = format!(
        "/api/clusters/{cluster}/tool/tty?argv={}&token={}",
        urlencoding::encode(&serde_json::to_string(argv).unwrap_or_default()),
        client.token
    );
    let (mut ws, _) = connect_async(client.absolute(&q))
        .await
        .map_err(|e| CliError::Other(e.to_string()))?;

    let mut out = io::stdout();
    let mut stdin = tokio::io::stdin();
    let mut buf = [0u8; 4096];
    let mut exit_code: i32 = 0;
    let mut stdin_open = true;

    loop {
        tokio::select! {
            r = stdin.read(&mut buf), if stdin_open => {
                match r {
                    Ok(0) => {
                        stdin_open = false;
                    }
                    Ok(k) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&buf[..k]);
                        let msg = WsMsg::Text(serde_json::json!({ "type": "stdin", "data": b64 }).to_string());
                        if ws.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => stdin_open = false,
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
