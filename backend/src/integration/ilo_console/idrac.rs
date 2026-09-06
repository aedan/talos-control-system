//! iDRAC HTML5 / eHTML5 virtual-console sessions.
//!
//! Dell's viewer cannot be reached from the operator's browser (BMC subnet,
//! self-signed cert, `X-Frame-Options`, Host-header check). TCS logs into the
//! iDRAC with the stored BMC credentials, mints a Direct-Launch `/console`
//! URL (Redfish `GetKVMSession` temp credentials when available), and reverse-
//! proxies the viewer + its WebSockets from TCS's origin — the same model as
//! the iLO HTML5 console.
//!
//! Redfish (`/redfish/v1`) is not bot-gated; the GUI/viewer paths sometimes
//! are (TLS fingerprint / HTTP2). Viewer fetches therefore try a Chrome-like
//! HTTP/1.1 rustls client first, then OpenSSL (`native-tls`) if the GUI
//! returns 404.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use uuid::Uuid;

use super::session::{bmc_host, bmc_origin, session_prefix};
use crate::db::models::machine::Machine;
use crate::utils::secrets;
use crate::AppError;

const SESSION_TTL: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(25);
pub const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct IdracSession {
    pub session_id: String,
    pub machine_id: Uuid,
    pub hostname: String,
    pub bmc_host: String,
    pub username: String,
    /// Cookie jar to attach to upstream GUI/viewer requests.
    pub cookies: Vec<(String, String)>,
    pub x_auth_token: Option<String>,
    /// Path+query on the iDRAC to load as the viewer root (e.g. `/console?username=…`).
    pub viewer_path: String,
    /// Redfish session URI to DELETE on close (if we created one).
    pub redfish_session_uri: Option<String>,
    /// Use OpenSSL native-tls for GUI fetches (set when rustls was 404-gated).
    pub use_native_tls: bool,
    created_at: Instant,
}

impl IdracSession {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > SESSION_TTL
    }

    pub fn cookie_header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        Some(
            self.cookies
                .iter()
                .map(|(n, v)| format!("{n}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

type Store = Mutex<HashMap<String, IdracSession>>;

static SESSIONS: std::sync::LazyLock<Store> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn sessions() -> MutexGuard<'static, HashMap<String, IdracSession>> {
    SESSIONS.lock().unwrap()
}

fn purge_expired(g: &mut HashMap<String, IdracSession>) -> Vec<IdracSession> {
    let dead: Vec<(String, IdracSession)> = g
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

pub fn get_session(session_id: &str) -> Option<IdracSession> {
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
    g: &HashMap<String, IdracSession>,
    machine_id: Uuid,
) -> Option<&IdracSession> {
    g.values()
        .find(|v| v.machine_id == machine_id && !v.is_expired())
}

pub fn is_safe_session_id(id: &str) -> bool {
    id.starts_with("idrac_") && (8..=80).contains(&id.len())
}

/// Merge newly observed Set-Cookie values into a live session jar.
pub fn merge_session_cookies(session_id: &str, incoming: Vec<(String, String)>) {
    if incoming.is_empty() || !is_safe_session_id(session_id) {
        return;
    }
    let mut g = sessions();
    if let Some(sess) = g.get_mut(session_id) {
        sess.cookies = merge_cookies(sess.cookies.clone(), incoming);
    }
}

pub fn looks_like_dell(bmc_type: &str) -> bool {
    let t = bmc_type.trim().to_ascii_lowercase();
    t == "redfish" || t.contains("idrac") || t.contains("dell")
}

/// Open (or reuse) an iDRAC HTML5 console session.
///
/// Returns `(session_id, embed_path, reused)`. `embed_path` is relative so the
/// iframe stays same-origin with TCS.
pub async fn open_console_session(
    machine: &Machine,
    jwt_secret: &str,
) -> Result<(String, String, bool), AppError> {
    {
        let mut g = sessions();
        if let Some(sess) = find_machine_session(&g, machine.id) {
            let embed = format!(
                "{}{}",
                session_prefix(&machine.id.to_string(), &sess.session_id),
                sess.viewer_path
            );
            return Ok((sess.session_id.clone(), embed, true));
        }
        let purged = purge_expired(&mut g);
        for sess in purged {
            tokio::spawn(async move { logout_idrac(&sess).await });
        }
    }

    let plain = match &machine.bmc_password_enc {
        Some(enc) => secrets::decrypt(jwt_secret, enc)?,
        None => return Err(AppError::InvalidInput("No BMC password stored".into())),
    };
    let host = bmc_host(&machine.bmc_address);
    if host.is_empty() {
        return Err(AppError::InvalidInput("No BMC address configured".into()));
    }

    let login = login_idrac(&host, &machine.bmc_username, &plain).await?;

    let session_id = format!("idrac_{}", Uuid::new_v4().simple());
    let embed = format!(
        "{}{}",
        session_prefix(&machine.id.to_string(), &session_id),
        login.viewer_path
    );

    let sess = IdracSession {
        session_id: session_id.clone(),
        machine_id: machine.id,
        hostname: machine.hostname.clone(),
        bmc_host: host,
        username: machine.bmc_username.clone(),
        cookies: login.cookies,
        x_auth_token: login.x_auth_token,
        viewer_path: login.viewer_path,
        redfish_session_uri: login.redfish_session_uri,
        use_native_tls: login.use_native_tls,
        created_at: Instant::now(),
    };
    {
        let mut g = sessions();
        g.insert(session_id.clone(), sess);
    }
    Ok((session_id, embed, false))
}

pub async fn close_session(session_id: &str) {
    if !is_safe_session_id(session_id) {
        return;
    }
    let removed = { sessions().remove(session_id) };
    if let Some(sess) = removed {
        logout_idrac(&sess).await;
    }
}

struct LoginResult {
    cookies: Vec<(String, String)>,
    x_auth_token: Option<String>,
    viewer_path: String,
    redfish_session_uri: Option<String>,
    use_native_tls: bool,
}

/// iDRAC 7/8 `/data/login` first (R720 / 12G), then iDRAC 9 session API, then
/// Redfish GetKVMSession Direct Launch.
async fn login_idrac(host: &str, username: &str, password: &str) -> Result<LoginResult, AppError> {
    let mut errors = Vec::new();

    match idrac7_data_login(host, username, password).await {
        Ok(r) => return Ok(r),
        Err(e) => {
            let msg = e.to_string();
            errors.push(format!("iDRAC7/8 login: {e}"));
            // Session-full / bad-password on /data/login means this *is* an
            // iDRAC 7/8. Falling through to iDRAC 9 / GetKVMSession only
            // burns more session slots.
            if msg.contains("session table full") || msg.contains("login rejected") {
                return Err(AppError::Network(errors.join("; ")));
            }
        }
    }
    match idrac9_session_login(host, username, password).await {
        Ok(r) => return Ok(r),
        Err(e) => errors.push(format!("iDRAC9 session: {e}")),
    }
    match redfish_kvm_launch(host, username, password).await {
        Ok(r) => return Ok(r),
        Err(e) => errors.push(format!("GetKVMSession: {e}")),
    }

    Err(AppError::Network(format!(
        "iDRAC HTML5 console login failed ({})",
        errors.join("; ")
    )))
}

/// Drop leftover GUI/Redfish sessions so `/data/login` has a free slot.
/// iDRAC 7 caps concurrent user sessions (often 4–8); TCS probes and failed
/// console opens leak them. DELETE via `/redfish/v1/Sessions` with basic auth.
async fn clear_idrac_sessions(origin: &str, username: &str, password: &str) {
    let client = match api_client() {
        Ok(c) => c,
        Err(_) => return,
    };
    let list = client
        .get(format!("{origin}/redfish/v1/Sessions"))
        .basic_auth(username, Some(password))
        .header("Accept", "application/json")
        .send()
        .await;
    let Ok(resp) = list else { return };
    if !resp.status().is_success() {
        return;
    }
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let members = body
        .get("Members")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    for m in members {
        let Some(id) = m.get("@odata.id").and_then(|v| v.as_str()) else {
            continue;
        };
        let url = if id.starts_with("http") {
            id.to_string()
        } else {
            format!("{origin}{id}")
        };
        let _ = client
            .delete(&url)
            .basic_auth(username, Some(password))
            .send()
            .await;
    }
}

async fn redfish_kvm_launch(
    host: &str,
    username: &str,
    password: &str,
) -> Result<LoginResult, AppError> {
    let origin = bmc_origin(host);
    let client = api_client()?;

    let (token, session_uri, cookies) =
        redfish_create_session(&client, &origin, username, password).await?;

    let action = discover_kvm_action(&client, &origin, &token).await?;
    let session_name = format!("{:x}", Uuid::new_v4().as_u128());
    let session_name: String = session_name.chars().take(32).collect();
    let body = serde_json::json!({ "SessionTypeName": session_name });

    let mut req = client.post(&action).json(&body).header("X-Auth-Token", &token);
    if let Some(c) = cookie_header(&cookies) {
        req = req.header("Cookie", c);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Network(format!("GetKVMSession request: {e}")))?;
    let status = resp.status().as_u16();
    let more_cookies = merge_cookies(cookies, collect_cookies(resp.headers()));
    let payload: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    if status >= 400 {
        return Err(AppError::Network(format!(
            "GetKVMSession HTTP {status}: {payload}"
        )));
    }
    let temp_user = payload
        .get("TempUsername")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let temp_pass = payload
        .get("TempPassword")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if temp_user.is_empty() || temp_pass.is_empty() {
        return Err(AppError::Network(
            "GetKVMSession did not return TempUsername/TempPassword".into(),
        ));
    }

    let viewer_path = format!(
        "/console?username={}&tempUsername={}&tempPassword={}",
        urlencoding::encode(username),
        urlencoding::encode(&temp_user),
        urlencoding::encode(&temp_pass),
    );

    let (use_native_tls, cookies) =
        probe_viewer(&origin, &viewer_path, &more_cookies, Some(&token)).await?;

    Ok(LoginResult {
        cookies,
        x_auth_token: Some(token),
        viewer_path,
        redfish_session_uri: session_uri,
        use_native_tls,
    })
}

async fn redfish_create_session(
    client: &reqwest::Client,
    origin: &str,
    username: &str,
    password: &str,
) -> Result<(String, Option<String>, Vec<(String, String)>), AppError> {
    let url = format!("{origin}/redfish/v1/SessionService/Sessions");
    let body = serde_json::json!({ "UserName": username, "Password": password });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("Redfish session: {e}")))?;
    let status = resp.status();
    if !status.is_success() && status.as_u16() != 201 {
        return Err(AppError::Network(format!(
            "Redfish session HTTP {}",
            status.as_u16()
        )));
    }
    let token = resp
        .headers()
        .get("x-auth-token")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Network("Redfish session missing X-Auth-Token".into()))?;
    let loc = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|h| h.to_str().ok())
        .map(|s| {
            if s.starts_with("http") {
                s.to_string()
            } else if s.starts_with('/') {
                format!("{origin}{s}")
            } else {
                format!("{origin}/{s}")
            }
        });
    let cookies = collect_cookies(resp.headers());
    Ok((token, loc, cookies))
}

async fn discover_kvm_action(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
) -> Result<String, AppError> {
    const FALLBACKS: &[&str] = &[
        "/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/DelliDRACCardService/Actions/DelliDRACCardService.GetKVMSession",
        "/redfish/v1/Dell/Managers/iDRAC.Embedded.1/DelliDRACCardService/Actions/DelliDRACCardService.GetKVMSession",
        "/redfish/v1/Managers/iDRAC.Embedded.1/Actions/Oem/EID_674_Manager.GetKVMSession",
    ];

    let svc_urls = [
        format!("{origin}/redfish/v1/Managers/iDRAC.Embedded.1/Oem/Dell/DelliDRACCardService"),
        format!("{origin}/redfish/v1/Dell/Managers/iDRAC.Embedded.1/DelliDRACCardService"),
    ];
    for url in svc_urls {
        if let Ok(resp) = client.get(&url).header("X-Auth-Token", token).send().await {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<serde_json::Value>().await {
                    if let Some(target) = v
                        .pointer("/Actions/#DelliDRACCardService.GetKVMSession/target")
                        .or_else(|| v.pointer("/Actions/Oem/#DelliDRACCardService.GetKVMSession/target"))
                        .and_then(|x| x.as_str())
                    {
                        if target.starts_with("http") {
                            return Ok(target.to_string());
                        }
                        if target.starts_with('/') {
                            return Ok(format!("{origin}{target}"));
                        }
                        return Ok(format!("{origin}/{target}"));
                    }
                    if let Some(obj) = v.get("Actions").and_then(|a| a.as_object()) {
                        for (k, val) in obj {
                            if k.contains("GetKVMSession") {
                                if let Some(t) = val.get("target").and_then(|x| x.as_str()) {
                                    if t.starts_with("http") {
                                        return Ok(t.to_string());
                                    }
                                    if t.starts_with('/') {
                                        return Ok(format!("{origin}{t}"));
                                    }
                                    return Ok(format!("{origin}/{t}"));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(format!("{origin}{}", FALLBACKS[0]))
}

async fn idrac9_session_login(
    host: &str,
    username: &str,
    password: &str,
) -> Result<LoginResult, AppError> {
    let origin = bmc_origin(host);
    let client = gui_client(false)?;
    let url = format!("{origin}/sysmgmt/2015/bmc/session");
    let body = serde_json::json!({ "username": username, "password": password });
    let resp = client
        .post(&url)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iDRAC9 session: {e}")))?;
    let status = resp.status().as_u16();
    let token = resp
        .headers()
        .get("x-auth-token")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let cookies = collect_cookies(resp.headers());
    if status >= 400 {
        // Retry with `user` key used by some firmware.
        let body = serde_json::json!({ "user": username, "password": password });
        let resp = gui_client(false)?
            .post(&url)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Network(format!("iDRAC9 session: {e}")))?;
        let status = resp.status().as_u16();
        let token = resp
            .headers()
            .get("x-auth-token")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let cookies = collect_cookies(resp.headers());
        if status >= 400 {
            return Err(AppError::Network(format!("iDRAC9 session HTTP {status}")));
        }
        return finish_gui_login(origin, cookies, token, "/console").await;
    }
    if token.is_none() && cookies.is_empty() {
        return Err(AppError::Network(
            "iDRAC9 session did not return a token or cookies".into(),
        ));
    }
    finish_gui_login(origin, cookies, token, "/console").await
}

/// iDRAC 7/8 web login (`POST /data/login`).
///
/// 12G iDRAC 7 (R720, firmware 2.x) 404s every HTML asset unless the client
/// sends `Accept-Encoding: gzip` — the pages are stored gzipped. After a
/// successful login the authenticated UI is `/index.html?ST1=…,ST2=…` (the
/// tokens come from `<forwardUrl>`). `/console` only redirects at the *login*
/// page (`/start.html?console`), so it must not be used as the embed target.
async fn idrac7_data_login(
    host: &str,
    username: &str,
    password: &str,
) -> Result<LoginResult, AppError> {
    let origin = bmc_origin(host);
    // Free leaked GUI sessions *before* the first /data/login. Trying rustls
    // then OpenSSL after a "table full" error just burns more slots.
    clear_idrac_sessions(&origin, username, password).await;

    let mut last_tls_err = None;
    for native in [false, true] {
        match idrac7_data_login_with(native, &origin, username, password).await {
            Ok(r) => return Ok(r),
            Err(e) => {
                let msg = e.to_string();
                last_tls_err = Some(e);
                if msg.contains("session table full") || msg.contains("login rejected") {
                    break;
                }
            }
        }
    }
    Err(last_tls_err.unwrap_or_else(|| AppError::Network("iDRAC7/8 login failed".into())))
}

async fn idrac7_data_login_with(
    native: bool,
    origin: &str,
    username: &str,
    password: &str,
) -> Result<LoginResult, AppError> {
    let client = gui_client(native)?;

    let form = format!(
        "user={}&password={}",
        urlencoding::encode(username),
        urlencoding::encode(password)
    );
    let mut cookies = Vec::new();
    let mut text = String::new();
    for attempt in 0..2 {
        let resp = client
            .post(format!("{origin}/data/login"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/xml,text/xml,text/html;q=0.9,*/*;q=0.8")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", format!("{origin}/start.html?console"))
            .body(form.clone())
            .send()
            .await
            .map_err(|e| AppError::Network(format!("iDRAC7/8 login: {e}")))?;
        let status = resp.status().as_u16();
        cookies = merge_cookies(cookies, collect_cookies(resp.headers()));
        text = resp.text().await.unwrap_or_default();
        if status >= 400 {
            return Err(AppError::Network(format!("iDRAC7/8 login HTTP {status}")));
        }
        let result = auth_result(&text);
        if result == Some(5) && attempt == 0 {
            clear_idrac_sessions(origin, username, password).await;
            cookies.clear();
            continue;
        }
        break;
    }

    match auth_result(&text) {
        Some(0) => {}
        Some(1) | Some(99) => {
            return Err(AppError::Network("iDRAC7/8 login rejected".into()));
        }
        Some(5) => {
            return Err(AppError::Network(
                "iDRAC session table full; close another console and retry".into(),
            ));
        }
        Some(n) => {
            return Err(AppError::Network(format!(
                "iDRAC7/8 login authResult={n}"
            )));
        }
        None => {
            if !text.to_ascii_lowercase().contains("forwardurl") {
                return Err(AppError::Network(
                    "iDRAC7/8 login did not return a session".into(),
                ));
            }
        }
    }

    let (st1, st2) = parse_st_tokens(&text).ok_or_else(|| {
        AppError::Network("iDRAC7/8 login missing ST1/ST2 session tokens".into())
    })?;
    // index.html JS parses ST1/ST2 out of window.location.href; ST2 must be last.
    let viewer_path = format!("/index.html?ST1={st1},ST2={st2}");

    match probe_viewer(origin, &viewer_path, &cookies, None).await {
        Ok((use_native_tls, cookies)) => Ok(LoginResult {
            cookies,
            x_auth_token: None,
            viewer_path,
            redfish_session_uri: None,
            use_native_tls: use_native_tls || native,
        }),
        Err(e) => {
            let _ = client
                .get(format!("{origin}/data/logout"))
                .header("Cookie", cookie_header(&cookies).unwrap_or_default())
                .send()
                .await;
            Err(e)
        }
    }
}

fn auth_result(xml: &str) -> Option<u32> {
    let lower = xml.to_ascii_lowercase();
    let start = lower.find("<authresult>")? + "<authresult>".len();
    let end = lower[start..].find("</authresult>")?;
    lower[start..start + end].trim().parse().ok()
}

fn parse_st_tokens(xml: &str) -> Option<(String, String)> {
    // forwardUrl is `index.html?ST1=<hex>,ST2=<hex>`
    let re = regex::Regex::new(r"ST1=([0-9a-fA-F]+),ST2=([0-9a-fA-F]+)").ok()?;
    let caps = re.captures(xml)?;
    Some((caps[1].to_string(), caps[2].to_string()))
}

async fn finish_gui_login(
    origin: String,
    cookies: Vec<(String, String)>,
    token: Option<String>,
    preferred: &str,
) -> Result<LoginResult, AppError> {
    let candidates = [preferred, "/console", "/html5.html", "/public/html5.html"];
    let mut last_err = AppError::Network("no iDRAC viewer path responded".into());
    for path in candidates {
        match probe_viewer(&origin, path, &cookies, token.as_deref()).await {
            Ok((use_native_tls, cookies)) => {
                return Ok(LoginResult {
                    cookies,
                    x_auth_token: token,
                    viewer_path: path.to_string(),
                    redfish_session_uri: None,
                    use_native_tls,
                });
            }
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Fetch the viewer path with a browser-like client. Returns whether native-tls
/// was required and the (possibly updated) cookie jar.
async fn probe_viewer(
    origin: &str,
    path: &str,
    cookies: &[(String, String)],
    token: Option<&str>,
) -> Result<(bool, Vec<(String, String)>), AppError> {
    let mut last = AppError::Network("viewer probe failed".into());
    for native in [false, true] {
        match probe_viewer_with(native, origin, path, cookies, token).await {
            Ok(c) => return Ok((native, c)),
            Err(e) => last = e,
        }
    }
    Err(last)
}

async fn probe_viewer_with(
    native: bool,
    origin: &str,
    path: &str,
    cookies: &[(String, String)],
    token: Option<&str>,
) -> Result<Vec<(String, String)>, AppError> {
    let client = gui_client(native)?;
    let url = format!("{}{}", origin.trim_end_matches('/'), path);
    let mut req = client
        .get(&url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Upgrade-Insecure-Requests", "1");
    if let Some(c) = cookie_header(cookies) {
        req = req.header("Cookie", c);
    }
    if let Some(t) = token {
        req = req.header("X-Auth-Token", t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Network(format!("viewer GET {path}: {e}")))?;
    let status = resp.status().as_u16();
    let cookies = merge_cookies(cookies.to_vec(), collect_cookies(resp.headers()));
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.bytes().await.unwrap_or_default();
    if status == 404 || status == 403 {
        return Err(AppError::Network(format!(
            "viewer {path} HTTP {status} (tls={})",
            if native { "openssl" } else { "rustls" }
        )));
    }
    if status >= 400 {
        return Err(AppError::Network(format!("viewer {path} HTTP {status}")));
    }
    // A real viewer is HTML (or a redirect already followed). Empty/JSON 200 is not it.
    let looks_html = ctype.contains("html")
        || body.windows(5).any(|w| w.eq_ignore_ascii_case(b"<html"))
        || body.windows(9).any(|w| w.eq_ignore_ascii_case(b"<!doctype"));
    if !looks_html && status != 200 {
        return Err(AppError::Network(format!(
            "viewer {path} did not return HTML"
        )));
    }
    // Authenticated iDRAC 7 index.html is ~45k; the login page is ~3k. A
    // redirect to start.html means the session cookie did not stick.
    if path.contains("index.html") && body.len() < 8000 {
        return Err(AppError::Network(
            "iDRAC returned the login page instead of the authenticated UI".into(),
        ));
    }
    Ok(cookies)
}

pub async fn logout_idrac(sess: &IdracSession) {
    let origin = bmc_origin(&sess.bmc_host);
    let client = api_client().ok();
    if let (Some(client), Some(uri)) = (client.as_ref(), sess.redfish_session_uri.as_ref()) {
        let mut req = client.delete(uri);
        if let Some(t) = &sess.x_auth_token {
            req = req.header("X-Auth-Token", t);
        }
        if let Some(c) = sess.cookie_header() {
            req = req.header("Cookie", c);
        }
        let _ = req.timeout(Duration::from_secs(8)).send().await;
        return;
    }
    if let Ok(client) = gui_client(sess.use_native_tls) {
        let mut req = client.get(format!("{origin}/data/logout"));
        if let Some(c) = sess.cookie_header() {
            req = req.header("Cookie", c);
        }
        let _ = req.timeout(Duration::from_secs(8)).send().await;
    }
}

/// Redfish / JSON API client (not bot-gated).
pub fn api_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .http1_only()
        .user_agent("tcs-bmc/idrac")
        .build()
        .map_err(|e| AppError::Network(format!("iDRAC API client: {e}")))
}

/// GUI / viewer client. `native` selects OpenSSL instead of rustls so the
/// ClientHello looks less like a bot to iDRAC's GUI WAF.
pub fn gui_client(native: bool) -> Result<reqwest::Client, AppError> {
    let mut b = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .http1_only()
        .user_agent(CHROME_UA);
    if native {
        b = b.use_native_tls();
    }
    b.build()
        .map_err(|e| AppError::Network(format!("iDRAC GUI client: {e}")))
}

pub fn collect_cookies(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for val in headers.get_all(reqwest::header::SET_COOKIE) {
        if let Ok(s) = val.to_str() {
            if let Some((n, v)) = parse_set_cookie(s) {
                out.push((n, v));
            }
        }
    }
    out
}

fn parse_set_cookie(s: &str) -> Option<(String, String)> {
    let pair = s.split(';').next()?.trim();
    let (n, v) = pair.split_once('=')?;
    let n = n.trim();
    if n.is_empty() {
        return None;
    }
    Some((n.to_string(), v.trim().to_string()))
}

fn merge_cookies(
    mut existing: Vec<(String, String)>,
    incoming: Vec<(String, String)>,
) -> Vec<(String, String)> {
    for (n, v) in incoming {
        if let Some(slot) = existing.iter_mut().find(|(en, _)| en.eq_ignore_ascii_case(&n)) {
            slot.1 = v;
        } else {
            existing.push((n, v));
        }
    }
    existing
}

fn cookie_header(cookies: &[(String, String)]) -> Option<String> {
    if cookies.is_empty() {
        return None;
    }
    Some(
        cookies
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_shape() {
        assert!(is_safe_session_id("idrac_0123456789abcdef0123456789abcdef"));
        assert!(!is_safe_session_id("ilo_0123456789abcdef0123456789abcdef"));
        assert!(!is_safe_session_id("../idrac_x"));
    }

    #[test]
    fn dell_detection() {
        assert!(looks_like_dell("redfish"));
        assert!(looks_like_dell("iDRAC"));
        assert!(looks_like_dell("Dell-idrac9"));
        assert!(!looks_like_dell("auto"));
        assert!(!looks_like_dell("ipmi"));
        assert!(!looks_like_dell("ilo"));
    }

    #[test]
    fn set_cookie_parse() {
        let (n, v) = parse_set_cookie("_appwebSessionId_=abc; Path=/; HttpOnly").unwrap();
        assert_eq!(n, "_appwebSessionId_");
        assert_eq!(v, "abc");
        let (n, v) = parse_set_cookie("-http-session-=37::http.session::xyz; Path=/").unwrap();
        assert_eq!(n, "-http-session-");
        assert_eq!(v, "37::http.session::xyz");
    }

    #[test]
    fn parse_login_xml() {
        let xml = r#"<?xml version="1.0"?><root><status>ok</status><authResult>0</authResult><forwardUrl>index.html?ST1=abc123,ST2=def456</forwardUrl></root>"#;
        assert_eq!(auth_result(xml), Some(0));
        assert_eq!(
            parse_st_tokens(xml),
            Some(("abc123".into(), "def456".into()))
        );
        assert_eq!(
            auth_result("<authResult>5</authResult>"),
            Some(5)
        );
    }
}
