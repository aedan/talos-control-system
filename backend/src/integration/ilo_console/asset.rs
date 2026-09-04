//! iLO console asset proxy + URL rewrites.
//!
//! Serves iLO's `irc.html` console and its assets **from TCS's origin**, fetching
//! them server-side with the session's iLO `session_key` cookie and rewriting the
//! JS so the browser's KVM WebSocket and relative `json/`/`rest/` fetches stay
//! under the TCS session prefix (not the iLO host). This defeats iLO's
//! `X-Frame-Options: sameorigin` — the framed content is same-origin with TCS.
//!
//! Port of genestack-console's `ilo_console.py` asset path (single-viewer).

use std::collections::HashMap;
use std::sync::{Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;

use super::session::{bmc_host, http_client};
use crate::AppError;

const ASSET_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const JSON_CACHE_TTL: Duration = Duration::from_secs(3);

// --- Rewrite strings (verbatim from iLO's shipped console JS) ------------------
//
// iLO's js/socket.js builds the KVM socket as:
//     this.sockaddr = "wss://" + options.host + "/wss/ircport"
// i.e. it points at the iLO host (self-signed cert, cross-origin). We rewrite it
// to target TCS's session-scoped relay. `options.host` is already the TCS host
// (iLO sets it to window.location.host), so we only need to swap the path for the
// literal session prefix (baked in at proxy time — no `document` access, which
// would be undefined if socket.js runs in a Worker).
const SOCKADDR_OLD: &str = r#"this.sockaddr = "wss://" + options.host + "/wss/ircport""#;

// renderer.js: in an iframe, iLO sets path="../" so Worker("js/worker_decoder.js")
// resolves outside the session prefix and 404s (black KVM canvas). Force it to "".
const IFRAME_REL_OLD: &str = r#"window.top === window.self ? "" : "../""#;
const IFRAME_REL_NEW: &str = r#""""#;

// iLO.js forces a leading slash on relative json/rest URLs, which would escape
// the session prefix. Keep the .match(...) suffix checks; only prepend our prefix.
const JSON_PREFIXER_OLD: &str = r#""json/" == my_url.match("^json/") && (my_url = "/" + my_url)"#;
const REST_PREFIXER_OLD: &str = r#""rest/" == my_url.match("^rest/") && (my_url = "/" + my_url)"#;

const HEARTBEAT_SCRIPT: &str = r#"<script>(function(){function p(e){try{parent.postMessage({tcsIlo:e||"alive"},"*")}catch(x){}}p("ready");setInterval(function(){p("alive")},2000);window.addEventListener("pagehide",function(){p("down")});})();</script>"#;

struct CacheEntry {
    at: Instant,
    status: u16,
    ctype: String,
    body: Vec<u8>,
}

type Cache = Mutex<HashMap<(String, String, String), CacheEntry>>;

static CACHE: std::sync::LazyLock<Cache> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_key(origin: &str, method: &str, path: &str) -> (String, String, String) {
    (origin.to_string(), method.to_string(), path.to_string())
}

fn is_static(name: &str) -> bool {
    let fname = name.rsplit('/').next().unwrap_or("").to_lowercase();
    [".js", ".css", ".png", ".svg", ".gif", ".ico", ".woff", ".woff2", ".ttf", ".html", ".map"]
        .iter()
        .any(|ext| fname.ends_with(ext))
}

/// Reject path traversal / absolute / suspicious paths (we only serve iLO console
/// assets relative to the iLO origin).
pub fn is_safe_console_path(path: &str) -> bool {
    let t = path.trim();
    if t.is_empty() {
        return true;
    }
    if t.starts_with('/') || t.contains('\\') || t.split('/').any(|seg| seg == "..") {
        return false;
    }
    if t.contains(['?', ':', '#', '%', '@']) {
        return false;
    }
    if t == "irc.html" || t == "favicon.ico" {
        return true;
    }
    // json/... or rest/... API calls the console makes
    if (t.starts_with("json/") || t.starts_with("rest/") || t.starts_with("blob/"))
        && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '$' || c == '-' || c == '/')
    {
        return true;
    }
    // Static asset path segments
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' || c == '/')
        && !t.contains("..")
}

/// Fetch an iLO asset/JSON/REST call server-side. Returns (status, content-type,
/// body). Caches static + json GETs.
pub async fn fetch_ilo_asset(
    bmc_addr: &str,
    path: &str,
    session_key: &str,
    method: &str,
    body: Option<&[u8]>,
    extra_headers: &HeaderMap,
) -> Result<(u16, String, Vec<u8>), AppError> {
    let origin = if bmc_addr.starts_with("http://") || bmc_addr.starts_with("https://") {
        bmc_addr.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", bmc_host(bmc_addr))
    };
    let rel = path.trim_start_matches('/') ;
    let rel = if rel.is_empty() { "irc.html" } else { rel };
    let method = method.to_uppercase();
    let key = cache_key(&origin, &method, rel);

    let json_get = method == "GET" && rel.starts_with("json/");
    let ttl = if json_get { JSON_CACHE_TTL } else { ASSET_CACHE_TTL };

    {
        let cache = CACHE.lock().unwrap();
        if is_static(rel) || json_get {
            if let Some(e) = cache.get(&key) {
                if e.at.elapsed() < ttl {
                    return Ok((e.status, e.ctype.clone(), e.body.clone()));
                }
            }
        }
    }

    let client = http_client().await?;
    let url = format!("{origin}/{rel}");
    let mut req = client.request(
        reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    );
    req = req.header("Cookie", format!("sessionKey={session_key}"));
    for name in ["content-type", "x-auth-token", "x-client-type", "accept"] {
        if let Some(v) = extra_headers.get(name).and_then(|h| h.to_str().ok()) {
            if !v.is_empty() {
                req = req.header(name, v);
            }
        }
    }
    if method != "GET" && method != "HEAD" {
        if let Some(b) = body {
            req = req.body(b.to_vec());
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iLO fetch failed: {e}")))?;
    let status = resp.status().as_u16();
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("application/octet-stream").trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    let content = resp
        .bytes()
        .await
        .map_err(|e| AppError::Network(format!("iLO fetch body: {e}")))?
        .to_vec();

    if status == 200 && (is_static(rel) || json_get) {
        let mut cache = CACHE.lock().unwrap();
        cache.insert(key, CacheEntry {
            at: Instant::now(),
            status,
            ctype: ctype.clone(),
            body: content.clone(),
        });
    }
    Ok((status, ctype, content))
}

/// Answer the console's json/login_session locally so the browser never re-hits
/// the iLO (and so the BMC password never reaches the browser).
pub fn shared_login_response(bmc_addr: &str, username: &str, session_key: &str, _body: Option<&[u8]>) -> (u16, String, String) {
    let _ = bmc_addr;
    let out = serde_json::json!({
        "session_key": session_key,
        "user_name": username,
        "remote_cons_priv": 1,
    });
    (200, out.to_string(), "application/json".into())
}

/// Apply the JS rewrites + heartbeat to a proxied asset body.
pub fn apply_rewrites(path: &str, body: &[u8], prefix: &str) -> Vec<u8> {
    let name = path.rsplit('/').next().unwrap_or("irc.html").to_string();
    if name.ends_with(".js") || name == "irc.html" || name.is_empty() {
        let mut text = String::from_utf8_lossy(body).into_owned();
        if name == "socket.js" || text.contains("sockaddr") {
            text = rewrite_socket_js(&text, prefix);
        }
        if text.contains(IFRAME_REL_OLD) || text.contains("worker_decoder") {
            text = text.replace(IFRAME_REL_OLD, IFRAME_REL_NEW);
        }
        if text.contains("my_url") || text.contains("/json/") || text.contains("favicon.ico") {
            let pfx = prefix.trim_end_matches('/');
            text = text.replace(
                JSON_PREFIXER_OLD,
                &format!(r#""json/" == my_url.match("^json/") && (my_url = "{pfx}/" + my_url)"#),
            );
            text = text.replace(
                REST_PREFIXER_OLD,
                &format!(r#""rest/" == my_url.match("^rest/") && (my_url = "{pfx}/" + my_url)"#),
            );
            text = text.replace("href=\"/favicon.ico", &format!("href=\"{pfx}/favicon.ico"));
            text = text.replace("href='/favicon.ico", &format!("href='{pfx}/favicon.ico"));
        }
        if name == "irc.html" || name.is_empty() {
            text = inject_base(&text, prefix);
            text = inject_heartbeat(&text);
        }
        return text.into_bytes();
    }
    body.to_vec()
}

fn rewrite_socket_js(text: &str, prefix: &str) -> String {
    // Bake the session prefix in as a literal path. options.host is the TCS host.
    let pfx = prefix.trim_end_matches('/');
    let new = format!(
        r#"this.sockaddr = (self.location.protocol==="https:"?"wss://":"ws://") + options.host + "{pfx}/wss/ircport""#
    );
    if text.contains(SOCKADDR_OLD) {
        return text.replace(SOCKADDR_OLD, &new);
    }
    // Fallback: match any wss:// + options.host + /wss/ircport form.
    let re = regex::Regex::new(r#"this\.sockaddr\s*=\s*"wss://"\s*\+\s*options\.host\s*\+\s*"/wss/ircport""#).unwrap();
    re.replace(text, &new).into_owned()
}

/// Inject a `<base>` tag so iLO's relative asset refs (`css/…`, `js/…`,
/// `lang/…`) resolve under the session path instead of the bare `/console/`
/// segment. iLO's `irc.html` uses unqualified relative paths; with the console
/// served at `/console/{sid}` they would resolve to `/console/{asset}` (dropping
/// the session id) and hit RBAC -> 401. A `<base href="{prefix}/">` pins them to
/// `/console/{sid}/{asset}`, which the session-gated asset route serves.
fn inject_base(text: &str, prefix: &str) -> String {
    if text.contains("<base") || text.contains("<BASE") {
        return text.to_string();
    }
    let base_tag = format!("<base href=\"{prefix}/\">");
    // Insert right after the <head> open tag (case-insensitive); fall back to
    // the top of the document if not found.
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.find("<head") {
        if let Some(close) = text[idx..].find('>') {
            let pos = idx + close + 1;
            let mut out = String::with_capacity(text.len() + base_tag.len());
            out.push_str(&text[..pos]);
            out.push_str(&base_tag);
            out.push_str(&text[pos..]);
            return out;
        }
    }
    format!("{base_tag}{text}")
}

fn inject_heartbeat(text: &str) -> String {
    if text.contains("tcsIlo") {
        return text.to_string();
    }
    if let Some(idx) = text.rfind("</body>") {
        let mut out = text.to_string();
        out.insert_str(idx, HEARTBEAT_SCRIPT);
        return out;
    }
    if let Some(idx) = text.rfind("</BODY>") {
        let mut out = text.to_string();
        out.insert_str(idx, HEARTBEAT_SCRIPT);
        return out;
    }
    format!("{text}{HEARTBEAT_SCRIPT}")
}

/// Build the axum Response for a proxied iLO asset.
pub fn into_response(
    status: u16,
    ctype: String,
    body: Vec<u8>,
    set_cookie: Option<String>,
) -> Result<Response, AppError> {
    let mut builder = Response::builder().status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    builder = builder.header(header::CONTENT_TYPE, ctype);
    builder = builder.header("X-Content-Type-Options", "nosniff");
    builder = builder.header("X-Frame-Options", "SAMEORIGIN");
    builder = builder.header("Referrer-Policy", "no-referrer");
    if let Some(c) = set_cookie {
        builder = builder.header(header::SET_COOKIE, c);
    }
    builder
        .body(Body::from(body))
        .map_err(|e| AppError::Internal(format!("iLO asset response: {e}")))
}
