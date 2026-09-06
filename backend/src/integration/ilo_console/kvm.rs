//! iLO KVM WebSocket relay (single-viewer).
//!
//! iLO's HTML5 console opens a binary DVCNET WebSocket to `wss://<ilo>/wss/ircport`
//! for the live KVM stream. The browser can't reach the iLO directly (self-signed
//! cert, cross-origin, and the rewritten socket JS points at TCS instead), so TCS
//! opens the upstream iLO WS, completes the DVCNET handshake, and bridges bytes
//! both directions to the browser.
//!
//! Port of the handshake from genestack-console's `ilo_kvm.py` (single-viewer,
//! no fan-out).

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TwsMsg;
use tokio_tungstenite::{WebSocketStream, MaybeTlsStream};

use super::session::bmc_origin;
use super::tls::insecure_connector;
use crate::AppState;
use crate::integration::ilo_console::session::IloSession;

type Upstream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn upstream_url(bmc_host: &str) -> String {
    let origin = bmc_origin(bmc_host);
    let o = origin
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{}/wss/ircport", o.trim_end_matches('/'))
}

async fn connect_upstream(url: &str, session: &IloSession) -> Result<Upstream, String> {
    let origin = bmc_origin(&session.bmc_host);
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    {
        let h = req.headers_mut();
        h.insert(
            "Cookie",
            format!("sessionKey={}", session.session_key).parse().unwrap(),
        );
        h.insert("Origin", origin.parse().unwrap());
    }

    match connect_async_tls_with_config(req, None, false, insecure_connector()).await {
        Ok((stream, _resp)) => Ok(stream),
        Err(e) => Err(format!("iLO KVM connect {url}: {e}")),
    }
}

/// Relay one browser <-> iLO KVM session (single viewer).
///
/// The relay is **transparent**: iLO's HTML5 console performs the DVCNET
/// handshake *itself* in the browser (socket.js `sockrecv_auth` expects to
/// receive CMD_AUTHENTICATE(80), reply with the 34-byte session-key hello, and
/// receive CMD_AUTHENTICATED(82)). TCS must NOT do that handshake on the
/// server side — doing so consumes the 80 the browser needs and hands it video
/// bytes first, which iLO's `IRC_SERVER_HELLO` parses as a bogus status and
/// reports as "Handshake error". So we just bridge raw bytes both directions.
pub async fn run_kvm(_state: AppState, session: IloSession, mut socket: WebSocket) {
    let url = upstream_url(&session.bmc_host);
    let up = match connect_upstream(&url, &session).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "iLO KVM upstream connect failed");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut up_tx, mut up_rx) = up.split();

    // browser -> upstream (handshake hello + HID keyboard/mouse + video control).
    let to_up = tokio::spawn(async move {
        while let Some(Ok(msg)) = browser_rx.next().await {
            let bytes = match msg {
                Message::Binary(b) => b.to_vec(),
                Message::Text(t) => t.as_str().as_bytes().to_vec(),
                _ => break,
            };
            if up_tx.send(TwsMsg::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
    });

    // upstream -> browser (CMD_AUTHENTICATE, CMD_AUTHENTICATED, then video/CMD).
    // Ends when the iLO drops the stream OR the browser disconnects.
    let to_browser = tokio::spawn(async move {
        while let Some(Ok(msg)) = up_rx.next().await {
            if let TwsMsg::Binary(b) = msg {
                if browser_tx.send(Message::Binary(b.into())).await.is_err() {
                    break;
                }
            } else if matches!(msg, TwsMsg::Close(_)) {
                break;
            }
        }
    });

    let _ = to_browser.await;
    to_up.abort();
}
