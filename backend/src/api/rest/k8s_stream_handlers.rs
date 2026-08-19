//! Streaming K8s endpoints: pod logs (SSE) and exec/attach (WebSocket).
//!
//! Auth for these routes uses `?token=` (the RBAC middleware accepts it) because
//! EventSource / WebSocket clients cannot set an `Authorization` header.
//!
//! WebSocket protocol (JSON text frames, base64 payloads):
//!   client -> server: `{"type":"stdin","data":"<b64>"}`
//!                     `{"type":"resize","cols":N,"rows":N}`
//!   server -> client: `{"type":"stdout","data":"<b64>"}`
//!                     `{"type":"stderr","data":"<b64>"}`
//!                     `{"type":"exit","code":N}`

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::Json;
use futures::io::AsyncBufReadExt;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::AppState;

use super::k8s_common;

use base64::Engine as _;

#[derive(Deserialize)]
pub struct LogsQuery {
    ns: String,
    name: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    tail: Option<i64>,
    #[serde(default)]
    previous: bool,
    #[serde(default)]
    follow: bool,
}

/// GET /clusters/:id/k8s/logs?ns=&name=&container=&tail=&previous=&follow=
///
/// `follow=false` returns the log text as `text/plain`.
/// `follow=true` returns a `text/event-stream` (SSE) of `data: <line>` events.
pub async fn logs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<LogsQuery>,
) -> Result<Response, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;

    if !q.follow {
        let text = client
            .logs(&q.ns, &q.name, q.container.as_deref(), q.tail, q.previous, None)
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(text))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
    }

    let reader = client
        .log_stream(&q.ns, &q.name, q.container.as_deref(), q.tail, q.previous)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    // `reader` is a futures `AsyncBufRead`; `.lines()` yields a futures Stream of
    // `io::Result<String>`, which is also a `TryStream` — exactly what `Body::from_stream` wants.
    let stream = reader.lines().map(|res| {
        res.map(|line| format!("data: {line}\n\n").into_bytes())
    });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(stream))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?)
}

#[derive(Deserialize)]
pub struct ExecQuery {
    ns: String,
    name: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    tty: bool,
    /// Space-separated command (e.g. "sh -c ls"). Empty => default shell.
    #[serde(default)]
    command: Option<String>,
}

/// GET /clusters/:id/k8s/exec?ns=&name=&container=&tty=&command=  (WebSocket upgrade)
pub async fn exec_ws(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ExecQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let cmd: Vec<String> = q
        .command
        .as_deref()
        .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()]);
    let attached = client
        .exec(&q.ns, &q.name, &cmd, q.container.as_deref(), q.tty)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(ws.on_upgrade(move |socket| run_session(socket, attached)))
}

#[derive(Deserialize)]
pub struct AttachQuery {
    ns: String,
    name: String,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    tty: bool,
}

/// GET /clusters/:id/k8s/attach?ns=&name=&container=&tty=  (WebSocket upgrade)
pub async fn attach_ws(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<AttachQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let attached = client
        .attach(&q.ns, &q.name, q.container.as_deref(), q.tty)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(ws.on_upgrade(move |socket| run_session(socket, attached)))
}

fn parse_client_msg(s: &str) -> Option<(String, serde_json::Value)> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let t = v.get("type")?.as_str()?.to_string();
    Some((t, v))
}

/// Bridge a WebSocket session to a kube `AttachedProcess` (exec or attach).
async fn run_session(socket: WebSocket, mut attached: kube::api::AttachedProcess) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // stdin: main loop -> channel -> writer task (task owns the writer).
    let mut stdin_writer = attached.stdin();
    let (in_tx, in_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let stdin_task = tokio::spawn(async move {
        let Some(mut w) = stdin_writer else { return };
        let mut rx = in_rx;
        while let Some(data) = rx.recv().await {
            if w.write_all(&data).await.is_err() {
                break;
            }
            let _ = w.flush().await;
        }
        let _ = w.shutdown().await;
    });

    // stdout/stderr: reader task (owns the readers) -> channel -> main loop.
    let mut stdout_reader = attached.stdout();
    let mut stderr_reader = attached.stderr();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<(String, Vec<u8>)>(64);
    let out_task = tokio::spawn(async move {
        if let Some(mut r) = stdout_reader {
            let mut buf = [0u8; 8192];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if out_tx.send(("stdout".into(), buf[..n].to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        if let Some(mut r) = stderr_reader {
            let mut buf = [0u8; 8192];
            loop {
                match r.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if out_tx.send(("stderr".into(), buf[..n].to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    });

    // Terminal resize channel (futures mpsc sender; one-shot).
    let mut resize_tx = attached.terminal_size();

    // Drive both directions until the client disconnects or the process exits.
    loop {
        tokio::select! {
            biased;
            maybe = ws_rx.next() => {
                match maybe {
                    Some(Ok(Message::Text(text))) => {
                        if let Some((t, v)) = parse_client_msg(&text) {
                            match t.as_str() {
                                "stdin" => {
                                    if let Some(d) = v.get("data").and_then(|x| x.as_str()) {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(d) {
                                            let _ = in_tx.send(bytes).await;
                                        }
                                    }
                                }
                                "resize" => {
                                    if let (Some(cols), Some(rows)) = (
                                        v.get("cols").and_then(|x| x.as_u64()),
                                        v.get("rows").and_then(|x| x.as_u64()),
                                    ) {
                                        if let Some(ts) = resize_tx.as_mut() {
                                            let _ = ts
                                                .send(kube::api::TerminalSize {
                                                    width: cols as u16,
                                                    height: rows as u16,
                                                })
                                                .await;
                                        }
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
                    Some((stream, data)) => {
                        let payload = serde_json::json!({
                            "type": stream,
                            "data": base64::engine::general_purpose::STANDARD.encode(&data),
                        });
                        if ws_tx.send(Message::Text(payload.to_string())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // Tear down: abort the process; dropping in_tx ends the stdin task.
    attached.abort();
    drop(in_tx);
    stdin_task.abort();
    out_task.abort();
    let _ = ws_tx.send(Message::Text(
        serde_json::json!({"type":"exit","code":0}).to_string(),
    ))
    .await;
    let _ = ws_tx.close().await;
}
