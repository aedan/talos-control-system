//! Minimal DHCP server for metal provisioning (full DHCP on dedicated interface).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use dhcproto::v4::{
    Decodable, Decoder, DhcpOption, Encodable, Encoder, Message, MessageType, Opcode, OptionCode,
};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::config::MetalDhcpConfig;
use crate::db::pool::DbPool;
use crate::db::repos::{self, dhcp_lease::DhcpLease, machine::normalize_mac};
use crate::AppError;

/// Resolve IPv4 for a named interface (best-effort via `ip` CLI).
pub fn interface_ipv4(iface: &str) -> Option<Ipv4Addr> {
    let out = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", iface])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    // e.g. "2: eth1    inet 10.88.0.1/24 ..."
    for part in s.split_whitespace() {
        if part.contains('/') {
            let ip = part.split('/').next()?;
            if let Ok(v) = Ipv4Addr::from_str(ip) {
                if !v.is_loopback() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn parse_ip(s: &str) -> Result<Ipv4Addr, AppError> {
    Ipv4Addr::from_str(s.trim())
        .map_err(|e| AppError::Config(format!("invalid IP '{s}': {e}")))
}

fn ip_to_u32(ip: Ipv4Addr) -> u32 {
    u32::from(ip)
}

fn u32_to_ip(v: u32) -> Ipv4Addr {
    Ipv4Addr::from(v)
}

struct LeaseAllocator {
    range_start: u32,
    range_end: u32,
    /// ip -> mac
    used: HashMap<u32, String>,
}

impl LeaseAllocator {
    fn new(start: Ipv4Addr, end: Ipv4Addr) -> Self {
        Self {
            range_start: ip_to_u32(start),
            range_end: ip_to_u32(end),
            used: HashMap::new(),
        }
    }

    fn load(&mut self, leases: &[(String, Ipv4Addr)]) {
        for (mac, ip) in leases {
            self.used.insert(ip_to_u32(*ip), mac.clone());
        }
    }

    fn allocate(&mut self, mac: &str, preferred: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
        // keep existing assignment
        for (ip, m) in &self.used {
            if m == mac {
                return Some(u32_to_ip(*ip));
            }
        }
        if let Some(p) = preferred {
            let u = ip_to_u32(p);
            if u >= self.range_start && u <= self.range_end {
                if !self.used.contains_key(&u) || self.used.get(&u) == Some(&mac.to_string()) {
                    self.used.insert(u, mac.to_string());
                    return Some(p);
                }
            }
        }
        for u in self.range_start..=self.range_end {
            if !self.used.contains_key(&u) {
                self.used.insert(u, mac.to_string());
                return Some(u32_to_ip(u));
            }
        }
        None
    }
}

pub struct DhcpServerConfig {
    pub dhcp: MetalDhcpConfig,
    /// next-server / siaddr for PXE
    pub next_server: Ipv4Addr,
    /// HTTP base for iPXE script, e.g. http://10.88.0.1:6969
    pub http_boot_base: String,
    /// Filename for legacy PXE (iPXE script URL or undionly)
    pub boot_file: String,
    /// Serve iPXE binaries over TFTP for legacy PXE clients
    pub tftp_enabled: bool,
    /// iPXE binary filename (legacy BIOS)
    pub ipxe_bios_file: String,
    /// iPXE binary filename (UEFI)
    pub ipxe_uefi_file: String,
}

/// Spawn DHCP server when metal.dhcp.enabled.
pub fn spawn_dhcp_server(
    pool: DbPool,
    cfg: DhcpServerConfig,
) -> Option<tokio::task::JoinHandle<()>> {
    if !cfg.dhcp.enabled {
        return None;
    }
    if cfg.dhcp.interface.trim().is_empty() && cfg.dhcp.bind_ip.trim().is_empty() {
        warn!("metal.dhcp.enabled but interface and bind_ip empty — refusing to start DHCP");
        return None;
    }
    Some(tokio::spawn(async move {
        if let Err(e) = run_dhcp_loop(pool, cfg).await {
            warn!(error = %e, "DHCP server stopped");
        }
    }))
}

async fn run_dhcp_loop(pool: DbPool, cfg: DhcpServerConfig) -> Result<(), AppError> {
    let bind_ip = if !cfg.dhcp.bind_ip.is_empty() {
        parse_ip(&cfg.dhcp.bind_ip)?
    } else {
        interface_ipv4(&cfg.dhcp.interface).ok_or_else(|| {
            AppError::Config(format!(
                "Cannot resolve IPv4 for interface {}",
                cfg.dhcp.interface
            ))
        })?
    };

    let server_ip = bind_ip;
    let range_start = parse_ip(&cfg.dhcp.range_start)?;
    let range_end = parse_ip(&cfg.dhcp.range_end)?;
    let gateway = parse_ip(&cfg.dhcp.gateway)?;
    let subnet = parse_subnet_mask(&cfg.dhcp.subnet)?;
    let dns: Vec<Ipv4Addr> = cfg
        .dhcp
        .dns
        .iter()
        .filter_map(|d| Ipv4Addr::from_str(d).ok())
        .collect();
    let lease_secs = cfg.dhcp.lease_ttl_secs.max(60);

    // Bind 0.0.0.0:67 so we receive broadcasts; filter by interface via SO_BINDTODEVICE when set.
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| AppError::Network(format!("DHCP socket: {e}")))?;
    socket
        .set_reuse_address(true)
        .map_err(|e| AppError::Network(format!("DHCP reuse: {e}")))?;
    #[cfg(target_os = "linux")]
    if !cfg.dhcp.interface.is_empty() {
        if let Err(e) = socket.bind_device(Some(cfg.dhcp.interface.as_bytes())) {
            warn!(
                error = %e,
                iface = %cfg.dhcp.interface,
                "SO_BINDTODEVICE failed; DHCP may answer on all interfaces"
            );
        }
    }
    socket
        .set_broadcast(true)
        .map_err(|e| AppError::Network(format!("DHCP broadcast: {e}")))?;
    socket
        .bind(&SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 67)).into())
        .map_err(|e| AppError::Network(format!("DHCP bind :67: {e}")))?;
    socket
        .set_nonblocking(true)
        .map_err(|e| AppError::Network(format!("DHCP nonblocking: {e}")))?;
    let std_sock: std::net::UdpSocket = socket.into();
    let sock = UdpSocket::from_std(std_sock)
        .map_err(|e| AppError::Network(format!("DHCP tokio socket: {e}")))?;

    info!(
        bind = %server_ip,
        iface = %cfg.dhcp.interface,
        range = %format!("{}-{}", range_start, range_end),
        "DHCP server listening on UDP/67"
    );

    let allocator = Arc::new(Mutex::new(LeaseAllocator::new(range_start, range_end)));
    {
        let mut alloc = allocator.lock().await;
        if let Ok(leases) = repos::dhcp_lease::list_active(&pool, Utc::now()).await {
            let pairs: Vec<_> = leases
                .iter()
                .filter_map(|l| {
                    Ipv4Addr::from_str(&l.ip)
                        .ok()
                        .map(|ip| (l.mac.clone(), ip))
                })
                .collect();
            alloc.load(&pairs);
        }
    }

    let mut buf = vec![0u8; 2048];
    loop {
        // HA lock
        match crate::runtime::ha::try_acquire(&pool, "metal_dhcp", 30).await {
            Ok(true) => {}
            Ok(false) => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Err(e) => {
                warn!(error = %e, "DHCP HA lock error");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }

        let (n, src) = match tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf)).await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                warn!(error = %e, "DHCP recv error");
                continue;
            }
            Err(_) => continue, // timeout — recheck HA lock
        };

        let msg = match Message::decode(&mut Decoder::new(&buf[..n])) {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "DHCP decode failed");
                continue;
            }
        };

        let msg_type = match msg.opts().get(OptionCode::MessageType) {
            Some(DhcpOption::MessageType(t)) => *t,
            _ => continue,
        };

        let chaddr = msg.chaddr();
        if chaddr.len() < 6 {
            continue;
        }
        let mac = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            chaddr[0], chaddr[1], chaddr[2], chaddr[3], chaddr[4], chaddr[5]
        );
        let mac_n = normalize_mac(&mac);

        let machine = repos::machine::get_by_mac(&pool, &mac_n).await.ok().flatten();
        if machine.is_none() && !cfg.dhcp.allow_unknown {
            debug!(%mac_n, "DHCP ignore unknown MAC");
            continue;
        }

        let preferred = {
            let yi = msg.yiaddr();
            if yi != Ipv4Addr::UNSPECIFIED {
                Some(yi)
            } else if let Some(DhcpOption::RequestedIpAddress(ip)) =
                msg.opts().get(OptionCode::RequestedIpAddress)
            {
                Some(*ip)
            } else {
                None
            }
        };
        let mut alloc = allocator.lock().await;
        let yiaddr = match alloc.allocate(&mac_n, preferred) {
            Some(ip) => ip,
            None => {
                warn!(%mac_n, "DHCP pool exhausted");
                continue;
            }
        };
        drop(alloc);

        let hostname = machine
            .as_ref()
            .map(|m| {
                if m.hostname.is_empty() {
                    m.system_uuid.clone()
                } else {
                    m.hostname.clone()
                }
            })
            .unwrap_or_default();

        let reply_type = match msg_type {
            MessageType::Discover => MessageType::Offer,
            MessageType::Request => MessageType::Ack,
            MessageType::Inform => MessageType::Ack,
            _ => continue,
        };

        let mut reply = Message::default();
        reply.set_opcode(Opcode::BootReply);
        reply.set_xid(msg.xid());
        reply.set_flags(msg.flags());
        reply.set_chaddr(chaddr);
        reply.set_yiaddr(yiaddr);
        reply.set_siaddr(cfg.next_server);
        reply
            .opts_mut()
            .insert(DhcpOption::MessageType(reply_type));
        reply
            .opts_mut()
            .insert(DhcpOption::ServerIdentifier(server_ip));
        reply
            .opts_mut()
            .insert(DhcpOption::AddressLeaseTime(lease_secs));
        reply
            .opts_mut()
            .insert(DhcpOption::SubnetMask(subnet));
        reply
            .opts_mut()
            .insert(DhcpOption::Router(vec![gateway]));
        if !dns.is_empty() {
            reply.opts_mut().insert(DhcpOption::DomainNameServer(dns.clone()));
        }
        // bootfile — iPXE chain via HTTP when client is iPXE; legacy PXE
        // (BIOS/UEFI firmware) gets the iPXE binary name via TFTP.
        let script_url = format!(
            "{}/pxe/ipxe/{}",
            cfg.http_boot_base.trim_end_matches('/'),
            mac_n
        );
        // iPXE identifies itself via option 77 (User-Class) = "iPXE". Some
        // builds also echo it in option 60 (Vendor-Class); check both.
        let is_ipxe = matches!(
            msg.opts().get(OptionCode::UserClass),
            Some(DhcpOption::UserClass(v)) if String::from_utf8_lossy(v).contains("iPXE")
        ) || matches!(
            msg.opts().get(OptionCode::ClassIdentifier),
            Some(DhcpOption::ClassIdentifier(v)) if String::from_utf8_lossy(v).contains("iPXE")
        );
        let is_uefi = matches!(
            msg.opts().get(OptionCode::ClientSystemArchitecture),
            Some(DhcpOption::ClientSystemArchitecture(a)) if arch_is_uefi(*a)
        );
        let bootfile: String = if is_ipxe {
            script_url.clone()
        } else if cfg.tftp_enabled && !cfg.boot_file.is_empty() {
            cfg.boot_file.clone()
        } else if cfg.tftp_enabled {
            if is_uefi {
                cfg.ipxe_uefi_file.clone()
            } else {
                cfg.ipxe_bios_file.clone()
            }
        } else if !cfg.boot_file.is_empty() {
            cfg.boot_file.clone()
        } else {
            script_url.clone()
        };
        reply
            .opts_mut()
            .insert(DhcpOption::BootfileName(bootfile.clone().into_bytes()));

        // option 66/67 style also via siaddr + file field. Legacy PXE clients
        // use the sname/file fields for TFTP; iPXE clients consume option 67
        // (the URL above). sname always holds the TFTP server for compatibility.
        reply.set_sname_str(cfg.next_server.to_string());
        reply.set_fname_str(&bootfile);

        let mut out = Vec::new();
        if let Err(e) = reply.encode(&mut Encoder::new(&mut out)) {
            warn!(error = %e, "DHCP encode failed");
            continue;
        }

        // Broadcast reply if client flags broadcast or source is 0.0.0.0
        let dest = if src.ip().is_unspecified()
            || msg.flags().broadcast()
            || matches!(src.ip(), std::net::IpAddr::V4(ip) if ip.is_unspecified())
        {
            SocketAddr::from(SocketAddrV4::new(Ipv4Addr::BROADCAST, 68))
        } else {
            SocketAddr::new(src.ip(), 68)
        };

        if let Err(e) = sock.send_to(&out, dest).await {
            warn!(error = %e, "DHCP send failed");
            continue;
        }

        debug!(%mac_n, %yiaddr, ?reply_type, "DHCP reply sent");

        // Persist lease on OFFER/ACK
        let now = Utc::now();
        let lease = DhcpLease {
            mac: mac_n.clone(),
            ip: yiaddr.to_string(),
            hostname: hostname.clone(),
            machine_id: machine.as_ref().map(|m| m.id),
            expires_at: now + ChronoDuration::seconds(lease_secs as i64),
            created_at: now,
            updated_at: now,
        };
        if let Err(e) = repos::dhcp_lease::upsert(&pool, &lease).await {
            warn!(error = %e, "Failed to persist DHCP lease");
        }

        // Bind machine address from lease when empty
        if let Some(mut m) = machine {
            if m.address.is_empty() {
                m.address = yiaddr.to_string();
                m.updated_at = now;
                let _ = repos::machine::update(&pool, &m).await;
            }
        }
    }
}

fn parse_subnet_mask(cidr: &str) -> Result<Ipv4Addr, AppError> {
    if let Some((ip, prefix)) = cidr.split_once('/') {
        let _ = parse_ip(ip)?;
        let p: u32 = prefix
            .parse()
            .map_err(|_| AppError::Config(format!("bad prefix in {cidr}")))?;
        if p > 32 {
            return Err(AppError::Config(format!("bad prefix in {cidr}")));
        }
        let mask = if p == 0 {
            0u32
        } else {
            u32::MAX << (32 - p)
        };
        Ok(Ipv4Addr::from(mask))
    } else {
        // assume already a mask
        parse_ip(cidr)
    }
}

/// True when the DHCP client architecture indicates UEFI (any EFI flavor).
fn arch_is_uefi(arch: dhcproto::v4::Architecture) -> bool {
    !matches!(arch, dhcproto::v4::Architecture::Intelx86PC)
}
