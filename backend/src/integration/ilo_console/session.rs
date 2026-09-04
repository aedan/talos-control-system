//! In-memory iLO HTML5 console sessions.
//!
//! The operator's browser cannot reach iLO BMC addresses directly (they live on a
//! management subnet, and the iLO serves its console with a self-signed cert plus
//! `X-Frame-Options: sameorigin`). So TCS logs into the iLO **once** using the
//! stored BMC credentials, keeps the iLO `session_key`, and serves the iLO console
//! assets + KVM WebSocket through TCS's own origin (see `asset.rs` / `kvm.rs`).
//!
//! Sessions are in-memory (like the noVNC/k8s-exec streams): an unguessable
//! `ilo_…` id bound to a machine, holding the iLO JSON `session_key` so the BMC
//! password never leaves the server. One live session per machine; a second open
//! while live reuses it (single-viewer model).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::Uuid;

use crate::db::models::machine::Machine;
use crate::utils::secrets;
use crate::AppError;

const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const LOGIN_COOLDOWN: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// A single authenticated iLO console session.
#[derive(Clone)]
pub struct IloSession {
    pub session_id: String,
    pub machine_id: Uuid,
    pub hostname: String,
    pub bmc_host: String,
    pub username: String,
    /// iLO JSON `session_key` (cookie value). Never serialized to the browser.
    pub session_key: String,
    created_at: Instant,
    last_login_at: Instant,
}

impl IloSession {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > SESSION_TTL
    }
    pub fn in_login_cooldown(&self) -> bool {
        self.last_login_at.elapsed() < LOGIN_COOLDOWN
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleSessionResponse {
    pub ok: bool,
    pub mode: String, // "ilo" | "sol"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// TCS-origin URL the browser iframes for the iLO console.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_url: Option<String>,
    /// iDRAC HTML5 console URL (opened in a new tab) for Dell machines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idrac_console_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewers: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

type Store = Mutex<HashMap<String, IloSession>>;

static SESSIONS: std::sync::LazyLock<Store> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn sessions() -> MutexGuard<'static, HashMap<String, IloSession>> {
    SESSIONS.lock().unwrap()
}

fn purge_expired(g: &mut HashMap<String, IloSession>) -> Vec<IloSession> {
    let dead: Vec<(String, IloSession)> = g
        .iter()
        .filter(|(_, v)| v.is_expired())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut out = Vec::new();
    for (k, v) in dead {
        g.remove(&k);
        out.push(v);
    }
    out
}

/// Look up a live session by id (unguessable id is the auth gate).
pub fn get_session(session_id: &str) -> Option<IloSession> {
    if !is_safe_session_id(session_id) {
        return None;
    }
    let mut g = sessions();
    let sess = g.get(session_id)?;
    if sess.is_expired() {
        g.remove(session_id);
        None
    } else {
        Some(sess.clone())
    }
}

fn find_machine_session(
    g: &HashMap<String, IloSession>,
    machine_id: Uuid,
) -> Option<&IloSession> {
    g.values().find(|v| v.machine_id == machine_id && !v.is_expired())
}

/// Open (or reuse) the iLO console session for a machine.
///
/// Returns `(session_id, embed_path, reused)` on success, or an `Err`
/// describing why the iLO console could not be opened (caller may then fall
/// back to SOL). `embed_path` is *relative* (`/api/machines/{id}/console/{sid}`)
/// so the browser loads it same-origin — an absolute URL built from
/// `advertised_url` can be the wrong scheme/port (e.g. `http://…:8081`) and
/// gets blocked as mixed content inside the HTTPS TCS page.
pub async fn open_console_session(
    machine: &Machine,
    jwt_secret: &str,
) -> Result<(String, String, bool), AppError> {
    // Reuse a live session for this machine.
    {
        let mut g = sessions();
        if let Some(sess) = find_machine_session(&g, machine.id) {
            let prefix = session_prefix(&machine.id.to_string(), &sess.session_id);
            return Ok((sess.session_id.clone(), prefix, true));
        }
        let purged = purge_expired(&mut g);
        for sess in purged {
            let host = sess.bmc_host.clone();
            let key = sess.session_key.clone();
            tokio::spawn(async move { logout_iolo(&host, &key).await });
        }
    }

    // Fresh login.
    let plain = match &machine.bmc_password_enc {
        Some(enc) => secrets::decrypt(jwt_secret, enc)?,
        None => return Err(AppError::InvalidInput("No BMC password stored".into())),
    };
    let host = bmc_host(&machine.bmc_address);
    if host.is_empty() {
        return Err(AppError::InvalidInput("No BMC address configured".into()));
    }

    let session_key = json_login(&host, &machine.bmc_username, &plain).await?;

    let session_id = format!("ilo_{}", Uuid::new_v4().simple());
    let embed_path = session_prefix(&machine.id.to_string(), &session_id);

    let now = Instant::now();
    let sess = IloSession {
        session_id: session_id.clone(),
        machine_id: machine.id,
        hostname: machine.hostname.clone(),
        bmc_host: host.clone(),
        username: machine.bmc_username.clone(),
        session_key: session_key.clone(),
        created_at: now,
        last_login_at: now,
    };
    {
        let mut g = sessions();
        g.insert(session_id.clone(), sess);
    }
    Ok((session_id, embed_path, false))
}

/// Close a session (best-effort iLO logout to free the BMC slot).
pub async fn close_session(session_id: &str) {
    if !is_safe_session_id(session_id) {
        return;
    }
    let removed = { sessions().remove(session_id) };
    if let Some(sess) = removed {
        logout_iolo(&sess.bmc_host, &sess.session_key).await;
    }
}

/// Session URL prefix under which the browser loads the proxied iLO console.
///
/// The KVM WebSocket lives at `{prefix}/wss/ircport` and all iLO assets under
/// `{prefix}/…`, so everything is same-origin with TCS (defeating iLO's
/// `X-Frame-Options: sameorigin`). The asset-proxy handler must use this exact
/// prefix so the rewritten JS reassembles to the same URLs.
pub fn session_prefix(machine_id: &str, session_id: &str) -> String {
    format!("/api/machines/{machine_id}/console/{session_id}")
}

pub fn is_safe_session_id(id: &str) -> bool {
    id.starts_with("ilo_") && (8..=80).contains(&id.len())
}

/// iDRAC HTML5 console launch URL (new tab) for Dell machines.
pub fn idrac_console_url(bmc_address: &str) -> String {
    format!("https://{}/login.html", bmc_host(bmc_address))
}

/// Strip scheme/path/port from a stored BMC address, keep host[:port].
pub fn bmc_host(address: &str) -> String {
    let a = address.trim().trim_start_matches("https://").trim_start_matches("http://");
    a.split('/')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string()
}

/// iLO JSON login: POST /json/login_session -> session_key.
async fn json_login(host: &str, username: &str, password: &str) -> Result<String, AppError> {
    let origin = bmc_origin(host);
    let client = http_client().await?;
    let body = serde_json::json!({
        "method": "login",
        "user_login": username,
        "password": password,
    });
    let resp = client
        .post(format!("{origin}/json/login_session"))
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iLO login request failed: {e}")))?;
    let status = resp.status();
    let payload: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    if status.as_u16() >= 400 {
        let exhausted = payload_exhausted(status.as_u16(), &payload);
        return Err(AppError::Network(format!(
            "iLO login failed (HTTP {}){}",
            status.as_u16(),
            if exhausted {
                " — session table full; retry in a moment"
            } else {
                ""
            }
        )));
    }
    let key = payload
        .get("session_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Network("iLO login did not return a session_key".into()))?;
    Ok(key)
}

/// Best-effort iLO JSON logout (frees the BMC session slot).
pub async fn logout_iolo(host: &str, session_key: &str) {
    if session_key.is_empty() {
        return;
    }
    let origin = bmc_origin(host);
    if let Ok(client) = http_client().await {
        let _ = client
            .post(format!("{origin}/json/login_session"))
            .json(&serde_json::json!({
                "method": "logout",
                "session_key": session_key,
            }))
            .header("Cookie", format!("sessionKey={session_key}"))
            .timeout(Duration::from_secs(8))
            .send()
            .await;
    }
}

fn payload_exhausted(status: u16, payload: &serde_json::Value) -> bool {
    if status == 500 {
        return true;
    }
    let mut text = String::new();
    if let Some(obj) = payload.as_object() {
        for k in ["message", "details", "error"] {
            if let Some(v) = obj.get(k) {
                text.push_str(&v.to_string());
                text.push(' ');
            }
        }
    }
    let lowered = text.to_lowercase();
    lowered.contains("createlimitreached") || (lowered.contains("invalid login") && status >= 400)
}

pub fn bmc_origin(host: &str) -> String {
    let h = host.trim().trim_end_matches('/');
    if h.starts_with("http://") || h.starts_with("https://") {
        h.to_string()
    } else {
        format!("https://{h}")
    }
}

/// Shared reqwest client: ignore self-signed iLO certs, follow redirects.
pub async fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| AppError::Network(format!("iLO http client: {e}")))
}
