//! oob-agent: dials out to TCS over WebSocket and relays Redfish BMC ops.
//!
//! Usage:
//!   oob-agent --server wss://tcs.example.com --token pxj_<hex> [--label my-site]
//!
//! The agent holds no BMC credentials: each `bmc.op` frame carries the
//! credentials for that machine, and the agent executes the single operation
//! against the local Redfish endpoint, then replies with a `resp` frame.

use std::time::Duration;

use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use talos_control_system::integration::bmc::{BootTarget, BmcSession, PowerState};
use talos_control_system::network::tunnel::BmcOp;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use tracing::{info, warn};

#[derive(Parser, Debug)]
#[command(name = "oob-agent", about = "Remote OOB BMC relay agent")]
struct Args {
    /// TCS WebSocket endpoint, e.g. wss://tcs.example.com/api/proxy/tunnel
    #[arg(long, env = "OOB_AGENT_SERVER")]
    server: String,

    /// Join token (pxj_...) issued from TCS Settings → Proxy.
    #[arg(long, env = "OOB_AGENT_TOKEN")]
    token: String,

    /// Optional human label reported to TCS.
    #[arg(long, env = "OOB_AGENT_LABEL")]
    label: Option<String>,

    /// Reconnect backoff between attempts.
    #[arg(long, default_value_t = 5u64)]
    backoff_secs: u64,
}

/// A framed operation from TCS: { "type": "bmc.op", "opId": "...", "op": {...} }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpFrame {
    #[serde(rename = "opId")]
    op_id: String,
    op: BmcOp,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();
    let url = format!("{}?token={}", args.server.trim_end_matches('?'), args.token);

    loop {
        match run(&args, &url).await {
            Ok(()) => info!("connection closed by peer; reconnecting"),
            Err(e) => {
                warn!(error = %e, "connection failed; retrying");
            }
        }
        tokio::time::sleep(Duration::from_secs(args.backoff_secs)).await;
    }
}

async fn run(args: &Args, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (ws, _resp) = connect_async(url).await?;
    info!("connected to TCS tunnel");
    let (mut ws_tx, mut ws_rx) = ws.split();

    // Announce capabilities so TCS can display what this agent supports.
    let hello = serde_json::json!({
        "type": "hello",
        "caps": ["redfish"],
        "label": args.label,
    });
    ws_tx
        .send(WsMessage::Text(hello.to_string()))
        .await?;

    // Pump: read frames, execute ops, send results.
    while let Some(msg) = ws_rx.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => return Err(e.into()),
        };
        let text = match msg {
            WsMessage::Text(t) => t,
            WsMessage::Ping(p) => {
                ws_tx.send(WsMessage::Pong(p)).await?;
                continue;
            }
            WsMessage::Close(_) => return Ok(()),
            _ => continue,
        };

        let frame: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if frame.get("type").and_then(|t| t.as_str()) != Some("bmc.op") {
            continue;
        }
        let op_frame: OpFrame = match serde_json::from_value(frame) {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "malformed op frame");
                continue;
            }
        };

        let result = execute(&op_frame.op).await;
        let resp = serde_json::json!({
            "type": "resp",
            "opId": op_frame.op_id,
            "ok": result.ok,
            "error": result.error,
            "powerState": result.power_state,
        });
        if ws_tx.send(WsMessage::Text(resp.to_string())).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

struct OpOutcome {
    ok: bool,
    error: Option<String>,
    power_state: Option<String>,
}

async fn execute(op: &BmcOp) -> OpOutcome {
    let sess = match BmcSession::connect(&op.creds).await {
        Ok(s) => s,
        Err(e) => return OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
    };

    match op.op.as_str() {
        "power" => {
            let action = op.action.clone().unwrap_or_default();
            match sess.power(&action).await {
                Ok(()) => OpOutcome { ok: true, error: None, power_state: None },
                Err(e) => OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
            }
        }
        "set_boot" => {
            let target = match op.target.as_deref() {
                Some("pxe") => BootTarget::Pxe,
                _ => BootTarget::Disk,
            };
            let once = op.once.unwrap_or(false);
            match sess.set_boot(target, once).await {
                Ok(()) => OpOutcome { ok: true, error: None, power_state: None },
                Err(e) => OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
            }
        }
        "get_power_state" => {
            match sess.get_power_state().await {
                Ok(state) => OpOutcome {
                    ok: true,
                    error: None,
                    power_state: Some(state_str(state)),
                },
                Err(e) => OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
            }
        }
        "mount_iso" => {
            let iso = op.iso_url.clone().unwrap_or_default();
            let media = op.media.clone().unwrap_or_default();
            match sess.mount_iso(&iso, &media).await {
                Ok(()) => OpOutcome { ok: true, error: None, power_state: None },
                Err(e) => OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
            }
        }
        "unmount_iso" => {
            let media = op.media.clone().unwrap_or_default();
            match sess.unmount_iso(&media).await {
                Ok(()) => OpOutcome { ok: true, error: None, power_state: None },
                Err(e) => OpOutcome { ok: false, error: Some(e.to_string()), power_state: None },
            }
        }
        other => OpOutcome {
            ok: false,
            error: Some(format!("unknown op: {other}")),
            power_state: None,
        },
    }
}

fn state_str(s: PowerState) -> String {
    match s {
        PowerState::On => "on".into(),
        PowerState::Off => "off".into(),
        PowerState::Unknown => "unknown".into(),
    }
}
