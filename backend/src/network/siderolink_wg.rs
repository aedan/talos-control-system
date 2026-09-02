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
        // Create the device only if it is absent. A FRESHLY netlink-created
        // WireGuard device (`ip link add type wireguard`) ends up with a kernel
        // UDP socket that is bound but does NOT receive: incoming handshake-inits
        // arrive at the host (tcpdump) yet the peer shows 0 rx and no handshake,
        // and UDP RcvbufErrors climb. Reusing an already-existing device (a plain
        // down->up) works reliably. So we never delete+recreate here — if a
        // device survives a TCS restart we bounce it; only when truly absent do
        // we add it (and the trailing down->up below re-attaches its socket).
        let exists = run_cmd(&["ip", "link", "show", "dev", &self.iface]).is_ok();
        if !exists {
            let _ = run_cmd(&["ip", "link", "add", "dev", &self.iface, "type", "wireguard"]);
        }
        // Bounce the link to re-attach the kernel WireGuard UDP socket. A
        // freshly netlink-created WG device (or a leftover one from a prior boot)
        // can be left with a socket that is bound to the listen port but never
        // demultiplexes incoming datagrams to the device — handshake-inits arrive
        // at the host (tcpdump) yet the peer shows 0 rx / no handshake and UDP
        // RcvbufErrors climb. The proven-working order (validated live on kronos)
        // is: down -> up, THEN `wg set private-key` on the now-live device.
        // Setting the key BEFORE the bounce leaves the socket stale. This step
        // MUST also come before the address assignment: bringing a WG link down
        // clears its addresses, so adding them afterwards guarantees they survive.
        run_cmd(&["ip", "link", "set", "up", "dev", &self.iface])?;
        let _ = run_cmd(&["ip", "link", "set", "down", "dev", &self.iface]);
        run_cmd(&["ip", "link", "set", "up", "dev", &self.iface])?;
        // Settle delay before setting the key. A freshly (re)created/`up`-ed WG
        // device needs a moment for the kernel to finish attaching its UDP socket
        // before `wg set private-key` binds to it; setting the key immediately
        // after the up leaves the socket in a state that is bound but does not
        // receive (validated live on kronos: a 1.5s settle makes the boot
        // sequence work where the no-delay sequence leaves the peer at 0 rx).
        std::thread::sleep(std::time::Duration::from_millis(1500));
        // WireGuard data port (not the gRPC API port) + identity key, applied on
        // the live (post-bounce) device so the kernel re-binds its UDP socket to
        // the listen port and starts receiving.
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
        // Assign the server's own overlay address (first usable in the /64) so
        // nodes can reach TCS at server_address over the tunnel.
        let server_addr = format!("{}/64", self.server_address());
        let _ = run_cmd(&["ip", "address", "add", &server_addr, "dev", &self.iface]);
        // Keep the legacy IPv4 CGNAT address too so pre-existing tooling/peers
        // that expect 100.64.0.1 keep a route; harmless alongside the IPv6.
        let _ = run_cmd(&["ip", "address", "add", "100.64.0.1/10", "dev", &self.iface]);
        // WireGuard link MTU (nodes use 1280; the host interface can be higher).
        let _ = run_cmd(&["ip", "link", "set", "mtu", "1420", "dev", &self.iface]);
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

    /// Re-register known peers to the kernel WG interface. Peers are persisted
    /// in the DB but live only in the kernel's per-device state, which is wiped
    /// when `tcs-sl0` is (re)created — i.e. on every TCS restart. Without this,
    /// nodes that do NOT re-provision after a TCS restart (Talos keeps its own
    /// cached provisionData and only retries the existing WG handshake) find no
    /// matching peer on the fresh device and the tunnel stays down until they
    /// happen to re-dial Provision. Re-applying all DB peers at boot closes that
    /// gap. Returns the number of peers successfully applied.
    pub fn reapply_peers(&self, peers: &[(String, String)]) -> usize {
        if !self.enabled {
            return 0;
        }
        let mut ok = 0;
        for (public_key, allowed_ip) in peers {
            if self.set_peer(public_key, allowed_ip).is_ok() {
                ok += 1;
            } else {
                warn!(
                    peer = %public_key,
                    "Siderolink: failed to re-apply peer at boot"
                );
            }
        }
        if ok > 0 {
            info!(count = ok, "Siderolink: re-applied known WireGuard peers at boot");
        }
        ok
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
