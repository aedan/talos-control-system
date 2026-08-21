//! Run real `kubectl` / `helm` / `talosctl` on the server on behalf of an
//! authenticated caller, using the cluster's stored (encrypted) credentials.
//!
//! Security model:
//!   * The kubeconfig / talosconfig are decrypted in memory only.
//!   * They are written to a `0600` file inside a `0700` temp dir, with a
//!     `Drop` guard that removes the file (and dir) even on panic.
//!   * The plaintext never reaches the CLI — only command output does.
//!   * The tool name is restricted to a fixed allowlist; the argv is passed
//!     verbatim (no shell interpretation), so there is no shell-injection
//!     surface.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;

use super::k8s_common;

use base64::Engine as _;

const TOOLS: [&str; 3] = ["kubectl", "helm", "talosctl"];
const ONE_SHOT_TIMEOUT: Duration = Duration::from_secs(600);

fn bin_path(tool: &str) -> PathBuf {
    match tool {
        "kubectl" => PathBuf::from("/usr/local/bin/kubectl"),
        "helm" => PathBuf::from("/usr/local/bin/helm"),
        "talosctl" => PathBuf::from("/usr/local/bin/talosctl"),
        other => PathBuf::from(other),
    }
}

/// Write a decrypted credential blob to a `0600` file in a `0700` temp dir.
/// The `TempDir` is kept alive for the struct's lifetime; dropping it removes
/// both the file and the directory (including on panic).
struct TempCred {
    path: PathBuf,
    dir: tempfile::TempDir,
}

impl TempCred {
    fn new(content: &str, label: &str) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix(&format!("tcs-{label}-"))
            .tempdir()
            .map_err(|e| e.to_string())?;
        let path = dir.path().join(format!("{label}.yaml"));
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            let _ = std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self { path, dir })
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}

// `TempDir`'s Drop removes the directory (and the file within it) on scope
// exit or panic, so no manual cleanup is needed.

/// Decrypt the credential blob the tool needs for this cluster.
fn credential_for(state: &AppState, cluster: &crate::db::models::Cluster, tool: &str) -> Result<String, (StatusCode, String)> {
    let enc = match tool {
        "talosctl" => cluster
            .talosconfig
            .clone()
            .ok_or((StatusCode::BAD_REQUEST, "Cluster has no talosconfig attached".to_string()))?,
        _ => cluster
            .kubeconfig
            .clone()
            .ok_or((StatusCode::BAD_REQUEST, "Cluster has no kubeconfig attached".to_string()))?,
    };
    crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, &enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn validate_argv(argv: &[String]) -> Result<Vec<String>, (StatusCode, String)> {
    let first = argv.first().map(|s| s.as_str()).unwrap_or("");
    if !TOOLS.contains(&first) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("tool '{first}' not allowed (use kubectl, helm, or talosctl)"),
        ));
    }
    Ok(argv.to_vec())
}

/// Build the environment for the spawned tool, pointing it at the temp cred.
fn tool_env(tool: &str, cred: &TempCred) -> Vec<(String, String)> {
    let mut env = vec![
        ("PATH".to_string(), "/usr/local/bin:/usr/bin:/bin".to_string()),
        ("HOME".to_string(), "/root".to_string()),
    ];
    match tool {
        "talosctl" => env.push(("TALOSCONFIG".to_string(), cred.path().to_string_lossy().to_string())),
        _ => env.push(("KUBECONFIG".to_string(), cred.path().to_string_lossy().to_string())),
    }
    env
}

#[derive(Deserialize)]
pub struct ToolRunBody {
    /// Full argv including the tool name, e.g. ["kubectl","get","pods"].
    pub argv: Vec<String>,
    /// Optional base64-encoded stdin (for `kubectl apply -f -`).
    #[serde(default)]
    pub stdin: Option<String>,
}

/// POST /clusters/:id/tool — run a one-shot command and return output + exit code.
pub async fn run_tool(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ToolRunBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let argv = validate_argv(&body.argv)?;
    let tool = argv[0].as_str();
    let cluster = k8s_common::load_cluster(&state, id).await?;
    let plain = credential_for(&state, &cluster, tool)?;
    let cred = TempCred::new(&plain, tool).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let stdin_bytes = body
        .stdin
        .as_deref()
        .map(|s| base64::engine::general_purpose::STANDARD.decode(s))
        .transpose()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad stdin base64: {e}")))?;

    let mut cmd = tokio::process::Command::new(bin_path(tool));
    cmd.args(&argv[1..])
        .env_clear()
        .envs(tool_env(tool, &cred))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("spawn {tool} failed: {e}")))?;

    if let (Some(bytes), Some(mut stdin)) = (stdin_bytes, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(&bytes).await {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("write stdin failed: {e}")));
        }
        let _ = stdin.shutdown().await;
    }

    let output = tokio::time::timeout(ONE_SHOT_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| (StatusCode::GATEWAY_TIMEOUT, format!("{tool} timed out after {ONE_SHOT_TIMEOUT:?}")))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("wait failed: {e}")))?;

    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // `cred` is dropped here (and on any early return above), removing the file.
    Ok(Json(serde_json::json!({
        "exitCode": code,
        "stdout": stdout,
        "stderr": stderr,
    })))
}

#[derive(Deserialize)]
pub struct ToolTtyQuery {
    /// JSON array of argv including the tool name.
    pub argv: String,
}

/// GET /clusters/:id/tool/tty?argv=<json>  (WebSocket upgrade)
///
/// Interactive PTY session. Protocol (JSON text frames, base64 payloads):
///   client -> server: {"type":"stdin","data":"<b64>"}
///                     {"type":"resize","cols":N,"rows":N}
///   server -> client: {"type":"stdout","data":"<b64>"}
///                     {"type":"exit","code":N}
pub async fn tool_tty(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ToolTtyQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let argv: Vec<String> = serde_json::from_str(&q.argv)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("bad argv json: {e}")))?;
    let argv = validate_argv(&argv)?;
    let tool = argv[0].clone();
    let cluster = k8s_common::load_cluster(&state, id).await?;
    let plain = credential_for(&state, &cluster, &tool)?;
    let cred = TempCred::new(&plain, &tool).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(ws.on_upgrade(move |socket| run_pty_session(socket, tool, argv, cred)))
}

async fn run_pty_session(socket: WebSocket, tool: String, argv: Vec<String>, cred: TempCred) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Spawn the tool in a PTY.
    let pty = match portable_pty::native_pty_system().openpty(portable_pty::PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Text(serde_json::json!({"type":"exit","code":-1,"error":e.to_string()}).to_string()))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };

    let mut child_match = match pty.slave.spawn_command({
        let mut cb = portable_pty::CommandBuilder::new(bin_path(&tool));
        cb.args(&argv[1..]);
        cb.env_clear();
        for (k, v) in tool_env(&tool, &cred) {
            cb.env(k, v);
        }
        cb
    }) {
        Ok(m) => m,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Text(serde_json::json!({"type":"exit","code":-1,"error":e.to_string()}).to_string()))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };

    let master = pty.master;

    // Reader: PTY master (blocking std) read on a dedicated thread, pushed to a channel.
    let master_reader = match master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Text(serde_json::json!({"type":"exit","code":-1,"error":e.to_string()}).to_string()))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = master_reader;
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Writer: PTY master (blocking std) guarded by a Mutex, written via spawn_blocking.
    let master_writer = match master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            let _ = ws_tx
                .send(Message::Text(serde_json::json!({"type":"exit","code":-1,"error":e.to_string()}).to_string()))
                .await;
            let _ = ws_tx.close().await;
            return;
        }
    };
    let writer = std::sync::Arc::new(std::sync::Mutex::new(master_writer));

    // Drive both directions.
    let mut exit_code: i32 = 0;
    loop {
        tokio::select! {
            maybe = ws_rx.next() => {
                match maybe {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            match v.get("type").and_then(|t| t.as_str()) {
                                Some("stdin") => {
                                    if let Some(d) = v.get("data").and_then(|x| x.as_str()) {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d) {
                                            let w = writer.clone();
                                            let res = tokio::task::spawn_blocking(move || {
                                                use std::io::Write;
                                                let mut g = w.lock().unwrap();
                                                g.write_all(&bytes).and_then(|_| g.flush())
                                            })
                                            .await;
                                            if res.is_err() || res.unwrap().is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                Some("resize") => {
                                    if let (Some(cols), Some(rows)) = (
                                        v.get("cols").and_then(|x| x.as_u64()),
                                        v.get("rows").and_then(|x| x.as_u64()),
                                    ) {
                                        let _ = master.resize(portable_pty::PtySize {
                                            rows: rows as u16,
                                            cols: cols as u16,
                                            pixel_width: 0,
                                            pixel_height: 0,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                }
            }
            maybe = out_rx.recv() => {
                match maybe {
                    Some(data) => {
                        let payload = serde_json::json!({
                            "type": "stdout",
                            "data": base64::engine::general_purpose::STANDARD.encode(&data),
                        });
                        if ws_tx.send(Message::Text(payload.to_string())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            // Poll for child exit so we report the real code promptly.
            _ = async {
                loop {
                    match child_match.try_wait() {
                        Ok(Some(_)) => return,
                        Ok(None) => {}
                        Err(_) => return,
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => {
                if let Ok(status) = child_match.wait() {
                    exit_code = status.exit_code() as i32;
                }
                break;
            }
        }
    }

    // Tear down.
    let _ = child_match.kill();
    let payload = serde_json::json!({"type": "exit", "code": exit_code});
    let _ = ws_tx.send(Message::Text(payload.to_string())).await;
    let _ = ws_tx.close().await;
    // `cred` drops here, removing the temp file.
}
