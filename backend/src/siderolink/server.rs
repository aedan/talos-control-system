//! Tonic gRPC implementation of the upstream SideroLink API, so real Talos
//! nodes can dial into TCS and bring up a native WireGuard tunnel.
//!
//! The node calls `Provision` once (with its ephemeral WG public key + join
//! token). We validate the token, assign the node a random IPv6 in the ULA /64
//! derived from the installation id, add it as a WireGuard peer on `tcs-sl0`,
//! record it in the peer registry (keyed by system_uuid), and return the
//! server's WG endpoint/public key + the node's overlay address. Talos then
//! creates its own WG interface and the tunnel is live — no reboot.

use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::db::pool::DbPool;
use crate::network::SiderolinkWg;
use crate::config::SideroLinkConfig;

use super::address;
use super::pb::provision_service_server::{ProvisionService, ProvisionServiceServer};
use super::pb::wire_guard_over_grpc_service_server::{
    WireGuardOverGrpcService, WireGuardOverGrpcServiceServer,
};
use super::pb::{PeerPacket, ProvisionRequest, ProvisionResponse};

pub struct SiderolinkServer {
    pub cfg: SideroLinkConfig,
    pub wg: Arc<SiderolinkWg>,
    pub pool: DbPool,
    /// Stable identifier used to derive the ULA /64 prefix. Defaults to "tcs".
    pub installation_id: String,
    /// The public IP/hostname nodes dial for the WG data endpoint (listen_port).
    pub wg_endpoint_host: String,
    /// Revoke flag: when true, Provision rejects new joins (cluster disabled).
    pub enabled: Arc<RwLock<bool>>,
}

impl SiderolinkServer {
    pub fn server_public_key(&self) -> String {
        self.wg.server_public_key().to_string()
    }

    /// The WG data endpoint the node should dial: <host>:<listen_port>.
    pub fn wg_endpoint(&self) -> String {
        format!("{}:{}", self.wg_endpoint_host, self.cfg.listen_port)
    }

    /// Server address on the overlay (first usable in the /64).
    pub fn server_address(&self) -> String {
        address::server_address(&self.installation_id)
    }

    pub fn network_prefix(&self) -> String {
        let net = address::addr_from_prefix_bytes(
            address::network_prefix(&self.installation_id),
            [0; 8],
        );
        format!("{net}/64")
    }

    async fn register_peer(
        &self,
        node_uuid: &str,
        node_public_key: &str,
        node_addr: &str,
    ) -> Result<(), String> {
        use chrono::Utc;
        use crate::db::repos::siderolink::{upsert_peer, find_by_uuid, SiderolinkPeer};
        use uuid::Uuid;

        // Key the peer by the node's Talos UUID (== TCS system_uuid).
        let peer_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, node_uuid.as_bytes());
        let now = Utc::now();
        let existing = find_by_uuid(&self.pool, node_uuid).await.map_err(|e| e.to_string())?;
        let existing_id = existing.as_ref().map(|p| p.id);
        let existing_created = existing.as_ref().map(|p| p.created_at);
        let peer = SiderolinkPeer {
            id: existing_id.unwrap_or(peer_id),
            system_uuid: node_uuid.to_string(),
            public_key: node_public_key.to_string(),
            assigned_ip: node_addr.to_string(),
            last_seen: now,
            created_at: existing_created.unwrap_or(now),
        };
        upsert_peer(&self.pool, &peer).await.map_err(|e| e.to_string())?;

        // Add/update the WireGuard peer so the node's overlay address routes to it.
        if let Err(e) = self.wg.set_peer(node_public_key, node_addr) {
            warn!(error = %e, "failed to set WG peer for {node_uuid}");
        }

        // Mark the machine connected so effective_endpoint prefers the tunnel.
        if let Ok(true) = crate::db::repos::machine::set_siderolink_connected(&self.pool, node_uuid, true).await {
            info!(node = %node_uuid, overlay = %node_addr, "Siderolink peer connected");
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl ProvisionService for SiderolinkServer {
    async fn provision(
        &self,
        request: tonic::Request<ProvisionRequest>,
    ) -> Result<tonic::Response<ProvisionResponse>, tonic::Status> {
        let req = request.into_inner();

        // Cluster disabled -> refuse joins.
        if !*self.enabled.read().await {
            return Err(tonic::Status::permission_denied("Siderolink is disabled"));
        }

        // Validate the join token (ephemeral or per-cluster persistent).
        let token = req.join_token.as_deref().unwrap_or("");
        let ok = crate::db::repos::siderolink::validate_token(&self.pool, token)
            .await
            .map_err(|e| tonic::Status::internal(format!("token check failed: {e}")))?;
        if !ok {
            info!(node_uuid = %req.node_uuid, "Siderolink join with invalid/absent token rejected");
            return Err(tonic::Status::permission_denied("invalid join token"));
        }

        if req.node_uuid.is_empty() || req.node_public_key.is_empty() {
            return Err(tonic::Status::invalid_argument("node_uuid and node_public_key required"));
        }

        // Reuse the node's stable overlay address if it already provisioned (the
        // Talos SideroLink Manager re-dials Provision periodically — e.g. every
        // 30s to check peer health — and a changing address would prevent the
        // WireGuard tunnel from ever establishing).
        let existing = crate::db::repos::siderolink::find_by_uuid(&self.pool, &req.node_uuid)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;
        let node_addr = match existing {
            Some(peer) if !peer.assigned_ip.is_empty() => peer.assigned_ip,
            _ => {
                let (addr, _tail) = address::random_node_prefix(&self.installation_id);
                addr
            }
        };
        let node_prefix = format!("{node_addr}/64");

        if let Err(e) = self
            .register_peer(&req.node_uuid, &req.node_public_key, &node_addr)
            .await
        {
            return Err(tonic::Status::internal(e));
        }

        // Native WG mode: return the real WG data endpoint. (grpc_tunnel mode is
        // not implemented; the node requested native by default, and if it asks
        // for grpc_tunnel we still return a native endpoint so it works.)
        let endpoint = self.wg_endpoint();
        info!(
            node_uuid = %req.node_uuid,
            talos = ?req.talos_version,
            wireguard_over_grpc = ?req.wireguard_over_grpc,
            node_addr = %node_addr,
            endpoint = %endpoint,
            "Siderolink node provisioned"
        );

        Ok(tonic::Response::new(ProvisionResponse {
            server_endpoint: vec![endpoint],
            server_public_key: self.server_public_key(),
            node_address_prefix: node_prefix,
            server_address: self.server_address(),
            grpc_peer_addr_port: String::new(), // native mode (empty)
        }))
    }
}

/// WireGuard-over-gRPC stream (tunnel mode). We run in native WG mode and do not
/// support WG-over-gRPC; return an explicit error so a tunnel-mode node fails
/// fast instead of hanging. (Native nodes never call this.)
#[derive(Default)]
pub struct WgGrpcNotSupported;

#[tonic::async_trait]
impl WireGuardOverGrpcService for WgGrpcNotSupported {
    type CreateStreamStream =
        futures::stream::Empty<std::result::Result<PeerPacket, tonic::Status>>;

    async fn create_stream(
        &self,
        _request: tonic::Request<tonic::Streaming<PeerPacket>>,
    ) -> std::result::Result<
        tonic::Response<Self::CreateStreamStream>,
        tonic::Status,
    > {
        Err(tonic::Status::unimplemented(
            "WireGuard-over-gRPC is not supported; use native WireGuard mode",
        ))
    }
}

pub fn build_router(
    srv: Arc<SiderolinkServer>,
    wg_grpc: WgGrpcNotSupported,
) -> tonic::transport::server::Router {
    tonic::transport::Server::builder()
        .add_service(ProvisionServiceServer::from_arc(srv))
        .add_service(WireGuardOverGrpcServiceServer::new(wg_grpc))
}
