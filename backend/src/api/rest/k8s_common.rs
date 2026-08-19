//! Shared helpers for the `/clusters/:id/k8s/*` explorer + CLI endpoints.
//!
//! These live behind the RBAC middleware (which authenticates the JWT and
//! enforces role + per-cluster scope). Handlers here only need to:
//!   1. load the stored (encrypted) cluster row,
//!   2. pull a live [`K8sClient`] from the shared pool,
//!   3. perform the K8s operation.
//!
//! Mutation handlers additionally verify the caller's role directly (admin)
//! and write an audit-log entry.

use axum::http::HeaderMap;
use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::jwt::{verify_jwt, Claims};
use crate::db::repos;
use crate::integration::K8sClient;
use crate::AppState;

/// Fetch the stored cluster row (with its encrypted kubeconfig) or 404.
pub async fn load_cluster(
    state: &AppState,
    cluster_id: Uuid,
) -> Result<crate::db::models::Cluster, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(c)) => Ok(c),
        Ok(None) => Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// Pull a live, discovery-ready [`K8sClient`] for the stored cluster.
///
/// Returns 400 if the cluster has no kubeconfig attached.
pub async fn client_for(
    state: &AppState,
    cluster_id: Uuid,
) -> Result<std::sync::Arc<K8sClient>, (StatusCode, String)> {
    let cluster = load_cluster(state, cluster_id).await?;
    let Some(enc) = cluster.kubeconfig.clone() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cluster has no kubeconfig attached".to_string(),
        ));
    };
    state
        .k8s_pool
        .get(cluster_id, &enc, &state.config.auth.jwt_secret)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))
}

/// Extract the authenticated caller's claims from headers or `?token=`.
///
/// The RBAC middleware already validated the token; this is only used to
/// attribute audit-log entries.
pub fn claims_from(headers: &HeaderMap, query: Option<&str>) -> Result<Claims, (StatusCode, String)> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| {
            let q = query?;
            q.split('&')
                .find_map(|p| p.strip_prefix("token="))
                .map(|s| s.to_string())
        })
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing token".to_string()))?;

    verify_jwt(&token)
        .map(|t| t.claims)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".to_string()))
}

/// True when the caller is a global admin.
pub fn is_admin(claims: &Claims) -> bool {
    claims.role == "admin"
}

/// Record an audit-log entry for a mutation. Failures are logged, not fatal.
pub async fn audit(
    state: &AppState,
    user: &str,
    action: &str,
    target: &str,
    detail: &str,
) {
    crate::utils::audit::log_action(&state.db_pool, user, action, target, detail).await;
}
