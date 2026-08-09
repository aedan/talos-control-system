use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub peer_id: Uuid,
    pub public_key: Vec<u8>,
    pub assigned_ip: IpAddr,
    pub endpoint: String,
    pub last_seen: Instant,
    pub system_uuid: String,
}

impl PeerInfo {
    pub fn is_alive(&self, timeout: Duration) -> bool {
        self.last_seen.elapsed() < timeout
    }
}

pub struct SideroLinkManager {
    peers: Arc<RwLock<HashMap<Uuid, PeerInfo>>>,
    next_ip: Arc<RwLock<u32>>,
    subnet_start: u32,
    subnet_end: u32,
    listen_port: u16,
    bind_port: u16,
    mtu: u16,
    rate_limit: u64,
}

impl SideroLinkManager {
    pub fn new(config: &crate::config::SideroLinkConfig) -> Self {
        let (start, end) = Self::parse_subnet(&config.subnet);

        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            next_ip: Arc::new(RwLock::new(start)),
            subnet_start: start,
            subnet_end: end,
            listen_port: config.listen_port,
            bind_port: config.bind_port,
            mtu: config.mtu,
            rate_limit: config.rate_limit_bytes,
        }
    }

    fn parse_subnet(subnet: &str) -> (u32, u32) {
        let parts: Vec<&str> = subnet.split('/').collect();
        let ip_str = parts[0];
        let prefix_len: u32 = parts.get(1).unwrap_or(&"10").parse().unwrap_or(10);

        let ip_parts: Vec<u32> = ip_str.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        let start = u32::from_be_bytes([ip_parts[0] as u8, ip_parts[1] as u8, ip_parts[2] as u8, ip_parts[3] as u8]);

        let mask = if prefix_len == 0 { 0 } else { !0u32 << (32 - prefix_len) };
        let network = start & mask;
        let end = network | (!mask);

        (network + 2, end - 1)
    }

    pub async fn register_peer(&self, peer_id: Uuid, public_key: Vec<u8>, system_uuid: String) -> Result<PeerInfo, String> {
        let assigned_ip = self.assign_ip().await
            .ok_or_else(|| "IP pool exhausted".to_string())?;

        let peer = PeerInfo {
            peer_id,
            public_key,
            assigned_ip,
            endpoint: format!("{}:{}", assigned_ip, self.listen_port),
            last_seen: Instant::now(),
            system_uuid,
        };

        self.peers.write().await.insert(peer_id, peer.clone());
        info!(peer_id = %peer_id, ip = %assigned_ip, "Peer registered via SideroLink");

        Ok(peer)
    }

    pub async fn unregister_peer(&self, peer_id: Uuid) {
        if self.peers.write().await.remove(&peer_id).is_some() {
            info!(peer_id = %peer_id, "Peer unregistered from SideroLink");
        }
    }

    pub async fn get_peer(&self, peer_id: &Uuid) -> Option<PeerInfo> {
        self.peers.read().await.get(peer_id).cloned()
    }

    pub async fn update_peer_alive(&self, peer_id: &Uuid) {
        if let Some(peer) = self.peers.write().await.get_mut(peer_id) {
            peer.last_seen = Instant::now();
        }
    }

    pub async fn get_all_peers(&self) -> Vec<PeerInfo> {
        self.peers.read().await.values().cloned().collect()
    }

    pub async fn cleanup_dead_peers(&self, timeout: Duration) {
        let mut peers = self.peers.write().await;
        let dead_peers: Vec<Uuid> = peers.iter()
            .filter(|(_, peer)| !peer.is_alive(timeout))
            .map(|(id, _)| *id)
            .collect();

        for id in &dead_peers {
            peers.remove(id);
            warn!(peer_id = %id, "Dead peer cleaned up");
        }
    }

    async fn assign_ip(&self) -> Option<IpAddr> {
        let mut next = self.next_ip.write().await;
        if *next > self.subnet_end {
            warn!("IP pool exhausted");
            return None;
        }

        let ip = u32_to_ipv4(*next);
        *next += 1;
        Some(IpAddr::V4(ip))
    }

    pub fn config(&self) -> serde_json::Value {
        serde_json::json!({
            "listen_port": self.listen_port,
            "bind_port": self.bind_port,
            "mtu": self.mtu,
            "subnet": format!("{}-{}", ip_to_str(self.subnet_start), ip_to_str(self.subnet_end)),
            "rate_limit": self.rate_limit,
        })
    }
}

fn u32_to_ipv4(n: u32) -> Ipv4Addr {
    Ipv4Addr::from(n.to_be_bytes())
}

fn ip_to_str(n: u32) -> String {
    let bytes = n.to_be_bytes();
    format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
}
