//! Machine OOB console handlers: iLO HTML5 console (asset proxy + KVM WebSocket),
//! iDRAC HTML5 / eHTML5 virtual console (reverse proxy + WebSocket relay), and
//! SOL (Serial-over-LAN WebSocket) as a fallback.
//!
//! Auth model mirrors the K8s stream handlers: the `POST …/console/session` mint
//! requires operator/admin (Authorization header or `?token=`). The asset-proxy
//! and KVM-WebSocket routes are keyed by the unguessable `ilo_…` / `idrac_…`
//! session id (the iframe's `<script>` cannot send an Authorization header).

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use crate::auth::jwt::verify_jwt;

use crate::integration::bmc::{IpmiClient, BmcCredentials};
use crate::integration::ilo_console::{asset, idrac, idrac_proxy, kvm, session};
use crate::AppState;

use super::k8s_common::{claims_from, audit};

/// Resolve the machine + decrypt its BMC password, then build credentials.
async fn machine_bmc(
    state: &AppState,
    id: Uuid,
) -> Result<(crate::db::models::machine::Machine, String, BmcCredentials), (StatusCode, String)> {
    let m = crate::db::repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "BMC not configured".into()));
    }
    let enc = m.bmc_password_enc.as_ref().unwrap();
    let plain = crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((m, plain, creds))
}

#[derive(Deserialize)]
pub struct ForceQuery {
    #[serde(default)]
    force: bool,
}

#[derive(Serialize)]
pub struct IdracCredentialsResponse {
    pub ok: bool,
    pub username: String,
    pub password: String,
    pub idrac_url: String,
}

/// GET /machines/:id/console/idrac-credentials
///
/// On-demand reveal of a Dell machine's iDRAC login credentials (operator/admin,
/// audited). Used by the TCS "Show iDRAC login" popup so a user whose browser
/// password manager hasn't saved the iDRAC creds can read + use them to log in
/// to the new-tab iDRAC console (login.html?console). The password is decrypted
/// from the encrypted store at request time and never cached.
pub async fn idrac_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<IdracCredentialsResponse>, (StatusCode, String)> {
    let claims = claims_from(&headers, None).map_err(|(s, _)| (s, "auth error".to_string()))?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let m = crate::db::repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "machine has no iDRAC".into()));
    }
    let enc = m
        .bmc_password_enc
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "BMC password not configured".into()))?;
    let plain = crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let idrac_url = session::idrac_console_url(&m.bmc_address);
    audit(&state, &claims.sub, "idrac_credentials_reveal", &id.to_string(), &m.bmc_address).await;
    Ok(Json(IdracCredentialsResponse {
        ok: true,
        username: m.bmc_username.clone(),
        password: plain,
        idrac_url,
    }))
}

/// POST /machines/:id/console/session
///
/// Mint (or reuse) an HTML5 console session. Dell iDRAC is HTML5-only (no SOL).
/// HPE iLO is tried first otherwise, then iDRAC, then SOL. Never 500s.
pub async fn create_console_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<ForceQuery>,
) -> Result<Json<session::ConsoleSessionResponse>, (StatusCode, String)> {
    let claims = claims_from(&headers, None).map_err(|(s, _)| (s, "auth error".to_string()))?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let _ = q.force; // single-viewer: force is accepted but ignored

    let (m, _plain, _creds) = match machine_bmc(&state, id).await {
        Ok(v) => v,
        Err((code, msg)) => {
            let _ = code;
            return Ok(Json(session::ConsoleSessionResponse {
                ok: false,
                mode: "none".into(),
                session_id: None,
                embed_url: None,
                idrac_console_url: None,
                shared: None,
                viewers: None,
                error: Some(msg),
            }));
        }
    };

    let secret = &state.config.auth.jwt_secret;
    let idrac_url = session::idrac_console_url(&m.bmc_address);
    let prefer_idrac = idrac::looks_like_dell(&m.bmc_type);

    if prefer_idrac {
        match idrac::open_console_session(&m, secret).await {
            Ok((sid, embed, shared)) => {
                audit(&state, &claims.sub, "idrac_console_open", &id.to_string(), &sid).await;
                return Ok(Json(session::ConsoleSessionResponse {
                    ok: true,
                    mode: "idrac".into(),
                    session_id: Some(sid),
                    embed_url: Some(embed),
                    idrac_console_url: Some(idrac_url),
                    shared: Some(shared),
                    viewers: Some(1),
                    error: None,
                }));
            }
            Err(e) => {
                audit(&state, &claims.sub, "idrac_console_fail", &id.to_string(), &m.bmc_address).await;
                return Ok(Json(session::ConsoleSessionResponse {
                    ok: false,
                    mode: "none".into(),
                    session_id: None,
                    embed_url: None,
                    idrac_console_url: Some(idrac_url),
                    shared: None,
                    viewers: None,
                    error: Some(format!("iDRAC virtual console unavailable: {e}")),
                }));
            }
        }
    }

    // HPE iLO: mint an iLO HTML5 console session (embed_path is relative so the
    // browser loads it same-origin, avoiding mixed-content from a stale
    // advertised_url).
    match session::open_console_session(&m, secret).await {
        Ok((sid, embed, shared)) => {
            audit(&state, &claims.sub, "ilo_console_open", &id.to_string(), &sid).await;
            Ok(Json(session::ConsoleSessionResponse {
                ok: true,
                mode: "ilo".into(),
                session_id: Some(sid),
                embed_url: Some(embed),
                idrac_console_url: None,
                shared: Some(shared),
                viewers: Some(1),
                error: None,
            }))
        }
        Err(ilo_err) => {
            // iLO login failed — this might actually be a Dell box typed as
            // auto/ipmi. Try the iDRAC HTML5 path before SOL.
            match idrac::open_console_session(&m, secret).await {
                Ok((sid, embed, shared)) => {
                    audit(&state, &claims.sub, "idrac_console_open", &id.to_string(), &sid).await;
                    Ok(Json(session::ConsoleSessionResponse {
                        ok: true,
                        mode: "idrac".into(),
                        session_id: Some(sid),
                        embed_url: Some(embed),
                        idrac_console_url: Some(idrac_url),
                        shared: Some(shared),
                        viewers: Some(1),
                        error: None,
                    }))
                }
                Err(_) => Ok(Json(session::ConsoleSessionResponse {
                    ok: true,
                    mode: "sol".into(),
                    session_id: None,
                    embed_url: None,
                    idrac_console_url: None,
                    shared: None,
                    viewers: None,
                    error: Some(format!("iLO console unavailable; using SOL: {ilo_err}")),
                })),
            }
        }
    }
}

/// GET/POST /machines/:machine_id/console/:sid/{path}  — BMC asset proxy
/// (iLO or iDRAC). WebSocket upgrades on this catch-all are the iDRAC KVM
/// relay; iLO KVM still has a dedicated `/wss/ircport` route.
pub async fn ilo_asset(
    State(state): State<AppState>,
    Path((machine_id, sid)): Path<(Uuid, String)>,
    request: Request,
) -> Result<Response, (StatusCode, String)> {
    if idrac::is_safe_session_id(&sid) {
        return idrac_asset(state, machine_id, sid, request).await;
    }

    // Capture the raw path before the request is consumed by the body/headers.
    let full_path = request.uri().path().to_string();
    let headers = request.headers().clone();
    let path = full_path
        .rsplit_once(&format!("/console/{sid}/"))
        .map(|(_, rest)| rest.trim_start_matches('/').to_string())
        .unwrap_or_default();

    let sess = session::get_session(&sid)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "console session not found".to_string()))?;
    if sess.machine_id != machine_id {
        return Err((StatusCode::NOT_FOUND, "console session not found".into()));
    }

    let rel = if path.trim_end_matches('/').is_empty() {
        "irc.html".to_string()
    } else {
        path.trim_start_matches('/').to_string()
    };
    if !asset::is_safe_console_path(&rel) {
        return Err((StatusCode::BAD_REQUEST, "invalid path".into()));
    }

    let req_method = headers
        .get("x-request-method")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_ascii_uppercase())
        .unwrap_or_else(|| "GET".to_string());

    // Answer the console's login session lookup locally (GET or POST) so the
    // browser never re-hits iLO for it and the BMC password stays server-side.
    // Set the sessionKey cookie here too (defensive: the initial irc.html load
    // sets it, but if login_session is the first call the cookie must exist for
    // iLO.getSessionKey() to feed startHtml5Irc).
    if rel.trim_end_matches('/') == "json/login_session" {
        let (st, body, ctype) =
            asset::shared_login_response(&sess.bmc_host, &sess.username, &sess.session_key, None);
        let cookie = format!("sessionKey={}; Path=/; SameSite=Lax", sess.session_key);
        return asset::into_response(st, ctype, body.into_bytes(), Some(cookie))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    let (status, ctype, mut content) = asset::fetch_ilo_asset(
        &sess.bmc_host,
        &rel,
        &sess.session_key,
        &req_method,
        None,
        &headers,
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let prefix = session::session_prefix(&machine_id.to_string(), &sid);
    if status == 200
        && (ctype.contains("javascript") || ctype == "text/html" || rel.ends_with(".html") || rel == "irc.html")
    {
        content = asset::apply_rewrites(&rel, &content, &prefix);
    }

    let set_cookie = if rel == "irc.html" || rel.ends_with(".html") {
        Some(format!("sessionKey={}; Path=/; SameSite=Lax", sess.session_key))
    } else {
        None
    };

    asset::into_response(status, ctype, content, set_cookie)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn idrac_asset(
    _state: AppState,
    machine_id: Uuid,
    sid: String,
    request: Request,
) -> Result<Response, (StatusCode, String)> {
    let sess = idrac::get_session(&sid)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "console session not found".to_string()))?;
    if sess.machine_id != machine_id {
        return Err((StatusCode::NOT_FOUND, "console session not found".into()));
    }

    let full_path = request.uri().path().to_string();
    let query = request.uri().query().map(|s| s.to_string());
    let rel = idrac_proxy::parse_rel_from_uri(&full_path, &sid);
    if !idrac_proxy::is_safe_idrac_path(&rel) {
        return Err((StatusCode::BAD_REQUEST, "invalid path".into()));
    }

    let headers = request.headers().clone();
    let is_ws = idrac_proxy::is_websocket_request(&headers) || rel.starts_with("__rfb/");
    if is_ws {
        let proto = headers
            .get("sec-websocket-protocol")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let (mut parts, _body) = request.into_parts();
        let ws = WebSocketUpgrade::from_request_parts(&mut parts, &())
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("websocket upgrade: {e}")))?;
        let rel_owned = rel.clone();
        return Ok(ws.on_upgrade(move |socket| async move {
            idrac_proxy::relay_ws(sess, socket, rel_owned, query, proto).await;
        }));
    }

    let method = request.method().clone();
    let body = axum::body::to_bytes(request.into_body(), idrac_proxy::max_body())
        .await
        .map_err(|e| (StatusCode::PAYLOAD_TOO_LARGE, format!("body: {e}")))?;
    let body = if body.is_empty() { None } else { Some(body.to_vec()) };

    let (status, ctype, mut content, new_cookies, pass_headers) = idrac_proxy::fetch_upstream(
        &sess,
        &method,
        &rel,
        query.as_deref(),
        &headers,
        body,
    )
    .await
    .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    if !new_cookies.is_empty() {
        idrac::merge_session_cookies(&sid, new_cookies.clone());
    }

    let prefix = session::session_prefix(&machine_id.to_string(), &sid);
    content = idrac_proxy::apply_rewrites(&rel, &ctype, &content, &prefix, &sess.bmc_host);

    let set_cookies: Vec<String> = new_cookies
        .iter()
        .map(|(n, v)| idrac_proxy::rewrite_set_cookie(n, v, &prefix))
        .collect();

    idrac_proxy::into_response(status, ctype, content, set_cookies, pass_headers)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// GET /machines/:machine_id/console/:sid/wss/ircport  — iLO KVM WebSocket relay.
pub async fn ilo_kvm_ws(
    State(state): State<AppState>,
    Path((machine_id, sid)): Path<(Uuid, String)>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    let sess = session::get_session(&sid)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "console session not found".to_string()))?;
    if sess.machine_id != machine_id {
        return Err((StatusCode::NOT_FOUND, "console session not found".into()));
    }
    Ok(ws.on_upgrade(move |socket| run_kvm(state, sess, socket)))
}

async fn run_kvm(state: AppState, sess: session::IloSession, socket: WebSocket) {
    kvm::run_kvm(state, sess, socket).await;
}

/// GET /machines/:id/console/sol?token=…  — Dell SOL WebSocket bridge.
///
/// Auth is enforced by the RBAC middleware (it reads `?token=`); the handler
/// only re-reads the token for audit attribution. Follows the `exec_ws` pattern.
#[derive(Deserialize)]
pub struct SolQuery {
    #[serde(default)]
    token: Option<String>,
}

pub async fn sol_ws(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<SolQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, (StatusCode, String)> {
    // The RBAC middleware already validated the token + role for this route
    // (it reads `?token=` the same way), so reaching here means the caller is
    // authorized. Re-read the token only for audit attribution; if it's absent
    // here we still proceed, logging without a subject.
    let claims = q.token.as_deref().and_then(|t| verify_jwt(t).ok()).map(|t| t.claims);
    if let Some(c) = &claims {
        if c.role != "admin" && c.role != "operator" {
            return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
        }
    }

    let (m, plain, creds) = match machine_bmc(&state, id).await {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    let ipmi = IpmiClient::new(&creds).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let child = match ipmi.sol_activate().await {
        Ok(c) => c,
        Err(e) => {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("SOL unavailable: {e} (iDRAC SOL may be disabled)"),
            ))
        }
    };

    let sub = claims.map(|c| c.sub).unwrap_or_else(|| "unknown".into());
    audit(&state, &sub, "sol_console_open", &id.to_string(), &m.bmc_address).await;
    let _ = plain;
    Ok(ws.on_upgrade(move |socket| run_sol(socket, child, m.bmc_address.clone(), id)))
}

/// Bridge a browser WebSocket to the `ipmitool sol activate` child.
async fn run_sol(mut socket: WebSocket, mut child: tokio::process::Child, bmc: String, machine_id: Uuid) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    // stdout + stderr -> browser (SOL data is on stdout; ipmitool may emit
    // diagnostics on stderr, e.g. the tcgetattr warning when piped).
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let mut readers = Vec::new();
    if let Some(r) = stdout {
        let tx = out_tx.clone();
        readers.push(tokio::spawn(async move { pump_reader(r, tx).await }));
    }
    if let Some(r) = stderr {
        let tx = out_tx.clone();
        readers.push(tokio::spawn(async move { pump_reader(r, tx).await }));
    }
    drop(out_tx); // close the sender once both readers own their clones

    // browser -> stdin
    let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    let in_task = tokio::spawn(async move {
        if let Some(mut w) = stdin {
            while let Some(data) = in_rx.recv().await {
                if w.write_all(&data).await.is_err() {
                    break;
                }
                let _ = w.flush().await;
            }
        }
    });

    // Drive the bridge: select on browser input AND pending stdout so SOL
    // output reaches the browser immediately (the original code only flushed
    // stdout after receiving a browser message, so the initial console screen
    // never appeared until the user typed).
    'outer: loop {
        tokio::select! {
            msg = socket.recv() => match msg {
                Some(Ok(Message::Binary(b))) => {
                    if in_tx.send(b.to_vec()).await.is_err() { break; }
                }
                Some(Ok(Message::Text(t))) => {
                    if in_tx.send(t.as_str().as_bytes().to_vec()).await.is_err() { break; }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
            bytes = out_rx.recv() => match bytes {
                Some(b) => {
                    if socket.send(Message::Binary(b.into())).await.is_err() { break; }
                }
                // out_rx closed (readers finished) -> drain stdin, end session.
                None => {
                    let _ = in_tx.send(Vec::new()).await;
                    break 'outer;
                }
            },
        }
    }

    // Tear down: stop the readers and the stdin writer, kill the child.
    for t in readers {
        t.abort();
    }
    in_task.abort();
    let _ = child.kill().await;
    let _ = machine_id;
    let _ = bmc;
}

/// Pump a piped stream into `tx` until EOF/error.
async fn pump_reader<R: tokio::io::AsyncRead + Unpin + Send>(
    mut reader: R,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// POST /machines/:id/console/session/close?sid=…  — best-effort iLO logout.
#[derive(Deserialize)]
pub struct CloseQuery {
    sid: String,
}
pub async fn close_console_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<CloseQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _claims = claims_from(&headers, None).map_err(|(s, _)| (s, "auth error".to_string()))?;
    // Verify the session belongs to this machine before closing.
    if let Some(sess) = session::get_session(&q.sid) {
        if sess.machine_id == id {
            session::close_session(&q.sid).await;
            audit(&state, "operator", "ilo_console_close", &id.to_string(), &q.sid).await;
        }
    } else if let Some(sess) = idrac::get_session(&q.sid) {
        if sess.machine_id == id {
            idrac::close_session(&q.sid).await;
            audit(&state, "operator", "idrac_console_close", &id.to_string(), &q.sid).await;
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}
