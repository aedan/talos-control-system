//! Mutating K8s explorer endpoints: delete, scale, cordon/uncordon, drain, apply.
//!
//! These are powerful (arbitrary-kind mutation), so each handler:
//!   * requires the caller to be a **global admin** (RBAC middleware already
//!     authenticated + scoped them; this adds the admin gate), and
//!   * writes an **audit-log** entry.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;

use super::k8s_common;

/// Require the authenticated caller to be a global admin.
async fn require_admin(headers: &axum::http::HeaderMap) -> Result<String, (StatusCode, String)> {
    let claims = k8s_common::claims_from(headers, None)?;
    if !k8s_common::is_admin(&claims) {
        return Err((StatusCode::FORBIDDEN, "admin role required".to_string()));
    }
    Ok(claims.sub)
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    kind: String,
    #[serde(default)]
    ns: Option<String>,
    name: String,
}

/// DELETE /clusters/:id/k8s/resource/:name?kind=&ns=
pub async fn delete_resource(
    State(state): State<AppState>,
    Path((id, name)): Path<(Uuid, String)>,
    Query(q): Query<DeleteQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    client
        .delete_kind(&q.kind, q.ns.as_deref(), &name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(
        &state,
        &user,
        "k8s_delete",
        &id.to_string(),
        &format!("kind={} ns={:?} name={}", q.kind, q.ns, name),
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "deleted": name })))
}

#[derive(Deserialize)]
pub struct ScaleBody {
    #[serde(default)]
    ns: String,
    name: String,
    replicas: i32,
}

/// POST /clusters/:id/k8s/scale
pub async fn scale_deployment(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ScaleBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    client
        .scale_deployment(&body.ns, &body.name, body.replicas)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(
        &state,
        &user,
        "k8s_scale",
        &id.to_string(),
        &format!("ns={} name={} replicas={}", body.ns, body.name, body.replicas),
    )
    .await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "name": body.name,
        "replicas": body.replicas,
    })))
}

#[derive(Deserialize)]
pub struct NodeBody {
    name: String,
    #[serde(default)]
    force: bool,
}

/// POST /clusters/:id/k8s/cordon
pub async fn cordon_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<NodeBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    client
        .cordon(&body.name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(&state, &user, "k8s_cordon", &id.to_string(), &body.name).await;
    Ok(Json(serde_json::json!({ "ok": true, "node": body.name, "cordoned": true })))
}

/// POST /clusters/:id/k8s/uncordon
pub async fn uncordon_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<NodeBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    client
        .uncordon(&body.name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(&state, &user, "k8s_uncordon", &id.to_string(), &body.name).await;
    Ok(Json(serde_json::json!({ "ok": true, "node": body.name, "cordoned": false })))
}

/// POST /clusters/:id/k8s/drain
pub async fn drain_node(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<NodeBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    let result = client
        .drain(&body.name, body.force)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(
        &state,
        &user,
        "k8s_drain",
        &id.to_string(),
        &format!(
            "node={} force={} evicted={} skipped={} errors={}",
            body.name,
            body.force,
            result.evicted.len(),
            result.skipped.len(),
            result.errors.len()
        ),
    )
    .await;
    Ok(Json(serde_json::to_value(&result).unwrap_or_default()))
}

#[derive(Deserialize)]
pub struct ApplyBody {
    manifest: String,
}

/// POST /clusters/:id/k8s/apply
pub async fn apply_manifest(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(body): Json<ApplyBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let user = require_admin(&headers).await?;
    let client = k8s_common::client_for(&state, id).await?;
    let results = client
        .apply_manifest(&body.manifest)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    k8s_common::audit(
        &state,
        &user,
        "k8s_apply",
        &id.to_string(),
        &format!("docs={} applied={}", results.len(), results.iter().filter(|r| r.status == "applied").count()),
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "results": results })))
}
