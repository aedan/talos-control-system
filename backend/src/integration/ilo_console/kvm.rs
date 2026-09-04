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
use rustls::ClientConfig;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TwsMsg;
use tokio_tungstenite::{Connector, WebSocketStream, MaybeTlsStream};

use super::session::bmc_origin;
use crate::AppState;
use crate::integration::ilo_console::session::IloSession;

// DVCNET command byte values (from iLO console / genestack).
const HELLO: u8 = 80; // DVCNET.CMD_AUTHENTICATE
const AUTH_OK: u8 = 82; // DVCNET.CMD_AUTHENTICATED
const BUSY: u8 = 83;
const SEIZE: u8 = 85;
const BUSY_NO_MURC: u8 = 89;

type Upstream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

fn upstream_url(bmc_host: &str) -> String {
    let origin = bmc_origin(bmc_host);
    let o = origin
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    format!("{}/wss/ircport", o.trim_end_matches('/'))
}

/// A rustls config that accepts self-signed iLO certs.
fn insecure_client_config() -> Option<ClientConfig> {
    Some(
        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(InsecureVerifier))
            .with_no_client_auth(),
    )
}

/// Accept any server certificate (iLO self-signed certs).
#[derive(Debug)]
struct InsecureVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::ED25519,
        ]
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
}

fn hello_frame(channel: u8, session_key: &str) -> Vec<u8> {
    let mut frame = vec![0u8; 34];
    frame[0] = channel;
    frame[1] = 32;
    let key = session_key.as_bytes();
    let n = key.len().min(32);
    frame[2..2 + n].copy_from_slice(&key[..n]);
    frame
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

    let connector = match insecure_client_config() {
        Some(cfg) => Some(Connector::Rustls(std::sync::Arc::new(cfg))),
        None => None,
    };
    match connect_async_tls_with_config(req, None, false, connector).await {
        Ok((stream, _resp)) => Ok(stream),
        Err(e) => Err(format!("iLO KVM connect {url}: {e}")),
    }
}

async fn recv_bytes(
    up: &mut Upstream,
) -> Result<Vec<u8>, String> {
    match up.next().await {
        Some(Ok(TwsMsg::Binary(b))) => Ok(b.to_vec()),
        Some(Ok(TwsMsg::Text(t))) => Ok(t.as_str().as_bytes().to_vec()),
        Some(Ok(_)) => Err("unexpected upstream frame type".into()),
        Some(Err(e)) => Err(e.to_string()),
        None => Err("upstream closed".into()),
    }
}

async fn bmc_handshake(up: &mut Upstream, session: &IloSession, channel: u8) -> Result<(), String> {
    let first = recv_bytes(up).await?;
    if first.is_empty() || first[0] != HELLO {
        return Err(format!(
            "iLO did not send AUTHENTICATE (got 0x{:02x})",
            first.first().copied().unwrap_or(0)
        ));
    }
    up.send(TwsMsg::Binary(hello_frame(channel, &session.session_key).into()))
        .await
        .map_err(|e| e.to_string())?;
    let auth = recv_bytes(up).await?;
    let status = auth.first().copied().unwrap_or(0);
    if status == BUSY || status == BUSY_NO_MURC {
        up.send(TwsMsg::Binary(vec![SEIZE, 0].into()))
            .await
            .map_err(|e| e.to_string())?;
        let auth2 = recv_bytes(up).await?;
        let s2 = auth2.first().copied().unwrap_or(0);
        if s2 != AUTH_OK {
            return Err(format!("iLO seize status {s2}"));
        }
        return Ok(());
    }
    if status != AUTH_OK {
        return Err(format!("iLO handshake status {status}"));
    }
    Ok(())
}

/// Relay one browser <-> iLO KVM session (single viewer).
pub async fn run_kvm(_state: AppState, session: IloSession, mut socket: WebSocket) {
    let url = upstream_url(&session.bmc_host);
    let mut up = match connect_upstream(&url, &session).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "iLO KVM upstream connect failed");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    const CHANNEL: u8 = 1; // KVM channel
    if let Err(e) = bmc_handshake(&mut up, &session, CHANNEL).await {
        tracing::warn!(error = %e, "iLO KVM handshake failed");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut up_tx, mut up_rx) = up.split();

    // browser -> upstream (HID keyboard/mouse).
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

    // upstream -> browser (video + CMD). Ends when the iLO drops the stream OR
    // the browser disconnects (a send to a closed socket fails) — so awaiting it
    // is the "session over" signal for both directions.
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
