//! Remote OOB proxy: token management (protected) + agent tunnel (public,
//! token-gated). The agent dials out over WebSocket and relays Redfish BMC
//! operations; TCS never holds Core credentials and the agent holds no BMC
//! credentials (they are sent per-op over the authenticated tunnel).

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;

use super::handlers::extract_claims;

#[derive(Deserialize)]
pub struct TunnelQuery {
    pub token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentHello {
    #[serde(default)]
    caps: Vec<String>,
    #[serde(default)]
    label: Option<String>,
}

/// GET /api/proxy/tunnel?token=...  (WebSocket upgrade, token-gated)
pub async fn tunnel_ws(
    State(state): State<AppState>,
    Query(q): Query<TunnelQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let ok = crate::db::repos::proxy::validate_token(&state.db_pool, &q.token)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !ok {
        return Err((StatusCode::UNAUTHORIZED, "Invalid join token".into()));
    }
    let agent_id = crate::network::tunnel::agent_id_from_token(&q.token);
    let tunnel = state.tunnel.clone();
    Ok(ws.on_upgrade(move |socket| run_tunnel(socket, tunnel, agent_id)))
}

async fn run_tunnel(socket: WebSocket, tunnel: crate::network::tunnel::TunnelHandle, agent_id: String) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Wait for a hello frame to learn capabilities/label before registering.
    let hello = loop {
        match ws_rx.next().await {
            Some(Ok(Message::Text(t))) => {
                if let Some(h) = parse_hello(&t) {
                    break h;
                }
            }
            _ => break AgentHello { caps: vec![], label: None },
        }
    };

    let (_op_tx, mut op_rx) = tunnel.upsert(&agent_id, hello.caps.clone(), hello.label.clone());
    let _ = ws_tx
        .send(Message::Text(
            serde_json::json!({ "type": "hello.ack", "agentId": agent_id })
                .to_string(),
        ))
        .await;

    // Reader: agent -> TCS (operation results).
    let reader_tunnel = tunnel.clone();
    let reader_agent = agent_id.clone();
    let mut reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Text(t) => {
                    if let Some(result) = parse_result(&t) {
                        reader_tunnel.deliver(&reader_agent, result);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // Writer: TCS -> agent (framed BMC ops).
    let mut writer = tokio::spawn(async move {
        while let Some(frame) = op_rx.recv().await {
            if ws_tx.send(Message::Text(frame)).await.is_err() {
                break;
            }
        }
    });

    // End the session when either side finishes.
    tokio::select! {
        _ = &mut reader => {},
        _ = &mut writer => {},
    }
    reader.abort();
    writer.abort();
    tunnel.disconnect(&agent_id);
}

fn parse_hello(s: &str) -> Option<AgentHello> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("hello") {
        return None;
    }
    serde_json::from_value(v).ok()
}

fn parse_result(s: &str) -> Option<crate::network::tunnel::BmcOpResult> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    match v.get("type").and_then(|t| t.as_str()) {
        Some("resp") => serde_json::from_value(v).ok(),
        _ => None,
    }
}

// ─── Token management (admin) ────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProxyTokenRequest {
    pub label: Option<String>,
    pub expires_hours: Option<i64>,
}

pub async fn list_proxy_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    let tokens = crate::db::repos::proxy::list_tokens(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        tokens
            .into_iter()
            .filter_map(|t| serde_json::to_value(t).ok())
            .collect(),
    ))
}

pub async fn create_proxy_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateProxyTokenRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    let token = format!("pxj_{}", Uuid::new_v4().simple());
    let exp = payload
        .expires_hours
        .map(|h| chrono::Utc::now() + chrono::Duration::hours(h));
    crate::db::repos::proxy::create_token(
        &state.db_pool,
        &token,
        payload.label.as_deref(),
        exp,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "proxy_token_create",
        "proxy",
        &token,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "token": token, "expiresAt": exp })),
    ))
}

pub async fn delete_proxy_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    crate::db::repos::proxy::delete_token(&state.db_pool, &token)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let agent_id = crate::network::tunnel::agent_id_from_token(&token);
    state.tunnel.disconnect(&agent_id);
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "proxy_token_delete",
        "proxy",
        &token,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/proxy/agents — connected OOB agents for the admin UI.
pub async fn list_proxy_agents(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    Ok(Json(state.tunnel.online_list()))
}
