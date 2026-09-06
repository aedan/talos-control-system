//! Reverse-proxy the iDRAC HTML5 / eHTML5 virtual console from TCS's origin.
//!
//! The operator's browser never talks to the BMC: TCS fetches viewer assets
//! with the session cookie jar, strips frame-busting headers, rewrites
//! absolute BMC URLs onto the session prefix, and injects a small script that
//! patches `fetch` / XHR / `WebSocket` / `Worker` so runtime requests stay
//! under the TCS path. WebSocket upgrades are bridged transparently (the
//! viewer itself speaks VNC-over-WS or Avocent).
//!
//! A dedicated `{prefix}/__rfb/{port}` WebSocket maps viewers that still dial
//! the remote-presence port (5900/5901) onto a TLS (then plain) upstream.

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TwsMsg;

use super::idrac::{gui_client, IdracSession, CHROME_UA};
use super::session::bmc_origin;
use super::tls::insecure_connector;
use crate::AppError;

const MAX_BODY: usize = 16 * 1024 * 1024;

/// Paths the iDRAC viewer commonly requests with a leading slash. Rewritten
/// onto the TCS session prefix in HTML/JS/CSS so parser-loaded assets resolve.
const ABS_PREFIXES: &[&str] = &[
    "/console",
    "/html5.html",
    "/login.html",
    "/login",
    "/restgui",
    "/sysmgmt",
    "/redfish",
    "/public",
    "/Applications",
    "/ws",
    "/data",
    "/cgi-bin",
    "/session",
    "/vconsole",
    "/viewer",
    "/eHTML5",
    "/VConsole",
    "/kvm",
    "/wsman",
    "/help",
    "/images",
    "/css",
    "/js",
    "/lib",
    "/app",
    "/locale",
    "/resources",
    "/favicon",
];

pub fn is_safe_idrac_path(path: &str) -> bool {
    let t = path.trim();
    if t.is_empty() {
        return true;
    }
    if t.contains('\\') || t.split('/').any(|seg| seg == "..") {
        return false;
    }
    // `__rfb/5900` is our synthetic KVM-port relay.
    if let Some(rest) = t.strip_prefix("__rfb/") {
        return rest.chars().all(|c| c.is_ascii_digit()) && rest.len() <= 5;
    }
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '$' | '~' | '+'))
}

/// Build the injected rewrite script + `<base>` for a viewer HTML document.
pub fn inject_prefix_hooks(html: &str, prefix: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    let script = format!(
        r#"<script>(function(){{var P="{pfx}";function f(u){{if(typeof u!=="string")return u;if(u.charAt(0)==="/"&&u.indexOf(P)!==0&&u.indexOf("//")!==0)return P+u;return u;}}try{{var OF=window.fetch;window.fetch=function(u,o){{if(typeof u==="string")u=f(u);else if(u&&u.url){{try{{u=new Request(f(u.url),u)}}catch(e){{}}}}return OF.call(this,u,o);}};var xo=XMLHttpRequest.prototype.open;XMLHttpRequest.prototype.open=function(m,u){{arguments[1]=f(u);return xo.apply(this,arguments);}};var OW=window.WebSocket;window.WebSocket=function(u,p){{try{{var x=new URL(u,location.href);x.protocol=location.protocol==="https:"?"wss:":"ws:";if(x.port==="5900"||x.port==="5901"||x.port==="5902"){{x.host=location.host;x.pathname=P+"/__rfb/"+x.port;}}else{{x.host=location.host;if(x.pathname.indexOf(P)!==0)x.pathname=P+x.pathname;}}u=x.toString();}}catch(e){{}}return p!==undefined?new OW(u,p):new OW(u);}};window.WebSocket.prototype=OW.prototype;window.WebSocket.CONNECTING=OW.CONNECTING;window.WebSocket.OPEN=OW.OPEN;window.WebSocket.CLOSING=OW.CLOSING;window.WebSocket.CLOSED=OW.CLOSED;if(window.Worker){{var Wr=window.Worker;window.Worker=function(u,o){{return new Wr(f(u),o);}};}}var sa=HTMLElement.prototype.setAttribute;HTMLElement.prototype.setAttribute=function(n,v){{if((n==="src"||n==="href"||n==="action")&&typeof v==="string")v=f(v);return sa.call(this,n,v);}};}}catch(e){{}}}})();</script>"#
    );
    let base = format!("<base href=\"{pfx}/\">");
    let hook = format!("{script}{base}");
    let lower = html.to_ascii_lowercase();
    if let Some(idx) = lower.find("<head") {
        if let Some(close) = html[idx..].find('>') {
            let pos = idx + close + 1;
            let mut out = String::with_capacity(html.len() + hook.len());
            out.push_str(&html[..pos]);
            out.push_str(&hook);
            out.push_str(&html[pos..]);
            return out;
        }
    }
    format!("{hook}{html}")
}

pub fn rewrite_absolute_urls(text: &str, prefix: &str, bmc_host: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    let origin = bmc_origin(bmc_host);
    let mut t = text.to_string();
    // Absolute BMC origin → session prefix (http and https, with/without port).
    for scheme in ["https://", "http://", "wss://", "ws://"] {
        let needle = format!("{scheme}{bmc_host}");
        t = t.replace(&needle, pfx);
        // host may be stored as host:port already; also try without default 443.
        if let Some((h, _)) = bmc_host.rsplit_once(':') {
            let n2 = format!("{scheme}{h}");
            t = t.replace(&n2, pfx);
        }
    }
    t = t.replace(&origin, pfx);

    for abs in ABS_PREFIXES {
        let from_dq = format!("\"{abs}");
        let to_dq = format!("\"{pfx}{abs}");
        t = t.replace(&from_dq, &to_dq);
        let from_sq = format!("'{abs}");
        let to_sq = format!("'{pfx}{abs}");
        t = t.replace(&from_sq, &to_sq);
    }
    let doubled = format!("{pfx}{pfx}");
    t = t.replace(&doubled, pfx);

    // HTML attributes: src="/foo" (not protocol-relative src="//")
    t = rewrite_attr_slash_urls(&t, pfx);
    t
}

fn rewrite_attr_slash_urls(text: &str, pfx: &str) -> String {
    let re = regex::Regex::new(
        r#"(?i)(\b(?:src|href|action|poster|data-src|data-href)\s*=\s*["'])(/[^"']*)"#,
    )
    .expect("attr rewrite regex");
    re.replace_all(text, |caps: &regex::Captures| {
        let val = &caps[2];
        if val.starts_with("//") || val.starts_with(pfx) {
            caps[0].to_string()
        } else {
            format!("{}{}{}", &caps[1], pfx, val)
        }
    })
    .into_owned()
}

pub fn apply_rewrites(path: &str, ctype: &str, body: &[u8], prefix: &str, bmc_host: &str) -> Vec<u8> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let is_text = ctype.contains("javascript")
        || ctype.contains("ecmascript")
        || ctype.contains("json")
        || ctype.contains("html")
        || ctype.contains("css")
        || ctype.contains("xml")
        || ctype.contains("text/")
        || name.ends_with(".js")
        || name.ends_with(".html")
        || name.ends_with(".css")
        || name.ends_with(".json")
        || path.is_empty()
        || path == "console"
        || path.starts_with("console?");
    if !is_text {
        return body.to_vec();
    }
    let mut text = String::from_utf8_lossy(body).into_owned();
    text = rewrite_absolute_urls(&text, prefix, bmc_host);
    let is_html = ctype.contains("html")
        || name.ends_with(".html")
        || path.is_empty()
        || path == "console"
        || path.starts_with("console?")
        || text.trim_start().to_ascii_lowercase().starts_with("<!doctype")
        || text.trim_start().to_ascii_lowercase().starts_with("<html");
    if is_html {
        text = inject_prefix_hooks(&text, prefix);
        text = inject_heartbeat(&text);
    }
    text.into_bytes()
}

fn inject_heartbeat(text: &str) -> String {
    const HEARTBEAT: &str = r#"<script>(function(){function p(e){try{parent.postMessage({tcsIdrac:e||"alive"},"*")}catch(x){}}p("ready");setInterval(function(){p("alive")},2000);window.addEventListener("pagehide",function(){p("down")});})();</script>"#;
    if text.contains("tcsIdrac") {
        return text.to_string();
    }
    if let Some(idx) = text.rfind("</body>").or_else(|| text.rfind("</BODY>")) {
        let mut out = text.to_string();
        out.insert_str(idx, HEARTBEAT);
        return out;
    }
    format!("{text}{HEARTBEAT}")
}

pub async fn fetch_upstream(
    sess: &IdracSession,
    method: &Method,
    rel: &str,
    query: Option<&str>,
    extra: &HeaderMap,
    body: Option<Vec<u8>>,
) -> Result<(u16, String, Vec<u8>, Vec<(String, String)>), AppError> {
    let origin = bmc_origin(&sess.bmc_host);
    let rel = rel.trim_start_matches('/');
    let mut url = if rel.is_empty() {
        format!("{}{}", origin, sess.viewer_path)
    } else {
        format!("{origin}/{rel}")
    };
    if let Some(q) = query {
        if !url.contains('?') {
            url.push('?');
            url.push_str(q);
        }
    }

    let client = gui_client(sess.use_native_tls)?;
    let mut req = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    );
    req = req.header("User-Agent", CHROME_UA);
    req = req.header("Accept", extra.get(header::ACCEPT).and_then(|h| h.to_str().ok()).unwrap_or("*/*"));
    if let Some(c) = sess.cookie_header() {
        req = req.header("Cookie", c);
    }
    if let Some(t) = &sess.x_auth_token {
        req = req.header("X-Auth-Token", t);
    }
    // URL host is the BMC address, which is what iDRAC 9 HostHeaderCheck wants.
    req = req.header("Origin", &origin);
    req = req.header("Referer", format!("{origin}/"));
    for name in [
        "content-type",
        "xsrf-token",
        "x-requested-with",
        "x-csrf-token",
        "accept-language",
    ] {
        if let Some(v) = extra.get(name).and_then(|h| h.to_str().ok()) {
            if !v.is_empty() {
                req = req.header(name, v);
            }
        }
    }
    if let Some(b) = body {
        req = req.body(b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iDRAC fetch {url}: {e}")))?;
    let status = resp.status().as_u16();
    let set_cookies = super::idrac::collect_cookies(resp.headers());
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("application/octet-stream").trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Network(format!("iDRAC body: {e}")))?
        .to_vec();
    Ok((status, ctype, bytes, set_cookies))
}

pub fn into_response(
    status: u16,
    ctype: String,
    body: Vec<u8>,
    set_cookies: Vec<String>,
) -> Result<Response, AppError> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    builder = builder.header(header::CONTENT_TYPE, ctype);
    builder = builder.header("X-Content-Type-Options", "nosniff");
    builder = builder.header("X-Frame-Options", "SAMEORIGIN");
    builder = builder.header("Referrer-Policy", "no-referrer");
    // Drop upstream CSP/frame-busting; this document is served from TCS.
    let mut res = builder
        .body(Body::from(body))
        .map_err(|e| AppError::Internal(format!("iDRAC asset response: {e}")))?;
    for c in set_cookies {
        if let Ok(v) = HeaderValue::from_str(&c) {
            res.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    Ok(res)
}

/// Rewrite an upstream Set-Cookie so it is scoped to the TCS session prefix.
pub fn rewrite_set_cookie(raw_name: &str, raw_value: &str, prefix: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    format!("{raw_name}={raw_value}; Path={pfx}/; SameSite=Lax")
}

pub fn parse_rel_from_uri(full_path: &str, sid: &str) -> String {
    match full_path.rsplit_once(&format!("/console/{sid}")) {
        Some((_, rest)) => rest.trim_start_matches('/').to_string(),
        None => String::new(),
    }
}

pub fn is_websocket_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

pub async fn relay_ws(session: IdracSession, socket: WebSocket, rel: String, query: Option<String>, proto: Option<String>) {
    let origin = bmc_origin(&session.bmc_host);
    let ws_origin = origin
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    let upstream = if let Some(port) = rel.strip_prefix("__rfb/") {
        // Prefer TLS on the remote-presence port, then plain WS.
        let tls = format!("wss://{}:{port}/", host_only(&session.bmc_host));
        let plain = format!("ws://{}:{port}/", host_only(&session.bmc_host));
        match connect_ws(&tls, &session, &origin, proto.as_deref()).await {
            Ok(s) => s,
            Err(e1) => match connect_ws(&plain, &session, &origin, proto.as_deref()).await {
                Ok(s) => s,
                Err(e2) => {
                    tracing::warn!(error1 = %e1, error2 = %e2, "iDRAC RFB upstream failed");
                    let _ = socket;
                    return;
                }
            },
        }
    } else {
        let mut url = format!("{}/{}", ws_origin.trim_end_matches('/'), rel.trim_start_matches('/'));
        if let Some(q) = &query {
            if !url.contains('?') {
                url.push('?');
                url.push_str(q);
            }
        }
        match connect_ws(&url, &session, &origin, proto.as_deref()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, url = %url, "iDRAC KVM upstream connect failed");
                let _ = socket;
                return;
            }
        }
    };

    bridge(socket, upstream).await;
}

fn host_only(host: &str) -> String {
    host.split('/').next().unwrap_or(host).to_string()
}

async fn connect_ws(
    url: &str,
    session: &IdracSession,
    origin: &str,
    subprotocol: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    {
        let h = req.headers_mut();
        if let Some(c) = session.cookie_header() {
            if let Ok(v) = c.parse() {
                h.insert("Cookie", v);
            }
        }
        if let Some(t) = &session.x_auth_token {
            if let Ok(v) = t.parse() {
                h.insert("X-Auth-Token", v);
            }
        }
        if let Ok(v) = origin.parse() {
            h.insert("Origin", v);
        }
        h.insert("User-Agent", CHROME_UA.parse().unwrap());
        if let Some(p) = subprotocol {
            if let Ok(v) = p.parse() {
                h.insert("Sec-WebSocket-Protocol", v);
            }
        }
    }
    let connector = insecure_connector();
    match connect_async_tls_with_config(req, None, false, connector).await {
        Ok((stream, _resp)) => Ok(stream),
        Err(e) => Err(format!("iDRAC WS {url}: {e}")),
    }
}

async fn bridge(
    socket: WebSocket,
    up: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) {
    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut up_tx, mut up_rx) = up.split();

    let to_up = tokio::spawn(async move {
        while let Some(Ok(msg)) = browser_rx.next().await {
            let out = match msg {
                Message::Binary(b) => TwsMsg::Binary(b.to_vec().into()),
                Message::Text(t) => TwsMsg::Text(t.as_str().to_string().into()),
                Message::Ping(p) => TwsMsg::Ping(p.to_vec().into()),
                Message::Pong(p) => TwsMsg::Pong(p.to_vec().into()),
                Message::Close(_) => break,
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let to_browser = tokio::spawn(async move {
        while let Some(Ok(msg)) = up_rx.next().await {
            let out = match msg {
                TwsMsg::Binary(b) => Message::Binary(b.to_vec().into()),
                TwsMsg::Text(t) => Message::Text(t.to_string().into()),
                TwsMsg::Ping(p) => Message::Ping(p.to_vec().into()),
                TwsMsg::Pong(p) => Message::Pong(p.to_vec().into()),
                TwsMsg::Close(_) => break,
                TwsMsg::Frame(_) => continue,
            };
            if browser_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let _ = to_browser.await;
    to_up.abort();
}

pub fn max_body() -> usize {
    MAX_BODY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_safety() {
        assert!(is_safe_idrac_path(""));
        assert!(is_safe_idrac_path("console"));
        assert!(is_safe_idrac_path("restgui/vue/index.html"));
        assert!(is_safe_idrac_path("__rfb/5900"));
        assert!(!is_safe_idrac_path("../etc/passwd"));
        assert!(!is_safe_idrac_path("foo/../../bar"));
        assert!(!is_safe_idrac_path("__rfb/nope"));
    }

    #[test]
    fn rewrite_attrs_and_abs() {
        let html = r#"<script src="/restgui/app.js"></script><a href="/console">x</a>"#;
        let out = rewrite_absolute_urls(html, "/api/machines/m/console/idrac_abc", "10.0.0.5");
        assert!(out.contains("/api/machines/m/console/idrac_abc/restgui/app.js"));
        assert!(out.contains("/api/machines/m/console/idrac_abc/console"));
    }

    #[test]
    fn inject_has_prefix() {
        let html = "<html><head></head><body></body></html>";
        let out = inject_prefix_hooks(html, "/api/machines/m/console/idrac_abc");
        assert!(out.contains(r#"var P="/api/machines/m/console/idrac_abc""#));
        assert!(out.contains(r#"<base href="/api/machines/m/console/idrac_abc/">"#));
    }

    #[test]
    fn rel_from_uri() {
        let p = "/api/machines/111/console/idrac_abc/restgui/foo.js";
        assert_eq!(parse_rel_from_uri(p, "idrac_abc"), "restgui/foo.js");
        let p = "/api/machines/111/console/idrac_abc";
        assert_eq!(parse_rel_from_uri(p, "idrac_abc"), "");
    }
}
