//! Siderolink WireGuard control-plane side.
//!
//! Manages a host WG interface (`tcs-sl0` by default) via `wg` / `ip` CLI when available.
//! Registration returns peer config so Talos nodes (or a helper) can complete the tunnel.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use base64::Engine;
use tracing::{debug, info, warn};
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
    /// Cache of (peer public key -> allowed IP) currently known, maintained by
    /// `set_peer`/`reapply_peers`. `prime_socket` recreates the WG device (which
    /// wipes the kernel peer list) and re-applies from this cache, so a prime
    /// never loses the configured peers. Shared via `Arc` so all clones agree.
    peer_cache: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
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
            peer_cache: std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
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
        let arc = Arc::new(mgr.clone());
        if arc.enabled() {
            // Start the persistent, self-healing socket watchdog (idempotent). It
            // primes the freshly-created device repeatedly until it has aged into
            // a functional socket (no time cap — a from-scratch device can take
            // well over 5 minutes), then stays quiet while healthy and re-primes
            // if the socket ever degrades again (e.g. after a toggle re-provisions
            // the nodes). Runs on a detached thread so it never blocks boot.
            arc.re_prime_in_background();
        }
        arc
    }

    /// Prime the socket by FULLY RECREATING the WG device (`ip link del` +
    /// `ip link add`), not just a link down/up. A down/up leaves the kernel's UDP
    /// socket object in place — and once it has gone into the bound-but-not-
    /// receiving (stale) state, a down/up does NOT reset it, so node handshake-
    /// inits keep arriving on the wire but the kernel drops them (no reply,
    /// frozen transfer counters, stale handshake ages). Deleting and re-adding the
    /// device creates a brand-new receiving socket; validated live on kronos — a
    /// recreation brought all 15 peers to fresh handshakes and 0% ping6 loss within
    /// ~20s, where repeated down/up bounces for 10+ minutes did not.
    ///
    /// Recreation wipes the kernel peer list, so the known peers are re-applied
    /// from the cache afterwards. Returns true once a FRESH handshake is observed.
    pub fn prime_socket(&self) -> bool {
        if !self.enabled {
            return false;
        }
        // Tear down and recreate the device. `del` can fail if it's already gone;
        // `add` still yields a fresh device either way.
        let _ = run_cmd(&["ip", "link", "del", "dev", &self.iface]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let add_ok = run_cmd(&["ip", "link", "add", &self.iface, "type", "wireguard"]);
        if add_ok.is_err() {
            // `add` failed (device may still exist) — bring the existing one up.
            let _ = run_cmd(&["ip", "link", "set", "up", "dev", &self.iface]);
        }
        let _ = run_cmd(&["ip", "link", "set", "up", "dev", &self.iface]);
        std::thread::sleep(std::time::Duration::from_millis(800));
        let ok = run_cmd(&[
            "wg",
            "set",
            &self.iface,
            "listen-port",
            &self.cfg.listen_port.to_string(),
            "private-key",
            "/dev/stdin",
        ])
        .or_else(|_| {
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
        });
        // Recreation clears the overlay addresses; restore them.
        let server_addr = format!("{}/64", self.server_address());
        let _ = run_cmd(&["ip", "address", "add", &server_addr, "dev", &self.iface]);
        let _ = run_cmd(&["ip", "address", "add", "100.64.0.1/10", "dev", &self.iface]);
        // Recreation wipes the peer list; restore it from the cache.
        let n_peers = self.reapply_cached_peers();
        match ok {
            Ok(()) => debug!(iface = %self.iface, peers = n_peers, "Siderolink WireGuard device recreated (socket primed)"),
            Err(e) => warn!(error = %e, iface = %self.iface, "Siderolink device recreation: key set failed"),
        }
        // Give the nodes time to re-handshake against the fresh socket, then
        // check for a FRESH handshake (a stale pre-recreation one does not count).
        std::thread::sleep(std::time::Duration::from_secs(12));
        self.has_fresh_handshake()
    }

    /// Re-apply every peer in the cache to the (re)created device. Returns the
    /// number successfully applied. `set_peer` re-records each into the cache.
    fn reapply_cached_peers(&self) -> usize {
        let snapshot = match self.peer_cache.read() {
            Ok(c) => c.clone(),
            Err(_) => return 0,
        };
        let mut ok = 0;
        for (pk, ip) in &snapshot {
            if self.set_peer(pk, ip).is_ok() {
                ok += 1;
            } else {
                warn!(peer = %pk, "Siderolink: failed to re-apply peer after recreation");
            }
        }
        ok
    }

    /// Start (once) a persistent, self-healing socket watchdog on a detached
    /// thread. A freshly netlink-created WG device's kernel UDP socket is not
    /// reliably receiving until the device has aged an INDETERMINATE amount — on
    /// kronos a from-scratch device sometimes only became functional after well
    /// over 5 minutes, so a time-capped prime loop (v0.5.40) could give up before
    /// the socket was ready and leave the tunnels stale. The watchdog instead runs
    /// for the whole process lifetime: every ~20s it checks whether any peer has
    /// a *fresh* handshake (age < 45s); if there are known peers but none is
    /// fresh, it primes the socket once (bounce + key + addrs). This heals the
    /// boot case (primes repeatedly until the device ages and a handshake lands —
    /// no cap), the Enable/Disable-toggle case (nodes re-provision with new keys,
    /// handshakes go stale, watchdog re-primes them back), and any spontaneous
    /// socket degradation. When the tunnel is healthy it is quiet (fresh
    /// handshakes present → no bounce).
    ///
    /// Idempotent: a process-lifetime `Once` guard ensures only one watchdog runs,
    /// so it's safe to call from both boot and the toggle handlers.
    ///
    /// Crucially the watchdog uses EXONENTIAL BACKOFF between primes and STOPS
    /// priming once a fresh handshake is seen. A freshly-created `tcs-sl0` device's
    /// kernel UDP socket does not start receiving until the device has AGED, and a
    /// bounce (down/up) resets that aging — so bouncing on a fixed short interval
    /// (e.g. every 20s) keeps the device perpetually "fresh" and it NEVER becomes
    /// functional (the nodes keep sending handshake-inits, visible on the wire, but
    /// the kernel socket drops them). Backing off lets the device settle; the next
    /// handshake-init a node sends is then processed and the tunnel comes up. Once
    /// healthy it goes quiet, and only resumes (with backoff again) if the tunnel
    /// later degrades.
    pub fn re_prime_in_background(&self) {
        use std::sync::Once;
        static STARTED: Once = Once::new();
        if !self.enabled {
            return;
        }
        STARTED.call_once(|| {
            let mgr = self.clone();
            std::thread::spawn(move || {
                // Let the freshly-created device AGE before the first bounce. A
                // from-scratch `tcs-sl0` does not start receiving until it has been
                // up for well over a minute, and an early bounce resets that aging —
                // so wait before disturbing it at all.
                std::thread::sleep(std::time::Duration::from_secs(45));
                // Prime intervals (seconds) between consecutive no-fresh-handshake
                // primes. Growing gaps let the device settle between bounces so one
                // of them lands on an aged, functional socket. Reset to the first
                // gap the moment a fresh handshake is observed (stop disturbing it).
                let intervals: &[u64] = &[45, 90, 180, 300, 480];
                let mut idx = 0usize;
                loop {
                    // Healthy (or no peers expected) → go quiet and check occasionally;
                    // resume priming only if it degrades.
                    let healthy = !mgr.known_peers() || mgr.has_fresh_handshake();
                    if healthy {
                        if idx != 0 {
                            info!(
                                iface = %mgr.iface,
                                "Siderolink socket watchdog: fresh handshake present — tunnel healthy, standing down"
                            );
                        }
                        idx = 0;
                        std::thread::sleep(std::time::Duration::from_secs(90));
                        continue;
                    }
                    // No fresh handshake with peers present → prime, then back off.
                    let wait = intervals[idx];
                    info!(
                        next_prime_in_secs = wait,
                        iface = %mgr.iface,
                        "Siderolink socket watchdog: no fresh handshake with peers present — priming"
                    );
                    let _ = mgr.prime_socket();
                    idx = (idx + 1).min(intervals.len() - 1);
                    std::thread::sleep(std::time::Duration::from_secs(wait));
                }
            });
        });
    }

    /// True if the WG device currently has at least one kernel peer configured
    /// (a `peer:` line in `wg show`). Used by the watchdog to avoid needlessly
    /// bouncing the socket when no node is expected (e.g. SideroLink disabled).
    fn known_peers(&self) -> bool {
        let out = match run_cmd_stdout(&["wg", "show", &self.iface]) {
            Ok(o) => o,
            Err(_) => return false,
        };
        out.lines().any(|l| l.trim_start().starts_with("peer:"))
    }

    /// True if any peer has a *fresh* handshake — one completed within the last
    /// `max_age_secs`. Parses the `wg show` "latest handshake: N seconds/minutes
    /// ago" ages. A fresh handshake proves the socket is actively receiving from
    /// a re-provisioned node, which is the real "tunnel is up" signal after a
    /// toggle (stale pre-provision handshakes don't count).
    fn has_fresh_handshake(&self) -> bool {
        let out = match run_cmd_stdout(&["wg", "show", &self.iface]) {
            Ok(o) => o,
            Err(_) => return false,
        };
        const MAX_AGE_SECS: u64 = 20;
        for line in out.lines() {
            let Some(rest) = line.trim().strip_prefix("latest handshake:") else {
                continue;
            };
            if let Some(secs) = parse_handshake_age_secs(rest.trim()) {
                if secs <= MAX_AGE_SECS {
                    return true;
                }
            }
        }
        false
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
        // Remember the peer so a device recreation (prime) can restore it.
        if let Ok(mut c) = self.peer_cache.write() {
            c.insert(public_key.to_string(), allowed_ip.to_string());
        }
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

fn run_cmd_stdout(args: &[&str]) -> Result<String, AppError> {
    let (bin, rest) = args
        .split_first()
        .ok_or_else(|| AppError::Internal("empty command".into()))?;
    let out = Command::new(bin)
        .args(rest)
        .output()
        .map_err(|e| AppError::Internal(format!("spawn {}: {}", bin, e)))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        return Err(AppError::Internal(format!(
            "{} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(stdout)
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

/// Parse a `wg show` "latest handshake:" age string into total seconds.
/// Handles the human formats WireGuard emits: "just now", "N second(s) ago",
/// "N minute(s) ago", and "N minute(s), M second(s) ago". Returns None for
/// anything unrecognized (the caller treats it as "not fresh").
fn parse_handshake_age_secs(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches("ago").trim();
    if s == "just now" {
        return Some(0);
    }
    let mut total = 0u64;
    let mut found = false;
    for part in s.split(',') {
        let part = part.trim();
        let (num, unit) = part.split_once(' ')?;
        let n: u64 = num.parse().ok()?;
        match unit {
            "second" | "seconds" => total += n,
            "minute" | "minutes" => total += n * 60,
            // "hour(s)" or unknown units: treat the whole thing as unrecognized.
            _ => return None,
        }
        found = true;
    }
    if found { Some(total) } else { None }
}

#[cfg(test)]
mod handshake_age_tests {
    use super::parse_handshake_age_secs;

    #[test]
    fn parses_plain_seconds() {
        assert_eq!(parse_handshake_age_secs("47 seconds ago"), Some(47));
        assert_eq!(parse_handshake_age_secs("1 second ago"), Some(1));
    }

    #[test]
    fn parses_minutes() {
        assert_eq!(parse_handshake_age_secs("2 minutes ago"), Some(120));
        assert_eq!(parse_handshake_age_secs("1 minute ago"), Some(60));
    }

    #[test]
    fn parses_minutes_and_seconds() {
        assert_eq!(
            parse_handshake_age_secs("1 minute, 47 seconds ago"),
            Some(107)
        );
        assert_eq!(
            parse_handshake_age_secs("5 minutes, 4 seconds ago"),
            Some(304)
        );
    }

    #[test]
    fn just_now_and_unknown() {
        assert_eq!(parse_handshake_age_secs("just now"), Some(0));
        assert_eq!(parse_handshake_age_secs("2 hours ago"), None);
        assert_eq!(parse_handshake_age_secs(""), None);
        assert_eq!(parse_handshake_age_secs("nonsense"), None);
    }
}
