//! Siderolink WireGuard control-plane side.
//!
//! Manages a host WG interface (`tcs-sl0` by default) via `wg` / `ip` CLI when available.
//! Registration returns peer config so Talos nodes (or a helper) can complete the tunnel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use base64::Engine;
use tracing::{info, warn};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::config::SideroLinkConfig;
use crate::AppError;

#[derive(Clone)]
pub struct SiderolinkWg {
    cfg: SideroLinkConfig,
    private_key: String,
    public_key: String,
    iface: String,
    enabled: bool,
}

impl SiderolinkWg {
    pub fn init(cfg: &SideroLinkConfig, data_dir: &str) -> Arc<Self> {
        let iface = std::env::var("TCS_SIDEROLINK_IFACE").unwrap_or_else(|_| "tcs-sl0".into());
        let key_path = PathBuf::from(data_dir).join("siderolink_wg_private.key");
        let (private_key, public_key) = load_or_create_keys(&key_path);
        let mut mgr = Self {
            cfg: cfg.clone(),
            private_key,
            public_key,
            iface,
            enabled: false,
        };
        match mgr.ensure_interface() {
            Ok(()) => {
                mgr.enabled = true;
                info!(
                    iface = %mgr.iface,
                    port = mgr.cfg.bind_port,
                    "Siderolink WireGuard interface ready"
                );
            }
            Err(e) => {
                warn!(
                    error = %e,
                    "Siderolink WireGuard not active (install wireguard-tools or run as root). Inventory registration still works."
                );
            }
        }
        Arc::new(mgr)
    }

    pub fn server_public_key(&self) -> &str {
        &self.public_key
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn listen_port(&self) -> u16 {
        self.cfg.bind_port
    }

    pub fn endpoint_hint(&self) -> String {
        // Host may advertise via env
        std::env::var("TCS_SIDEROLINK_ENDPOINT").unwrap_or_else(|_| {
            format!(
                "{}:{}",
                std::env::var("TCS_PUBLIC_HOST").unwrap_or_else(|_| "tcs.local".into()),
                self.cfg.bind_port
            )
        })
    }

    fn ensure_interface(&mut self) -> Result<(), AppError> {
        // ip link add tcs-sl0 type wireguard (ignore exists)
        let _ = run_cmd(&["ip", "link", "add", "dev", &self.iface, "type", "wireguard"]);
        // Assign server IP .1 on the CGNAT-ish subnet
        let server_ip = "100.64.0.1/10";
        let _ = run_cmd(&["ip", "address", "add", server_ip, "dev", &self.iface]);
        run_cmd(&[
            "wg",
            "set",
            &self.iface,
            "listen-port",
            &self.cfg.bind_port.to_string(),
            "private-key",
            "/dev/stdin",
        ])
        .or_else(|_| {
            // write key to temp and set
            let tmp = std::env::temp_dir().join("tcs-wg-key");
            fs::write(&tmp, format!("{}\n", self.private_key))?;
            let r = run_cmd(&[
                "wg",
                "set",
                &self.iface,
                "listen-port",
                &self.cfg.bind_port.to_string(),
                "private-key",
                tmp.to_str().unwrap_or(""),
            ]);
            let _ = fs::remove_file(&tmp);
            r
        })?;
        run_cmd(&["ip", "link", "set", "mtu", &self.cfg.mtu.to_string(), "dev", &self.iface])?;
        run_cmd(&["ip", "link", "set", "up", "dev", &self.iface])?;
        Ok(())
    }

    /// Add or update a peer on the WG interface.
    pub fn set_peer(&self, public_key: &str, allowed_ip: &str) -> Result<(), AppError> {
        if !self.enabled {
            return Ok(());
        }
        run_cmd(&[
            "wg",
            "set",
            &self.iface,
            "peer",
            public_key,
            "allowed-ips",
            &format!("{}/32", allowed_ip.trim_end_matches("/32")),
            "persistent-keepalive",
            "25",
        ])?;
        Ok(())
    }

    pub fn remove_peer(&self, public_key: &str) -> Result<(), AppError> {
        if !self.enabled {
            return Ok(());
        }
        let _ = run_cmd(&["wg", "set", &self.iface, "peer", public_key, "remove"]);
        Ok(())
    }
}

fn load_or_create_keys(path: &Path) -> (String, String) {
    if let Ok(priv_b64) = fs::read_to_string(path) {
        let priv_b64 = priv_b64.trim();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(priv_b64) {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                let secret = StaticSecret::from(arr);
                let public = PublicKey::from(&secret);
                let pub_b64 = base64::engine::general_purpose::STANDARD.encode(public.as_bytes());
                return (priv_b64.to_string(), pub_b64);
            }
        }
    }
    let secret = StaticSecret::random_from_rng(rand::rngs::OsRng);
    let public = PublicKey::from(&secret);
    let priv_b64 = base64::engine::general_purpose::STANDARD.encode(secret.to_bytes());
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(public.as_bytes());
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{}\n", priv_b64));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    (priv_b64, pub_b64)
}

fn run_cmd(args: &[&str]) -> Result<(), AppError> {
    let (bin, rest) = args
        .split_first()
        .ok_or_else(|| AppError::Internal("empty command".into()))?;
    let out = Command::new(bin)
        .args(rest)
        .output()
        .map_err(|e| AppError::Internal(format!("spawn {}: {}", bin, e)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Ignore "File exists" for ip address/link
        if stderr.contains("File exists") || stderr.contains("exists") {
            return Ok(());
        }
        return Err(AppError::Internal(format!(
            "{} failed: {}",
            args.join(" "),
            stderr.trim()
        )));
    }
    Ok(())
}
