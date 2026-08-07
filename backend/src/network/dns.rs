use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::network::siderolink::SideroLinkManager;

pub struct DnsEntry {
    pub name: String,
    pub ip: String,
    pub ttl: u32,
    pub last_resolved: Instant,
}

pub struct DnsResolver {
    cache: Arc<RwLock<HashMap<String, DnsEntry>>>,
    siderolink: Arc<SideroLinkManager>,
}

impl DnsResolver {
    pub fn new(siderolink: Arc<SideroLinkManager>) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            siderolink,
        }
    }

    pub async fn resolve(&self, name: &str) -> Option<String> {
        let cache = self.cache.read().await;

        if let Some(entry) = cache.get(name) {
            if entry.last_resolved.elapsed() < Duration::from_secs(entry.ttl as u64) {
                debug!(name, ip = %entry.ip, "DNS cache hit");
                return Some(entry.ip.clone());
            }
        }
        drop(cache);

        if let Some(ip) = self.resolve_via_siderolink(name).await {
            let entry = DnsEntry {
                name: name.to_string(),
                ip: ip.clone(),
                ttl: 60,
                last_resolved: Instant::now(),
            };

            self.cache.write().await.insert(name.to_string(), entry);
            debug!(name, ip = %ip, "DNS resolved via SideroLink");
            return Some(ip);
        }

        debug!(name, "DNS resolution failed");
        None
    }

    async fn resolve_via_siderolink(&self, name: &str) -> Option<String> {
        let parts: Vec<&str> = name.split('.').collect();

        if parts.len() >= 2 {
            let machine_name = parts[0];
            let peers = self.siderolink.get_all_peers().await;

            for peer in peers {
                if peer.system_uuid.contains(machine_name) || peer.peer_id.to_string().contains(machine_name) {
                    info!(name, peer_id = %peer.peer_id, ip = %peer.assigned_ip, "SideroLink DNS resolution");
                    return Some(peer.assigned_ip.to_string());
                }
            }
        }

        None
    }

    pub async fn invalidate(&self, name: &str) {
        self.cache.write().await.remove(name);
    }

    pub async fn preload(&self, names: Vec<String>) {
        for name in names {
            let _ = self.resolve(&name).await;
        }
    }
}

impl Clone for DnsResolver {
    fn clone(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            siderolink: Arc::clone(&self.siderolink),
        }
    }
}
