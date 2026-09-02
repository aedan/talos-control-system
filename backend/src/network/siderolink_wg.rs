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
    installation_id: String,
}

impl SiderolinkWg {
    pub fn init(cfg: &SideroLinkConfig, data_dir: &str) -> Arc<Self> {
        let iface = std::env::var("TCS_SIDEROLINK_IFACE").unwrap_or_else(|_| "tcs-sl0".into());
        let installation_id =
            std::env::var("TCS_SIDEROLINK_INSTALLATION_ID").unwrap_or_else(|_| "tcs".into());
        let key_path = PathBuf::from(data_dir).join("siderolink_wg_private.key");
        let (private_key, public_key) = load_or_create_keys(&key_path);
        tracing::info!(
            key_path = %key_path.display(),
            public_key = %public_key,
            "Siderolink WG keypair loaded/created"
        );
        let mut mgr = Self {
            cfg: cfg.clone(),
            private_key,
            public_key,
            iface,
            enabled: false,
            installation_id,
        };
        match mgr.ensure_interface() {
            Ok(()) => {
                mgr.enabled = true;
                info!(
                    iface = %mgr.iface,
                    wg_port = mgr.cfg.listen_port,
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
        // The WireGuard *data* port nodes dial (separate from the SideroLink
        // gRPC API port, which is bind_port).
        self.cfg.listen_port
    }

    /// The server's own address on the SideroLink IPv6 ULA overlay.
    pub fn server_address(&self) -> String {
        crate::siderolink::address::server_address(&self.installation_id)
    }

    /// The /64 overlay network address the nodes join (`fd…::/64`): the first 8
    /// bytes of the installation-derived ULA prefix, tail zeroed.
    pub fn network_prefix(&self) -> String {
        let net = crate::siderolink::address::addr_from_prefix_bytes(
            crate::siderolink::address::network_prefix(&self.installation_id),
            [0; 8],
        );
        format!("{net}/64")
    }

    pub fn endpoint_hint(&self) -> String {
        // Host may advertise via env
        std::env::var("TCS_SIDEROLINK_ENDPOINT").unwrap_or_else(|_| {
            format!(
                "{}:{}",
                std::env::var("TCS_PUBLIC_HOST").unwrap_or_else(|_| "tcs.local".into()),
                self.cfg.listen_port
            )
        })
    }

    fn ensure_interface(&mut self) -> Result<(), AppError> {
        // ip link add tcs-sl0 type wireguard (ignore exists)
        let _ = run_cmd(&["ip", "link", "add", "dev", &self.iface, "type", "wireguard"]);
        // Assign the server's own overlay address (first usable in the /64) so
        // nodes can reach TCS at server_address over the tunnel.
        let server_addr = format!("{}/64", self.server_address());
        let _ = run_cmd(&["ip", "address", "add", &server_addr, "dev", &self.iface]);
        // Keep the legacy IPv4 CGNAT address too so pre-existing tooling/peers
        // that expect 100.64.0.1 keep a route; harmless alongside the IPv6.
        let _ = run_cmd(&["ip", "address", "add", "100.64.0.1/10", "dev", &self.iface]);
        // WireGuard data port (not the gRPC API port).
        run_cmd(&[
            "wg",
            "set",
            &self.iface,
            "listen-port",
            &self.cfg.listen_port.to_string(),
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
                &self.cfg.listen_port.to_string(),
                "private-key",
                tmp.to_str().unwrap_or(""),
            ]);
            let _ = fs::remove_file(&tmp);
            r
        })?;
        // WireGuard link MTU (nodes use 1280; the host interface can be higher).
        let _ = run_cmd(&["ip", "link", "set", "mtu", "1420", "dev", &self.iface]);
        // Bounce the link so the kernel re-creates the WireGuard UDP socket bound
        // to the final listen-port. When the device is (re)created via netlink the
        // kernel allocates its UDP socket up front; configuring listen-port/privkey
        // without a down/up can leave a stale socket that is bound but never
        // demultiplexes incoming datagrams to the WG device (handshakes arrive at
        // the host but the peer shows 0 rx / no handshake). A down->up rebind is
        // what `wg-quick` does implicitly and is the reliable fix.
        let _ = run_cmd(&["ip", "link", "set", "down", "dev", &self.iface]);
        run_cmd(&["ip", "link", "set", "up", "dev", &self.iface])?;
        Ok(())
    }

    /// Add or update a peer on the WG interface. `allowed_ip` is the node's
    /// overlay address (IPv6). IPv6 peers use /128, IPv4 /32.
    pub fn set_peer(&self, public_key: &str, allowed_ip: &str) -> Result<(), AppError> {
        if !self.enabled {
            return Ok(());
        }
        let bits = if allowed_ip.contains(':') { 128 } else { 32 };
        run_cmd(&[
            "wg",
            "set",
            &self.iface,
            "peer",
            public_key,
            "allowed-ips",
            &format!("{allowed_ip}/{bits}"),
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
                tracing::debug!(key_path = %path.display(), "Siderolink WG key loaded from file");
                return (priv_b64.to_string(), pub_b64);
            }
        }
        tracing::warn!(
            key_path = %path.display(),
            b64_len = priv_b64.len(),
            "Siderolink WG key file present but invalid; generating new"
        );
    } else {
        tracing::debug!(key_path = %path.display(), "Siderolink WG key file absent; generating new");
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
