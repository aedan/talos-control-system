//! Read-only K8s explorer endpoints (list + detail + arbitrary-kind get).
//!
//! All routes are `/clusters/:id/k8s/...` and sit behind the RBAC middleware.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::integration::k8s_explorer as ex;
use crate::AppState;

use super::k8s_common;

#[derive(Deserialize)]
pub struct NsQuery {
    #[serde(default)]
    ns: Option<String>,
}

/// GET /clusters/:id/k8s/kinds — discoverable kinds for the resource tree.
pub async fn list_kinds(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ex::ResolvedKind>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    Ok(Json(client.all_kinds()))
}

/// GET /clusters/:id/k8s/namespaces
pub async fn list_namespaces(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ex::NamespaceSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let ns = client
        .list_namespaces()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(ns.iter().map(ex::namespace_summary).collect()))
}

/// GET /clusters/:id/k8s/pods?ns=
pub async fn list_pods(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<ex::PodSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let pods = client
        .list_pods(q.ns.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(pods.iter().map(ex::pod_summary).collect()))
}

/// GET /clusters/:id/k8s/pods/:ns/:name — full pod detail (containers, labels, yaml).
pub async fn get_pod(
    State(state): State<AppState>,
    Path((id, ns, name)): Path<(Uuid, String, String)>,
) -> Result<Json<ex::PodDetail>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let pod = client
        .get_pod(&ns, &name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(ex::pod_detail(&pod)))
}

/// GET /clusters/:id/k8s/deployments?ns=
pub async fn list_deployments(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<ex::DeploymentSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let deps = client
        .list_deployments(q.ns.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(deps.iter().map(ex::deployment_summary).collect()))
}

/// GET /clusters/:id/k8s/services?ns=
pub async fn list_services(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<ex::ServiceSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let svcs = client
        .list_services(q.ns.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(svcs.iter().map(ex::service_summary).collect()))
}

/// GET /clusters/:id/k8s/events?ns=
pub async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<NsQuery>,
) -> Result<Json<Vec<ex::EventSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let evs = client
        .list_events(q.ns.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(evs.iter().map(ex::event_summary).collect()))
}

/// GET /clusters/:id/k8s/nodes
pub async fn list_nodes(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ex::NodeSummary>>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let nodes = client
        .list_nodes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(nodes.iter().map(ex::node_summary).collect()))
}

#[derive(Deserialize)]
pub struct KindQuery {
    kind: String,
    #[serde(default)]
    ns: Option<String>,
}

/// GET /clusters/:id/k8s/resource?kind=&ns= — list an arbitrary kind.
pub async fn list_resource(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<KindQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let v = client
        .list_kind(&q.kind, q.ns.as_deref())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct KindNameQuery {
    kind: String,
    #[serde(default)]
    ns: Option<String>,
}

/// GET /clusters/:id/k8s/resource/:name?kind=&ns= — get one object of an arbitrary kind.
pub async fn get_resource(
    State(state): State<AppState>,
    Path((id, name)): Path<(Uuid, String)>,
    Query(q): Query<KindNameQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = k8s_common::client_for(&state, id).await?;
    let v = client
        .get_kind(&q.kind, q.ns.as_deref(), &name)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(v))
}
