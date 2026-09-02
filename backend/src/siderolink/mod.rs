//! SideroLink API server: lets real Talos nodes dial into TCS and bring up a
//! WireGuard tunnel. Implements the upstream `sidero.link.ProvisionService`
//! (gRPC) so Talos's built-in SideroLink client can join, plus the native
//! WireGuard endpoint (reusing the `tcs-sl0` interface managed by
//! `network::SiderolinkWg`).
//!
//! Protocol reference: `github.com/siderolabs/siderolink` (api/siderolink).

pub mod address;
#[allow(dead_code, missing_docs, non_camel_case_types, non_snake_case, non_upper_case_globals)]
pub mod pb;
pub mod server;

use crate::config::{SideroLinkConfig, ServerConfig};
use crate::db::pool::DbPool;
use uuid::Uuid;

/// Build the standalone `SideroLinkConfig` machine-config document for a
/// cluster (the form Talos v1.10+ reconciles live). Returns `""` when there is
/// no cluster, no token, or token creation fails — callers treat `""` as
/// "Siderolink not baked in."
pub async fn siderolink_doc_for_cluster(
    pool: &DbPool,
    cluster_id: Option<Uuid>,
    sl: &SideroLinkConfig,
    server: &ServerConfig,
) -> String {
    let Some(cid) = cluster_id else {
        return String::new();
    };
    let token = match crate::controllers::ClusterController::new(pool.clone())
        .ensure_cluster_siderolink_token(cid)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "could not ensure cluster siderolink token; omitting doc");
            return String::new();
        }
    };
    // Host the node dials for the SideroLink API: env override, else advertised
    // host, else bind_addr. Port is the SideroLink gRPC API port (bind_port).
    let endpoint = std::env::var("TCS_SIDEROLINK_ENDPOINT").ok().filter(|s| !s.is_empty());
    let endpoint = match endpoint {
        Some(e) => e,
        None => {
            let host = server
                .advertised_url
                .trim()
                .split("//")
                .nth(1)
                .and_then(|h| h.split('/').next())
                .and_then(|h| h.split(':').next())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| server.bind_addr.clone());
            format!("{host}:{}", sl.bind_port)
        }
    };
    // Talos SideroLinkConfig document. `grpc://` = plaintext (the WG data plane
    // is the encrypted part); the join token authenticates the node.
    format!(
        "---\napiVersion: v1alpha1\nkind: SideroLinkConfig\napiUrl: grpc://{endpoint}/?jointoken={token}\n"
    )
}
