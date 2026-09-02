use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::jwt::{create_claims, create_jwt, Claims, verify_jwt};
use crate::auth::local::{authenticate_local, change_password as local_change_password};
use crate::db::models::auth::User;
use crate::db::models::branding::TenantBranding;
use crate::db::repos::{self, user};
use crate::config::tls::TlsConfig;
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub commit: String,
    pub build_time: String,
}

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: crate::utils::version::VERSION_INFO.version.clone(),
        commit: crate::utils::version::VERSION_INFO.commit.clone(),
        build_time: crate::utils::version::VERSION_INFO.build_time.clone(),
    })
}

/// Public: which auth entrypoints the login UI should show.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProvidersResponse {
    pub local: bool,
    pub oidc: bool,
    pub saml: bool,
    pub ldap_configured: bool,
}

pub async fn get_auth_providers(State(state): State<AppState>) -> Json<AuthProvidersResponse> {
    Json(AuthProvidersResponse {
        local: true,
        oidc: state
            .config
            .auth
            .oidc
            .as_ref()
            .map(|o| o.enabled)
            .unwrap_or(false),
        saml: state
            .config
            .auth
            .saml
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(false),
        ldap_configured: state.config.auth.ldap.is_some(),
    })
}

/// HTML bootstrap after browser SSO (OIDC redirect or SAML ACS POST).
fn sso_token_html(token: &str) -> axum::response::Html<String> {
    let escaped = token.replace('\\', "\\\\").replace('\'', "\\'");
    axum::response::Html(format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><title>Signing in…</title></head>
<body><p>Signing in…</p>
<script>
try {{
  localStorage.setItem('tcs_token', '{escaped}');
  window.location.replace('/');
}} catch (e) {{
  document.body.innerText = 'Login succeeded but browser storage failed: ' + e;
}}
</script></body></html>"#
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandingResponse {
    pub name: String,
    pub short_name: String,
    pub tagline: String,
    pub primary_color: String,
    pub secondary_color: String,
    pub background_color: String,
    pub surface_color: String,
    pub text_color: String,
    pub text_muted_color: String,
    pub font_family: String,
    pub docs_url: String,
    pub support_url: String,
}

/// Resolve tenant id: X-Tenant-ID → subdomain → default.
pub fn resolve_tenant_id(headers: &HeaderMap) -> String {
    if let Some(h) = headers.get("x-tenant-id").and_then(|v| v.to_str().ok()) {
        let t = h.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Some(host) = headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) {
        let host = host.split(':').next().unwrap_or(host);
        let parts: Vec<&str> = host.split('.').collect();
        if parts.len() >= 3 {
            let sub = parts[0];
            if sub != "www" && sub != "api" && sub != "localhost" {
                return sub.to_string();
            }
        }
    }
    "default".to_string()
}

#[cfg(test)]
mod tenant_tests {
    use super::resolve_tenant_id;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn tenant_from_header() {
        let mut h = HeaderMap::new();
        h.insert("x-tenant-id", HeaderValue::from_static("acme"));
        assert_eq!(resolve_tenant_id(&h), "acme");
    }

    #[test]
    fn tenant_from_subdomain() {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::HOST, HeaderValue::from_static("acme.tcs.example.com"));
        assert_eq!(resolve_tenant_id(&h), "acme");
    }

    #[test]
    fn tenant_default() {
        let h = HeaderMap::new();
        assert_eq!(resolve_tenant_id(&h), "default");
    }
}

fn branding_response(branding: crate::config::BrandingConfig) -> BrandingResponse {
    BrandingResponse {
        name: branding.name,
        short_name: branding.short_name,
        tagline: branding.tagline,
        primary_color: branding.primary_color,
        secondary_color: branding.secondary_color,
        background_color: branding.background_color,
        surface_color: branding.surface_color,
        text_color: branding.text_color,
        text_muted_color: branding.text_muted_color,
        font_family: branding.font_family,
        docs_url: branding.docs_url,
        support_url: branding.support_url,
    }
}

pub async fn get_branding(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<BrandingResponse> {
    let tenant = resolve_tenant_id(&headers);
    let branding = state.branding.get_branding(&tenant).await;
    Json(branding_response(branding))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBrandingRequest {
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub primary_color: Option<String>,
    pub secondary_color: Option<String>,
    pub background_color: Option<String>,
    pub surface_color: Option<String>,
    pub text_color: Option<String>,
    pub text_muted_color: Option<String>,
    pub font_family: Option<String>,
    pub docs_url: Option<String>,
    pub support_url: Option<String>,
}

pub async fn update_branding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateBrandingRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let tenant = resolve_tenant_id(&headers);
    let branding = TenantBranding {
        tenant_id: tenant,
        name: payload.name,
        short_name: payload.short_name,
        primary_color: payload.primary_color,
        secondary_color: payload.secondary_color,
        background_color: payload.background_color,
        surface_color: payload.surface_color,
        text_color: payload.text_color,
        text_muted_color: payload.text_muted_color,
        font_family: payload.font_family,
        docs_url: payload.docs_url,
        support_url: payload.support_url,
        ..Default::default()
    };

    if let Err(e) = repos::branding::upsert_tenant_branding(&state.db_pool, &branding).await {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    state.branding.reload().await.ok();
    Ok(StatusCode::OK)
}

pub async fn get_branding_css(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, String) {
    let tenant = resolve_tenant_id(&headers);
    let branding = state.branding.get_branding(&tenant).await;
    let css = crate::branding::theme::generate_css_variables(&branding);

    (StatusCode::OK, css)
}

pub async fn get_tenant_branding(
    State(state): State<AppState>,
    Path(tenant_id): Path<String>,
) -> Json<BrandingResponse> {
    let branding = state.branding.get_branding(&tenant_id).await;
    Json(branding_response(branding))
}

pub async fn put_tenant_branding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tenant_id): Path<String>,
    Json(payload): Json<UpdateBrandingRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }
    let branding = TenantBranding {
        tenant_id,
        name: payload.name,
        short_name: payload.short_name,
        primary_color: payload.primary_color,
        secondary_color: payload.secondary_color,
        background_color: payload.background_color,
        surface_color: payload.surface_color,
        text_color: payload.text_color,
        text_muted_color: payload.text_muted_color,
        font_family: payload.font_family,
        docs_url: payload.docs_url,
        support_url: payload.support_url,
        ..Default::default()
    };
    repos::branding::upsert_tenant_branding(&state.db_pool, &branding)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state.branding.reload().await.ok();
    Ok(StatusCode::OK)
}

pub async fn get_logo(
    State(state): State<AppState>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let branding = state.branding.get_branding("default").await;
    let svg = crate::branding::generator::generate_logo_svg(&branding);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/svg+xml".parse().unwrap());

    (StatusCode::OK, headers, svg)
}

pub async fn get_favicon(
    State(state): State<AppState>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let branding = state.branding.get_branding("default").await;
    let png = crate::branding::generator::generate_favicon_png(&branding);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/png".parse().unwrap());

    (StatusCode::OK, headers, png)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateClusterRequest {
    pub name: String,
    pub control_plane_version: String,
    pub talos_version: String,
    /// Optional Image Factory system extensions (modules) to bake into this
    /// cluster's machines, e.g. ["siderolabs/bnx2-bnx2x"].
    #[serde(default)]
    pub factory_modules: Option<Vec<String>>,
}

pub async fn create_cluster(
    State(state): State<AppState>,
    Json(payload): Json<CreateClusterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    // Inventory-only: does not provision Talos or Kubernetes.
    let mut cluster = crate::db::models::cluster::Cluster::new(
        payload.name,
        payload.control_plane_version,
        payload.talos_version,
    );

    if let Some(mods) = payload.factory_modules {
        let list: Vec<String> = mods
            .into_iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
        if !list.is_empty() {
            cluster.factory_modules = Some(serde_json::to_string(&list).unwrap_or_default());
        }
    }

    match repos::cluster::create(&state.db_pool, &cluster).await {
        Ok(c) => Ok((StatusCode::CREATED, Json(cluster_public_json(c)))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn list_clusters(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let clusters = repos::cluster::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if claims.role == "admin" {
        return Ok(Json(
            clusters.into_iter().map(cluster_public_json).collect(),
        ));
    }

    let user = repos::user::get_by_email(&state.db_pool, &claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    let n = repos::cluster_access::count_for_user(&state.db_pool, user.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if n == 0 {
        // Legacy: no membership rows → all clusters at global role.
        return Ok(Json(
            clusters.into_iter().map(cluster_public_json).collect(),
        ));
    }

    let memberships = repos::cluster_access::list_for_user(&state.db_pool, user.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let allowed: std::collections::HashSet<_> =
        memberships.into_iter().map(|m| m.cluster_id).collect();

    Ok(Json(
        clusters
            .into_iter()
            .filter(|c| allowed.contains(&c.id))
            .map(cluster_public_json)
            .collect(),
    ))
}

pub async fn get_cluster(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, id).await {
        Ok(Some(cluster)) => Ok(Json(cluster_public_json(cluster))),
        Ok(None) => Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn update_cluster(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, id).await {
        Ok(Some(mut cluster)) => {
            if let Some(name) = payload.get("name").and_then(|v| v.as_str()) {
                cluster.name = name.to_string();
            }
            match repos::cluster::update(&state.db_pool, &cluster).await {
                Ok(c) => Ok(Json(cluster_public_json(c))),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn delete_cluster(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _ = repos::cluster_access::delete_for_cluster(&state.db_pool, id).await;
    match repos::cluster::delete(&state.db_pool, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

pub async fn list_machines(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let machines = repos::machine::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let machines = if claims.role == "admin" {
        machines
    } else {
        let user = repos::user::get_by_email(&state.db_pool, &claims.sub)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found".to_string()))?;
        let n = repos::cluster_access::count_for_user(&state.db_pool, user.id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if n == 0 {
            machines
        } else {
            let memberships = repos::cluster_access::list_for_user(&state.db_pool, user.id)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let allowed: std::collections::HashSet<_> =
                memberships.into_iter().map(|m| m.cluster_id).collect();
            machines
                .into_iter()
                .filter(|m| m.cluster_id.map(|c| allowed.contains(&c)).unwrap_or(false))
                .collect()
        }
    };

    let vals: Result<Vec<_>, _> = machines.iter().map(machine_to_json).collect();
    match vals {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ─── Cluster access (per-cluster RBAC) ─────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertClusterAccessRequest {
    pub user_id: Uuid,
    pub role: String,
}

pub async fn list_cluster_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }
    let rows = repos::cluster_access::list_for_cluster(&state.db_pool, cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut out = Vec::new();
    for row in rows {
        let email = repos::user::get_by_id(&state.db_pool, row.user_id)
            .await
            .ok()
            .flatten()
            .map(|u| u.email)
            .unwrap_or_default();
        out.push(serde_json::json!({
            "userId": row.user_id,
            "clusterId": row.cluster_id,
            "role": row.role,
            "email": email,
            "createdAt": row.created_at,
        }));
    }
    Ok(Json(out))
}

pub async fn upsert_cluster_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
    Json(payload): Json<UpsertClusterAccessRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }
    if repos::cluster::get(&state.db_pool, cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string()));
    }
    if repos::user::get_by_id(&state.db_pool, payload.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }
    let row = repos::cluster_access::upsert(
        &state.db_pool,
        payload.user_id,
        cluster_id,
        &payload.role,
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "cluster_access_upsert",
        &cluster_id.to_string(),
        &format!("user={} role={}", payload.user_id, row.role),
    )
    .await;
    Ok(Json(serde_json::json!({
        "userId": row.user_id,
        "clusterId": row.cluster_id,
        "role": row.role,
        "createdAt": row.created_at,
    })))
}

pub async fn delete_cluster_access(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((cluster_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }
    repos::cluster_access::delete(&state.db_pool, user_id, cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "cluster_access_delete",
        &cluster_id.to_string(),
        &format!("user={}", user_id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn machine_to_json(machine: &crate::db::models::machine::Machine) -> Result<serde_json::Value, serde_json::Error> {
    let mut v = serde_json::to_value(machine)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "hasBmc".to_string(),
            serde_json::Value::Bool(machine.has_bmc()),
        );
    }
    Ok(v)
}

/// Same as `machine_to_json` but also resolves the machine's **effective
/// management endpoint** (Siderolink tunnel IP when connected + fresh, else the
/// LAN address) and its Siderolink-assigned IP, so the UI can show *how* TCS is
/// reaching the node. Async because it looks the peer up in the DB.
async fn machine_to_json_with_endpoint(
    pool: &crate::db::pool::DbPool,
    machine: &crate::db::models::machine::Machine,
) -> Result<serde_json::Value, serde_json::Error> {
    let mut v = machine_to_json(machine)?;
    if let Some(obj) = v.as_object_mut() {
        let endpoint = crate::controllers::cluster::effective_endpoint(pool, machine)
            .await
            .unwrap_or_else(|_| machine.address.clone());
        obj.insert(
            "effectiveEndpoint".to_string(),
            serde_json::Value::String(endpoint.clone()),
        );
        let via_tunnel = !machine.address.is_empty() && endpoint != machine.address;
        obj.insert(
            "viaSiderolink".to_string(),
            serde_json::Value::Bool(via_tunnel),
        );
        if let Ok(Some(peer)) =
            crate::db::repos::siderolink::find_by_uuid(pool, &machine.system_uuid).await
        {
            obj.insert(
                "siderolinkIp".to_string(),
                serde_json::Value::String(peer.assigned_ip),
            );
        }
    }
    Ok(v)
}

pub async fn get_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::machine::get(&state.db_pool, id).await {
        Ok(Some(machine)) => match machine_to_json_with_endpoint(&state.db_pool, &machine).await {
            Ok(v) => Ok(Json(v)),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Ok(None) => Err((StatusCode::NOT_FOUND, "Machine not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn delete_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    match repos::machine::delete(&state.db_pool, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

pub async fn get_metrics() -> String {
    let metrics = prometheus::gather();
    let mut buf = String::new();
    prometheus::TextEncoder::new()
        .encode_utf8(&metrics, &mut buf)
        .ok();
    buf
}

#[derive(Deserialize)]
pub struct ImportClusterRequest {
    pub name: String,
    pub kubeconfig: String,
    /// Optional talosconfig YAML (mTLS client credentials). Required for backups / apply / reboot.
    #[serde(default)]
    pub talosconfig: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportClusterResponse {
    pub cluster: serde_json::Value,
    pub machines_imported: i32,
}

fn cluster_public_json(mut cluster: crate::db::models::cluster::Cluster) -> serde_json::Value {
    let has_creds = cluster.has_talos_credentials();
    let has_kube = cluster.has_kubeconfig();
    cluster.talosconfig = None;
    cluster.kubeconfig = None;
    let mut v = serde_json::to_value(&cluster).unwrap_or_default();
    if let Some(obj) = v.as_object_mut() {
        obj.insert("hasTalosconfig".to_string(), serde_json::Value::Bool(has_creds));
        obj.insert("hasKubeconfig".to_string(), serde_json::Value::Bool(has_kube));
        // Expose factory_modules as a parsed array (or null) for the UI.
        let mods: Vec<String> = cluster
            .factory_modules
            .clone()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        obj.insert(
            "factoryModules".to_string(),
            serde_json::Value::Array(mods.into_iter().map(serde_json::Value::String).collect()),
        );
    }
    v
}

fn controller_for(state: &AppState) -> crate::controllers::cluster::ClusterController {
    crate::controllers::cluster::ClusterController::with_context(
        state.db_pool.clone(),
        state.config.database.sqlite_path.clone(),
        state.config.auth.jwt_secret.clone(),
    )
}

pub async fn import_cluster(
    State(state): State<AppState>,
    Json(payload): Json<ImportClusterRequest>,
) -> Result<(StatusCode, Json<ImportClusterResponse>), (StatusCode, String)> {
    let controller = controller_for(&state);

    match controller
        .import_cluster(payload.name, payload.kubeconfig, payload.talosconfig)
        .await
    {
        Ok(cluster) => {
            let machines = crate::db::repos::machine::list_by_cluster(&state.db_pool, cluster.id)
                .await
                .unwrap_or_default();

            Ok((
                StatusCode::CREATED,
                Json(ImportClusterResponse {
                    cluster: cluster_public_json(cluster),
                    machines_imported: machines.len() as i32,
                }),
            ))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn preview_import(
    State(state): State<AppState>,
    Json(payload): Json<ImportClusterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);

    match controller.preview_import(payload.kubeconfig).await {
        Ok(discovered) => match serde_json::to_value(discovered) {
            Ok(v) => Ok(Json(v)),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
pub struct SetTalosconfigRequest {
    pub talosconfig: String,
}

pub async fn set_cluster_talosconfig(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    Json(payload): Json<SetTalosconfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .set_talosconfig(cluster_id, payload.talosconfig)
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({
            "ok": true,
            "hasTalosconfig": true,
        }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// GET /clusters/:id/kubeconfig — return the decrypted kubeconfig YAML.
///
/// Used by `tcs kubeconfig` and the zero-touch tool wrappers on the TCS host.
/// The caller is authenticated + RBAC-checked by the middleware.
pub async fn get_cluster_kubeconfig(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let cluster = super::k8s_common::load_cluster(&state, cluster_id).await?;
    let Some(enc) = cluster.kubeconfig else {
        return Err((StatusCode::BAD_REQUEST, "Cluster has no kubeconfig attached".to_string()));
    };
    let plain = crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, &enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/yaml")
        .body(axum::body::Body::from(plain))
        .unwrap())
}

/// GET /clusters/:id/talosconfig — return the decrypted talosconfig YAML.
pub async fn get_cluster_talosconfig(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let cluster = super::k8s_common::load_cluster(&state, cluster_id).await?;
    let Some(enc) = cluster.talosconfig else {
        return Err((StatusCode::BAD_REQUEST, "Cluster has no talosconfig attached".to_string()));
    };
    let plain = crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, &enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/yaml")
        .body(axum::body::Body::from(plain))
        .unwrap())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetKubeconfigRequest {
    pub kubeconfig: String,
}

pub async fn set_cluster_kubeconfig(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    Json(payload): Json<SetKubeconfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .set_kubeconfig(cluster_id, payload.kubeconfig)
        .await
    {
        Ok(()) => Ok(Json(serde_json::json!({
            "ok": true,
            "hasKubeconfig": true,
        }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyConfigRequest {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupScheduleRequest {
    /// Hours between automatic snapshots; null or 0 disables.
    pub schedule_hours: Option<i32>,
    pub retention: Option<i32>,
}

pub async fn set_backup_schedule(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    Json(payload): Json<BackupScheduleRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .set_backup_schedule(cluster_id, payload.schedule_hours, payload.retention)
        .await
    {
        Ok(()) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "backup_schedule",
                &cluster_id.to_string(),
                &format!(
                    "hours={:?} retention={:?}",
                    payload.schedule_hours, payload.retention
                ),
            )
            .await;
            // Return updated public cluster view
            match repos::cluster::get(&state.db_pool, cluster_id).await {
                Ok(Some(c)) => Ok(Json(cluster_public_json(c))),
                Ok(None) => Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn apply_cluster_config(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    payload: Option<Json<ApplyConfigRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let dry_run = payload.map(|p| p.0.dry_run).unwrap_or(false);
    let controller = controller_for(&state);
    match controller.apply_config_patches(cluster_id, dry_run).await {
        Ok(result) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                if dry_run { "config_apply_dry_run" } else { "config_apply" },
                &cluster_id.to_string(),
                &result.to_string(),
            ).await;
            Ok(Json(result))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn refresh_cluster(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.refresh_from_kubeconfig(cluster_id).await {
        Ok(n) => Ok(Json(serde_json::json!({ "ok": true, "machines": n }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn test_cluster_talos(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.test_talos_connectivity(cluster_id).await {
        Ok(results) => Ok(Json(serde_json::json!({ "results": results }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn probe_cluster_versions(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.probe_cluster_talos_versions(cluster_id).await {
        Ok(v) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "talos_version_probe",
                &cluster_id.to_string(),
                &v.to_string(),
            )
            .await;
            Ok(Json(v))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn reboot_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.reboot_machine(id).await {
        Ok(()) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "reboot",
                &id.to_string(),
                "Machine reboot requested",
            ).await;
            Ok(Json(serde_json::json!({ "ok": true, "action": "reboot" })))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
pub struct UpgradeMachineRequest {
    pub image: String,
}

pub async fn upgrade_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<UpgradeMachineRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.upgrade_machine(id, &payload.image).await {
        Ok(()) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "upgrade",
                &id.to_string(),
                &format!("image={}", payload.image),
            ).await;
            Ok(Json(serde_json::json!({ "ok": true, "action": "upgrade", "image": payload.image })))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetMachineRequest {
    #[serde(default)]
    pub confirm: bool,
    #[serde(default = "default_true_reset")]
    pub graceful: bool,
    #[serde(default = "default_true_reset")]
    pub reboot: bool,
}

fn default_true_reset() -> bool {
    true
}

/// Destructive machine reset/wipe via Talos Reset RPC.
pub async fn reset_machine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<ResetMachineRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    if !payload.confirm {
        return Err((
            StatusCode::BAD_REQUEST,
            "confirm must be true for machine reset".into(),
        ));
    }
    let controller = controller_for(&state);
    controller
        .reset_machine(id, payload.graceful, payload.reboot)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "machine_reset",
        &id.to_string(),
        &format!("graceful={} reboot={}", payload.graceful, payload.reboot),
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "action": "reset" })))
}

pub async fn bootstrap_machine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let controller = controller_for(&state);
    controller
        .bootstrap_machine(id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "bootstrap",
        &id.to_string(),
        "",
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "action": "bootstrap" })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaleWorkersRequest {
    pub desired_workers: i32,
}

pub async fn scale_cluster_workers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<ScaleWorkersRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let controller = controller_for(&state);
    let cluster = controller
        .scale_workers(id, payload.desired_workers)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "scale_workers",
        &id.to_string(),
        &format!("desired={}", payload.desired_workers),
    )
    .await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "clusterId": cluster.id,
        "workerSize": cluster.worker_size,
        "note": "Inventory desired size updated. Apply worker configs out-of-band then import/register machines.",
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyProvisionConfigRequest {
    pub machine_id: Uuid,
    pub config_yaml: String,
}

/// Apply a generated provision config to a machine (greenfield assist).
pub async fn apply_provision_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ApplyProvisionConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    if payload.config_yaml.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "configYaml required".into()));
    }
    let controller = controller_for(&state);
    controller
        .apply_machine_config(payload.machine_id, &payload.config_yaml)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "apply_provision_config",
        &payload.machine_id.to_string(),
        "",
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn get_machine_version(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.machine_version(id).await {
        Ok(version) => Ok(Json(serde_json::json!({ "talosVersion": version }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn get_machine_services(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.machine_services(id).await {
        Ok(services) => Ok(Json(serde_json::json!({ "services": services }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// GET /machines/:id/versions — the node's installed/upgradable Talos versions.
pub async fn get_machine_versions(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.machine_versions(id).await {
        Ok(versions) => Ok(Json(versions)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// GET /machines/:id/extensions — the node's installed Talos extensions (modules).
pub async fn get_machine_extensions(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.machine_extensions(id).await {
        Ok(extensions) => Ok(Json(serde_json::json!({ "extensions": extensions }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn get_machine_hostname(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.machine_hostname(id).await {
        Ok(hostname) => Ok(Json(serde_json::json!({ "hostname": hostname }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// GET /factory/versions — Talos versions the Image Factory can build.
pub async fn list_factory_versions(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = crate::integration::image_factory::ImageFactoryClient::new(
        &state.config.factory.normalized_base(),
    );
    match client.list_versions().await {
        Ok(versions) => Ok(Json(serde_json::json!({ "versions": versions }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// GET /factory/extensions?version=v1.13.7 — official modules for a version.
pub async fn list_factory_extensions(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let client = crate::integration::image_factory::ImageFactoryClient::new(
        &state.config.factory.normalized_base(),
    );
    let version = q
        .get("version")
        .cloned()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "query param 'version' required".to_string()))?;
    match client.list_extensions(&version).await {
        Ok(extensions) => Ok(Json(serde_json::json!({ "extensions": extensions, "version": version }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetModulesRequest {
    pub modules: Option<Vec<String>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetModuleOverridesRequest {
    /// Modules to ADD on top of the cluster default set.
    #[serde(default)]
    pub adds: Option<Vec<String>>,
    /// Modules to REMOVE from the cluster default set.
    #[serde(default)]
    pub removes: Option<Vec<String>>,
    /// When true, also clears the machine's absolute `factory_modules`
    /// override so it returns fully to the cluster delta model.
    #[serde(default)]
    pub reset: bool,
}

/// GET /machines/:id/modules — the machine's effective factory modules
/// (delta model: cluster defaults ± node overrides, or absolute override).
pub async fn get_machine_modules(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.effective_modules(id).await {
        Ok(modules) => Ok(Json(serde_json::json!({ "modules": modules }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// PUT /machines/:id/modules — set/clear the machine's absolute module
/// override (legacy picker: "Apply these exact modules to this node").
pub async fn set_machine_modules(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<SetModulesRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.set_machine_factory_modules(id, payload.modules).await {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// PUT /machines/:id/module-overrides — set the node-level delta against the
/// cluster default module set (adds/removes), optionally resetting the
/// absolute override. Effective set = cluster − removes + adds.
pub async fn set_machine_module_overrides(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<SetModuleOverridesRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .set_machine_module_overrides(id, payload.adds, payload.removes, payload.reset)
        .await
    {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// POST /machines/:id/apply-modules — upgrade the machine to the factory image
/// that bundles its effective modules (reboots the node).
pub async fn apply_machine_modules(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .apply_machine_modules(id, &state.config.factory)
        .await
    {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// PUT /clusters/:id/modules — set the cluster's default factory modules.
pub async fn set_cluster_modules(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<SetModulesRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .set_cluster_factory_modules(id, payload.modules, &state.config.factory)
        .await
    {
        Ok(v) => Ok(Json(v)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

/// List disks on a machine via Talos StorageService.
pub async fn list_machine_disks(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.list_disks(id, None).await {
        Ok(disks) => Ok(Json(serde_json::json!({ "disks": disks }))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInstallDiskRequest {
    pub install_disk: String,
}

/// Set the install disk for a machine.
pub async fn set_install_disk(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<SetInstallDiskRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.set_install_disk(id, &payload.install_disk).await {
        Ok(m) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "set_install_disk",
                &id.to_string(),
                &payload.install_disk,
            ).await;
            Ok(Json(serde_json::json!({
                "ok": true,
                "installDisk": m.install_disk,
            })))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallMachineRequest {
    pub config_yaml: String,
}

/// Apply config with reboot to install Talos on a machine.
pub async fn install_machine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<InstallMachineRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    if payload.config_yaml.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "configYaml required".into()));
    }
    let controller = controller_for(&state);
    controller
        .install_machine(id, &payload.config_yaml, None)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "install_machine",
        &id.to_string(),
        "",
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMachineRequest {
    pub address: Option<String>,
    pub machine_type: Option<String>,
    pub cluster_id: Option<Uuid>,
    /// When true, detach machine from any cluster (overrides cluster_id).
    pub clear_cluster: Option<bool>,
    pub mac_address: Option<String>,
    pub hostname: Option<String>,
    pub install_disk: Option<String>,
    pub bmc_type: Option<String>,
    pub bmc_address: Option<String>,
    pub bmc_username: Option<String>,
    pub bmc_password: Option<String>,
}

pub async fn update_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<UpdateMachineRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    let mut changed = false;
    if let Some(addr) = payload.address {
        m.address = addr.trim().to_string();
        changed = true;
    }
    if let Some(t) = payload.machine_type {
        m.machine_type = t;
        changed = true;
    }
    if payload.clear_cluster == Some(true) {
        m.cluster_id = None;
        changed = true;
    } else if let Some(c) = payload.cluster_id {
        m.cluster_id = Some(c);
        changed = true;
    }
    if let Some(mac) = payload.mac_address {
        m.mac_address = repos::machine::normalize_mac(&mac);
        changed = true;
    }
    if let Some(h) = payload.hostname {
        m.hostname = h;
        changed = true;
    }
    if let Some(d) = payload.install_disk {
        m.install_disk = d;
        changed = true;
    }
    if let Some(bt) = payload.bmc_type {
        m.bmc_type = bt;
        changed = true;
    }
    if let Some(ba) = payload.bmc_address {
        m.bmc_address = ba;
        changed = true;
    }
    if let Some(bu) = payload.bmc_username {
        m.bmc_username = bu;
        changed = true;
    }
    if let Some(bp) = payload.bmc_password {
        m.bmc_password_enc = Some(bp);
        changed = true;
    }
    if !changed {
        return Err((StatusCode::BAD_REQUEST, "No fields to update".into()));
    }
    m.updated_at = chrono::Utc::now();
    let m = repos::machine::update(&state.db_pool, &m)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    machine_to_json(&m)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMachineRequest {
    pub system_uuid: Option<String>,
    pub machine_type: Option<String>,
    pub cluster_id: Option<Uuid>,
    pub address: Option<String>,
    pub mac_address: Option<String>,
    pub hostname: Option<String>,
    pub bmc_address: Option<String>,
    pub bmc_username: Option<String>,
    pub bmc_password: Option<String>,
    pub bmc_type: Option<String>,
    pub install_disk: Option<String>,
}

pub async fn create_machine(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateMachineRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let system_uuid = payload
        .system_uuid
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("baremetal-{}", Uuid::new_v4()));
    let machine_type = payload
        .machine_type
        .unwrap_or_else(|| "worker".into());
    let mut m = crate::db::models::machine::Machine::new(system_uuid, machine_type);
    m.cluster_id = payload.cluster_id;
    m.address = payload.address.unwrap_or_default();
    if let Some(mac) = payload.mac_address {
        m.mac_address = repos::machine::normalize_mac(&mac);
    }
    m.hostname = payload.hostname.unwrap_or_default();
    m.bmc_address = payload.bmc_address.unwrap_or_default();
    m.bmc_username = payload.bmc_username.unwrap_or_default();
    m.bmc_type = payload.bmc_type.unwrap_or_else(|| "auto".into());
    m.install_disk = payload.install_disk.unwrap_or_default();
    if let Some(pw) = payload.bmc_password.filter(|p| !p.is_empty()) {
        m.bmc_password_enc = Some(
            crate::utils::secrets::encrypt(&state.config.auth.jwt_secret, &pw)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        );
    }
    let m = repos::machine::create(&state.db_pool, &m)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "create_machine",
        &m.id.to_string(),
        &m.system_uuid,
    )
    .await;
    let v = serde_json::to_value(&m).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(v)))
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let existing = repos::user::get_by_email(&state.db_pool, &payload.email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let authenticated_user = if let Some(user) = existing {
        if !user.is_active {
            return Err((StatusCode::UNAUTHORIZED, "Account is disabled".to_string()));
        }
        match user.auth_provider.as_str() {
            "local" => authenticate_local(&state.db_pool, &payload.email, &payload.password)
                .await
                .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?,
            "ldap" => {
                if let Some(ref ldap_config) = state.config.auth.ldap {
                    let ldap_client = crate::auth::LdapClient::new(ldap_config.clone());
                    ldap_client
                        .authenticate(&state.db_pool, &payload.email, &payload.password)
                        .await
                        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?
                } else {
                    return Err((StatusCode::UNAUTHORIZED, "LDAP not configured".to_string()));
                }
            }
            "oidc" => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    "OIDC users must sign in via the OIDC button".to_string(),
                ));
            }
            other => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    format!("Auth provider '{}' not supported for password login", other),
                ));
            }
        }
    } else if let Some(ref ldap_config) = state.config.auth.ldap {
        // First-time / auto-provision LDAP login when no local row exists yet.
        let ldap_client = crate::auth::LdapClient::new(ldap_config.clone());
        ldap_client
            .authenticate(&state.db_pool, &payload.email, &payload.password)
            .await
            .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?
    } else {
        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".to_string()));
    };

    let token = create_jwt(
        &create_claims(
            &authenticated_user.email,
            &authenticated_user.role,
            std::time::Duration::from_secs(3600),
        ),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        user: authenticated_user,
    }))
}

pub async fn logout(
    headers: HeaderMap,
) -> StatusCode {
    if let Some(auth_header) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                if let Ok(token_data) = verify_jwt(token) {
                    tracing::info!(email = %token_data.claims.sub, "User logged out");
                }
            }
        }
    }
    StatusCode::OK
}

pub async fn refresh_token(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    // Issue new token with same claims
    let new_token = create_jwt(&create_claims(
        &claims.sub,
        &claims.role,
        std::time::Duration::from_secs(3600),
    )).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = repos::user::get_by_email(&state.db_pool, &claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    Ok(Json(LoginResponse {
        token: new_token,
        user,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    let user = repos::user::get_by_email(&state.db_pool, &claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    if user.auth_provider != "local" {
        return Err((StatusCode::BAD_REQUEST, "Password change only supported for local users".to_string()));
    }

    if !crate::auth::local::verify_password(&payload.current_password,
        user.password_hash.as_deref().ok_or_else(|| (StatusCode::BAD_REQUEST, "No password set".to_string()))?,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))? {
        return Err((StatusCode::UNAUTHORIZED, "Current password is incorrect".to_string()));
    }

    local_change_password(&state.db_pool, user.id, &payload.new_password)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInfoResponse {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
    pub auth_provider: String,
    pub password_needs_change: bool,
}

pub async fn get_user_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserInfoResponse>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    let user = repos::user::get_by_email(&state.db_pool, &claims.sub)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "User not found".to_string()))?;

    Ok(Json(UserInfoResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        role: user.role,
        is_active: user.is_active,
        last_login: user.last_login,
        auth_provider: user.auth_provider,
        password_needs_change: user.password_needs_change,
    }))
}

#[derive(Serialize)]
pub struct UserListResponse {
    pub users: Vec<UserInfoResponse>,
}

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<UserListResponse>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    let users = repos::user::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(UserListResponse {
        users: users.into_iter().map(|u| UserInfoResponse {
            id: u.id,
            email: u.email,
            display_name: u.display_name,
            role: u.role,
            is_active: u.is_active,
            last_login: u.last_login,
            auth_provider: u.auth_provider,
            password_needs_change: u.password_needs_change,
        }).collect(),
    }))
}

fn extract_claims(headers: &HeaderMap) -> Result<Claims, (StatusCode, String)> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Missing Authorization header".to_string()))?
        .to_str()
        .map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid Authorization header".to_string()))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid Authorization header format".to_string()))?;

    let token_data = verify_jwt(token)
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    Ok(token_data.claims)
}

pub async fn oidc_authorize(
    State(state): State<AppState>,
) -> Result<(StatusCode, axum::response::Redirect), (StatusCode, String)> {
    let oidc_config = state.config.auth.oidc
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "OIDC is not configured".to_string()))?;

    if !oidc_config.enabled {
        return Err((StatusCode::BAD_GATEWAY, "OIDC is disabled".to_string()));
    }

    let state_param = Uuid::new_v4().to_string();
    // Prefer DB-backed state for multi-replica HA; fall back to in-memory.
    if crate::db::repos::oidc_state::remember(&state.db_pool, &state_param, 600)
        .await
        .is_err()
    {
        crate::auth::TcsOidcProvider::remember_state(&state_param);
    }

    let provider = crate::auth::TcsOidcProvider::new(oidc_config.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let auth_url = provider.authorize_url(&state_param)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::FOUND, axum::response::Redirect::to(&auth_url)))
}

#[derive(Deserialize)]
pub struct OidcCallbackParams {
    pub code: String,
    pub state: String,
}

pub async fn oidc_callback(
    State(state): State<AppState>,
    Query(params): Query<OidcCallbackParams>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    let oidc_config = state.config.auth.oidc
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "OIDC is not configured".to_string()))?;

    if !oidc_config.enabled {
        return Err((StatusCode::BAD_GATEWAY, "OIDC is disabled".to_string()));
    }

    let db_ok = crate::db::repos::oidc_state::take(&state.db_pool, &params.state)
        .await
        .unwrap_or(false);
    if !db_ok && !crate::auth::TcsOidcProvider::take_state(&params.state) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid or expired OIDC state (CSRF check failed)".to_string(),
        ));
    }

    let provider = crate::auth::TcsOidcProvider::new(oidc_config.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_info = provider.exchange_code(&params.code, &oidc_config.redirect_url)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let token = provider.authenticate_and_issue_jwt(&state.db_pool, user_info)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(sso_token_html(&token))
}

// ─── Certificate Settings ─────────────────────────────────────────────

#[derive(Serialize)]
pub struct CertStatusResponse {
    pub mode: String,
    pub domains: Vec<String>,
    pub issuer: String,
    pub expires_at: Option<String>,
    pub days_remaining: i64,
    pub error: Option<String>,
}

pub async fn get_cert_status(
    State(state): State<AppState>,
) -> Result<Json<CertStatusResponse>, (StatusCode, String)> {
    // Prefer live runtime TLS (updated by Apply without restart); fall back to boot config.
    let tls_owned;
    let tls = if let Some(rt) = &state.tls_runtime {
        tls_owned = rt.tls.read().await.clone();
        &tls_owned
    } else {
        &state.config.tls
    };

    let mode = match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => "letsencrypt".to_string(),
        crate::config::TlsMode::SelfSigned => "self-signed".to_string(),
        crate::config::TlsMode::Provided => "provided".to_string(),
        crate::config::TlsMode::Disabled => "disabled".to_string(),
    };

    let (domains, issuer) = match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => {
            let le = tls.letsencrypt.as_ref();
            (
                le.map(|c| c.domains.clone()).unwrap_or_default(),
                "Let's Encrypt".to_string(),
            )
        }
        crate::config::TlsMode::SelfSigned => (
            tls.self_signed
                .as_ref()
                .map(|c| c.domains.clone())
                .unwrap_or_else(|| vec!["localhost".to_string()]),
            "Self-Signed".to_string(),
        ),
        crate::config::TlsMode::Provided => (vec![], "Custom".to_string()),
        crate::config::TlsMode::Disabled => (vec![], "None".to_string()),
    };

    let (days_remaining, expires_at) = if let Some(tls_runtime) = &state.tls_runtime {
        let certs = tls_runtime.certs.read().await;
        if let Some(exp) = crate::cert::provided::parse_expiry_from_cert_pem(&certs.0) {
            let diff = exp - chrono::Utc::now();
            (diff.num_days(), Some(exp.to_rfc3339()))
        } else {
            (-1, None)
        }
    } else {
        let cert_path = "/var/lib/tcs/certs/cert.pem";
        if let Ok(pem) = std::fs::read_to_string(cert_path) {
            if let Some(exp) = crate::cert::provided::parse_expiry_from_cert_pem(&pem) {
                let diff = exp - chrono::Utc::now();
                (diff.num_days(), Some(exp.to_rfc3339()))
            } else {
                (-1, None)
            }
        } else {
            (-1, None)
        }
    };

    Ok(Json(CertStatusResponse {
        mode,
        domains,
        issuer,
        expires_at,
        days_remaining,
        error: None,
    }))
}

/// Return the persisted certificate configuration (admin-only) so the
/// Certificates page can pre-fill its form with what's actually configured —
/// mode, domains, LE email, challenge type, DNS provider + zone + credentials —
/// instead of showing defaults. Source of truth is the `[tls]` overlay written
/// by PUT (same trust boundary where credentials were set); falls back to the
/// boot config if no overlay exists. The UI masks the credential inputs.
pub async fn get_cert_config(
    State(state): State<AppState>,
) -> Result<Json<crate::config::TlsConfig>, (StatusCode, String)> {
    // Mirror update_cert_config's overlay location: <data_dir>/tls.toml.
    let data_dir = std::path::Path::new(&state.config.database.sqlite_path)
        .parent()
        .map(|p| p.to_path_buf())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("/var/lib/tcs"));
    let overlay_path = data_dir.join("tls.toml");

    let tls = if overlay_path.exists() {
        let raw = std::fs::read_to_string(&overlay_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("cannot read {}: {}", overlay_path.display(), e)))?;
        let table: toml::value::Table = raw
            .parse()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("invalid tls overlay: {e}")))?;
        table
            .get("tls")
            .cloned()
            .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "tls overlay missing [tls] section".to_string()))?
            .try_into::<crate::config::TlsConfig>()
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to parse [tls]: {e}")))?
    } else {
        state.config.tls.clone()
    };

    Ok(Json(tls))
}

#[derive(Deserialize)]
pub struct CertConfigRequest {
    pub mode: String,
    pub domains: Option<Vec<String>>,
    pub letsencrypt: Option<LetsEncryptConfigRequest>,
    pub self_signed: Option<SelfSignedConfigRequest>,
    pub provided: Option<ProvidedCertConfigRequest>,
}

#[derive(Deserialize)]
pub struct LetsEncryptConfigRequest {
    pub email: String,
    #[serde(default)]
    pub challenge_type: String,
    pub dns_provider: Option<DnsProviderConfigRequest>,
}

#[derive(Deserialize)]
pub struct DnsProviderConfigRequest {
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_secret: String,
    #[serde(default)]
    pub api_token: String,
    #[serde(default)]
    pub zone_id: String,
    #[serde(default)]
    pub dns_zone: String,
}

#[derive(Deserialize)]
pub struct SelfSignedConfigRequest {
    pub domains: Vec<String>,
}

#[derive(Deserialize)]
pub struct ProvidedCertConfigRequest {
    pub cert_path: String,
    pub key_path: String,
    #[serde(default)]
    pub ca_path: Option<String>,
}

/// Build a `TlsConfig` from the API request, ready for live reload.
fn build_tls_config_from_request(req: &CertConfigRequest) -> TlsConfig {
    let domains: Vec<String> = req
        .domains
        .clone()
        .or_else(|| req.self_signed.as_ref().map(|s| s.domains.clone()))
        .unwrap_or_default();

    let mode = match req.mode.as_str() {
        "letsencrypt" => crate::config::tls::TlsMode::LetsEncrypt,
        "self-signed" => crate::config::tls::TlsMode::SelfSigned,
        "provided" => crate::config::tls::TlsMode::Provided,
        _ => crate::config::tls::TlsMode::Disabled,
    };

    let letsencrypt = req.letsencrypt.as_ref().map(|le| {
        crate::config::tls::LetsEncryptConfig {
            domains: domains.clone(),
            email: le.email.clone(),
            challenge_type: match le.challenge_type.as_str() {
                "dns-01" => crate::config::tls::ChallengeType::Dns01,
                _ => crate::config::tls::ChallengeType::Http01,
            },
            dns_provider: le.dns_provider.as_ref().map(|dns| {
                crate::config::tls::DnsProviderConfig {
                    provider: dns.provider.clone(),
                    api_key: dns.api_key.clone(),
                    api_secret: dns.api_secret.clone(),
                    api_token: dns.api_token.clone(),
                    zone_id: dns.zone_id.clone(),
                    dns_zone: dns.dns_zone.clone(),
                }
            }),
        }
    });

    let self_signed = if req.mode == "self-signed" {
        Some(crate::config::tls::SelfSignedConfig {
            domains: if domains.is_empty() {
                vec!["localhost".to_string()]
            } else {
                domains.clone()
            },
        })
    } else {
        None
    };

    let provided = req.provided.as_ref().map(|p| {
        crate::config::tls::ProvidedCertConfig {
            cert_path: p.cert_path.clone(),
            key_path: p.key_path.clone(),
            ca_path: p.ca_path.clone(),
        }
    });

    TlsConfig {
        enabled: req.mode != "disabled",
        mode,
        letsencrypt,
        self_signed,
        provided,
    }
}

pub async fn update_cert_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CertConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".into()));
    }

    // Domains: prefer top-level, then nested self_signed / empty
    let domains = req
        .domains
        .clone()
        .or_else(|| req.self_signed.as_ref().map(|s| s.domains.clone()))
        .unwrap_or_default();

    if req.mode == "letsencrypt" {
        let le = req.letsencrypt.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "letsencrypt config required (email, challenge_type)".into(),
            )
        })?;
        if le.email.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "letsencrypt.email is required".into()));
        }
        if domains.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "at least one domain is required for Let's Encrypt".into(),
            ));
        }
    }

    let mut tls_table = toml::value::Table::new();
    tls_table.insert(
        "enabled".to_string(),
        toml::Value::Boolean(req.mode != "disabled"),
    );
    tls_table.insert("mode".to_string(), toml::Value::String(req.mode.clone()));

    if req.mode == "letsencrypt" {
        if let Some(le) = &req.letsencrypt {
            let mut le_table = toml::value::Table::new();
            le_table.insert(
                "domains".to_string(),
                toml::Value::Array(
                    domains
                        .iter()
                        .map(|d| toml::Value::String(d.clone()))
                        .collect(),
                ),
            );
            le_table.insert("email".to_string(), toml::Value::String(le.email.clone()));
            let challenge = if le.challenge_type.trim().is_empty() {
                "http-01".to_string()
            } else {
                le.challenge_type.clone()
            };
            le_table.insert(
                "challenge_type".to_string(),
                toml::Value::String(challenge.clone()),
            );
            if challenge == "dns-01" {
                if let Some(dns) = &le.dns_provider {
                    let mut dns_table = toml::value::Table::new();
                    dns_table.insert(
                        "provider".to_string(),
                        toml::Value::String(dns.provider.clone()),
                    );
                    dns_table.insert(
                        "api_key".to_string(),
                        toml::Value::String(dns.api_key.clone()),
                    );
                    dns_table.insert(
                        "api_secret".to_string(),
                        toml::Value::String(dns.api_secret.clone()),
                    );
                    dns_table.insert(
                        "api_token".to_string(),
                        toml::Value::String(dns.api_token.clone()),
                    );
                    dns_table.insert(
                        "zone_id".to_string(),
                        toml::Value::String(dns.zone_id.clone()),
                    );
                    if !dns.dns_zone.trim().is_empty() {
                        dns_table.insert(
                            "dns_zone".to_string(),
                            toml::Value::String(dns.dns_zone.clone()),
                        );
                    }
                    le_table.insert("dns_provider".to_string(), toml::Value::Table(dns_table));
                }
            }
            tls_table.insert("letsencrypt".to_string(), toml::Value::Table(le_table));
        }
    } else if req.mode == "self-signed" {
        let ss_domains = if domains.is_empty() {
            vec!["localhost".to_string()]
        } else {
            domains.clone()
        };
        let mut ss_table = toml::value::Table::new();
        ss_table.insert(
            "domains".to_string(),
            toml::Value::Array(
                ss_domains
                    .iter()
                    .map(|d| toml::Value::String(d.clone()))
                    .collect(),
            ),
        );
        tls_table.insert("self-signed".to_string(), toml::Value::Table(ss_table));
    } else if req.mode == "provided" {
        if let Some(prov) = &req.provided {
            let mut prov_table = toml::value::Table::new();
            prov_table.insert(
                "cert_path".to_string(),
                toml::Value::String(prov.cert_path.clone()),
            );
            prov_table.insert(
                "key_path".to_string(),
                toml::Value::String(prov.key_path.clone()),
            );
            if let Some(ca) = &prov.ca_path {
                prov_table.insert("ca_path".to_string(), toml::Value::String(ca.clone()));
            }
            tls_table.insert("provided".to_string(), toml::Value::Table(prov_table));
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                "provided cert_path and key_path required".into(),
            ));
        }
    }

    let mut config_data = toml::value::Table::new();
    config_data.insert("tls".to_string(), toml::Value::Table(tls_table));
    let config_str = toml::to_string_pretty(&config_data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Prefer writable data dir overlay (systemd ProtectSystem often makes /etc read-only).
    let data_dir = std::path::Path::new(&state.config.database.sqlite_path)
        .parent()
        .map(|p| p.to_path_buf())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("/var/lib/tcs"));
    let overlay_path = data_dir.join("tls.toml");
    std::fs::create_dir_all(&data_dir).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot create data dir: {}", e),
        )
    })?;
    std::fs::write(&overlay_path, &config_str).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "cannot write {}: {} (check ReadWritePaths for the data directory)",
                overlay_path.display(),
                e
            ),
        )
    })?;

    // Best-effort merge into main config.toml when it is writable
    let main_path = std::env::var("TCS_CONFIG").unwrap_or_else(|_| "/etc/tcs/config.toml".into());
    let mut wrote_main = false;
    if let Ok(existing) = std::fs::read_to_string(&main_path) {
        if let Ok(mut root) = existing.parse::<toml::Value>() {
            if let Some(table) = root.as_table_mut() {
                table.insert(
                    "tls".to_string(),
                    config_data.get("tls").cloned().unwrap_or(toml::Value::Table(toml::map::Map::new())),
                );
                if let Ok(merged) = toml::to_string_pretty(&root) {
                    if std::fs::write(&main_path, merged).is_ok() {
                        wrote_main = true;
                    }
                }
            }
        }
    }

    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "tls_config_update",
        &req.mode,
        &format!("overlay={} main_merged={}", overlay_path.display(), wrote_main),
    )
    .await;

    // Attempt live reload if TLS is currently active
    let live_result = if let Some(tls_runtime) = &state.tls_runtime {
        // Build TlsConfig directly from the request (avoids TOML round-trip)
        let new_tls_config = build_tls_config_from_request(&req);
        match tls_runtime.apply_mode(&new_tls_config).await {
            Ok(note) => Some(Ok(note)),
            Err(e) => Some(Err(e.to_string())),
        }
    } else {
        None
    };

    match live_result {
        Some(Ok(note)) => Ok(Json(serde_json::json!({
            "message": note,
            "mode": req.mode,
            "overlayPath": overlay_path.display().to_string(),
            "mainConfigMerged": wrote_main,
            "restartRequired": false,
            "appliedLive": true,
        }))),
        Some(Err(err)) => Ok(Json(serde_json::json!({
            "message": format!("Config saved but live reload failed: {}", err),
            "mode": req.mode,
            "overlayPath": overlay_path.display().to_string(),
            "mainConfigMerged": wrote_main,
            "restartRequired": true,
            "appliedLive": false,
        }))),
        None => Ok(Json(serde_json::json!({
            "message": "TLS config saved. Restart TCS to apply (systemctl restart tcs).",
            "mode": req.mode,
            "overlayPath": overlay_path.display().to_string(),
            "mainConfigMerged": wrote_main,
            "restartRequired": true,
            "appliedLive": false,
        }))),
    }
}

pub async fn renew_certificate(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Use live runtime mode when present (updated by Settings without restart)
    let tls_owned;
    let tls = if let Some(rt) = &state.tls_runtime {
        tls_owned = rt.tls.read().await.clone();
        &tls_owned
    } else {
        &state.config.tls
    };

    match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => {
            let le = tls.letsencrypt.as_ref()
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "Let's Encrypt not configured".to_string()))?;

            // If TlsRuntime is available, do live reload
            if let Some(tls_runtime) = &state.tls_runtime {
                let new_tls_config = crate::config::tls::TlsConfig {
                    enabled: true,
                    mode: crate::config::tls::TlsMode::LetsEncrypt,
                    letsencrypt: Some(crate::config::tls::LetsEncryptConfig {
                        domains: le.domains.clone(),
                        email: le.email.clone(),
                        challenge_type: le.challenge_type.clone(),
                        dns_provider: le.dns_provider.clone(),
                    }),
                    self_signed: None,
                    provided: None,
                };
                match tls_runtime.apply_mode(&new_tls_config).await {
                    Ok(note) => Ok(Json(serde_json::json!({
                        "message": note,
                        "mode": "letsencrypt",
                        "appliedLive": true,
                    }))),
                    Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                }
            } else {
                let acme = crate::cert::acme::AcmeClient::new(
                    &le.email,
                    le.dns_provider.as_ref().map(|d| crate::config::tls::DnsProviderConfig {
                        provider: d.provider.clone(),
                        api_key: d.api_key.clone(),
                        api_secret: d.api_secret.clone(),
                        api_token: d.api_token.clone(),
                        zone_id: d.zone_id.clone(),
                        dns_zone: d.dns_zone.clone(),
                    }),
                    le.challenge_type.clone(),
                )
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                let result = acme.renew_certificate(&le.domains).await;
                match result {
                    Ok(_) => Ok(Json(serde_json::json!({
                        "message": "Certificate renewed successfully (restart to apply)",
                        "mode": "letsencrypt",
                        "appliedLive": false,
                    }))),
                    Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                }
            }
        }
        crate::config::TlsMode::SelfSigned => {
            if let Some(tls_runtime) = &state.tls_runtime {
                let new_tls_config = crate::config::tls::TlsConfig {
                    enabled: true,
                    mode: crate::config::tls::TlsMode::SelfSigned,
                    self_signed: Some(crate::config::tls::SelfSignedConfig {
                        domains: tls.self_signed.as_ref()
                            .map(|c| c.domains.clone())
                            .unwrap_or_else(|| vec!["localhost".to_string()]),
                    }),
                    letsencrypt: None,
                    provided: None,
                };
                match tls_runtime.apply_mode(&new_tls_config).await {
                    Ok(note) => Ok(Json(serde_json::json!({
                        "message": note,
                        "mode": "self-signed",
                        "appliedLive": true,
                    }))),
                    Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                }
            } else {
                let domains = tls.self_signed.as_ref()
                    .map(|c| c.domains.clone())
                    .unwrap_or_else(|| vec!["localhost".to_string()]);

                let (cert, key) = crate::cert::self_signed::generate_self_signed(&domains)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                let data_dir = std::env::var("TCS_DATA_DIR").unwrap_or_else(|_| "/var/lib/tcs".into());
                let certs_dir = format!("{}/certs", data_dir);
                std::fs::create_dir_all(&certs_dir).ok();
                std::fs::write(format!("{}/cert.pem", certs_dir), &cert).ok();
                std::fs::write(format!("{}/key.pem", certs_dir), &key).ok();

                Ok(Json(serde_json::json!({
                    "message": "Self-signed certificate regenerated (restart to apply)",
                    "mode": "self-signed",
                    "appliedLive": false,
                })))
            }
        }
        _ => Err((StatusCode::BAD_REQUEST, format!("Cannot renew {} certificates", match &tls.mode {
            crate::config::TlsMode::Provided => "provided",
            crate::config::TlsMode::Disabled => "disabled",
            _ => "unknown",
        }))),
    }
}

// ─── Auth Settings ────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfigResponse {
    pub ldap: Option<LdapConfigResponse>,
    pub oidc: Option<OidcConfigResponse>,
    pub saml: Option<SamlConfigResponse>,
}

#[derive(Serialize)]
pub struct LdapConfigResponse {
    pub server: String,
    pub bind_dn: String,
    pub user_search_base: String,
    pub user_search_filter: String,
    pub default_role: String,
    pub group_role_mappings: Vec<GroupRoleMappingResponse>,
}

#[derive(Serialize)]
pub struct GroupRoleMappingResponse {
    pub group_dn_pattern: String,
    pub role: String,
}

#[derive(Serialize)]
pub struct OidcConfigResponse {
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SamlConfigResponse {
    pub enabled: bool,
    pub sp_entity_id: String,
    pub acs_url: String,
    pub idp_metadata_url: String,
    pub has_idp_sso_url: bool,
}

pub async fn get_auth_config(
    State(state): State<AppState>,
) -> Result<Json<AuthConfigResponse>, (StatusCode, String)> {
    let ldap = state.config.auth.ldap.as_ref().map(|l| LdapConfigResponse {
        server: l.url.clone(),
        bind_dn: String::new(),
        user_search_base: l.user_search_base.clone(),
        user_search_filter: l.user_search_filter.clone(),
        default_role: l.default_role.clone(),
        group_role_mappings: l.group_role_mappings.iter().map(|m| GroupRoleMappingResponse {
            group_dn_pattern: m.group_dn_pattern.clone(),
            role: m.role.clone(),
        }).collect(),
    });

    let oidc = state.config.auth.oidc.as_ref().map(|o| OidcConfigResponse {
        enabled: o.enabled,
        issuer_url: o.issuer_url.clone(),
        client_id: o.client_id.clone(),
        redirect_url: o.redirect_url.clone(),
        scopes: o.scopes.clone(),
    });

    let saml = state.config.auth.saml.as_ref().map(|s| SamlConfigResponse {
        enabled: s.enabled,
        sp_entity_id: s.sp_entity_id.clone(),
        acs_url: s.acs_url.clone(),
        idp_metadata_url: s.idp_metadata_url.clone(),
        has_idp_sso_url: !s.idp_sso_url.is_empty(),
    });

    Ok(Json(AuthConfigResponse { ldap, oidc, saml }))
}

#[derive(Deserialize)]
pub struct AuthConfigRequest {
    pub ldap: Option<AuthLdapRequest>,
    pub oidc: Option<AuthOidcRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthLdapRequest {
    pub server: String,
    pub bind_dn: String,
    pub bind_password: String,
    pub user_search_base: String,
    pub user_search_filter: String,
    pub default_role: String,
    pub group_role_mappings: Vec<AuthGroupMappingRequest>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthGroupMappingRequest {
    pub group_dn_pattern: String,
    pub role: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthOidcRequest {
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
}

pub async fn update_auth_config(
    State(state): State<AppState>,
    Json(req): Json<AuthConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config_path = "/etc/tcs/config.toml";
    let mut config_data = toml::value::Table::new();
    let mut auth_table = toml::value::Table::new();

    if let Some(ldap_req) = req.ldap {
        let mut ldap_table = toml::value::Table::new();
        // Config file field is `url` (not "server").
        ldap_table.insert("url".to_string(), toml::Value::String(ldap_req.server));
        ldap_table.insert("bind_dn".to_string(), toml::Value::String(ldap_req.bind_dn));
        ldap_table.insert(
            "bind_password".to_string(),
            toml::Value::String(ldap_req.bind_password),
        );
        ldap_table.insert(
            "user_search_base".to_string(),
            toml::Value::String(ldap_req.user_search_base),
        );
        ldap_table.insert(
            "user_search_filter".to_string(),
            toml::Value::String(ldap_req.user_search_filter),
        );
        ldap_table.insert(
            "default_role".to_string(),
            toml::Value::String(ldap_req.default_role),
        );
        
        let mut mappings = vec![];
        for m in ldap_req.group_role_mappings {
            let mut map_table = toml::value::Table::new();
            map_table.insert("group_dn_pattern".to_string(), toml::Value::String(m.group_dn_pattern));
            map_table.insert("role".to_string(), toml::Value::String(m.role));
            mappings.push(toml::Value::Table(map_table));
        }
        ldap_table.insert("group_role_mappings".to_string(), toml::Value::Array(mappings));
        auth_table.insert("ldap".to_string(), toml::Value::Table(ldap_table));
    }

    if let Some(oidc_req) = req.oidc {
        if oidc_req.enabled {
            let mut oidc_table = toml::value::Table::new();
            oidc_table.insert("enabled".to_string(), toml::Value::Boolean(true));
            oidc_table.insert("issuer_url".to_string(), toml::Value::String(oidc_req.issuer_url));
            oidc_table.insert("client_id".to_string(), toml::Value::String(oidc_req.client_id));
            oidc_table.insert("client_secret".to_string(), toml::Value::String(oidc_req.client_secret));
            oidc_table.insert("redirect_url".to_string(), toml::Value::String(oidc_req.redirect_url));
            oidc_table.insert("scopes".to_string(), toml::Value::Array(
                oidc_req.scopes.iter().map(|s| toml::Value::String(s.clone())).collect()
            ));
            auth_table.insert("oidc".to_string(), toml::Value::Table(oidc_table));
        }
    }

    config_data.insert("auth".to_string(), toml::Value::Table(auth_table));

    if let Some(path) = std::path::Path::new(config_path).parent() {
        std::fs::create_dir_all(path).ok();
    }
    let config_str = toml::to_string_pretty(&config_data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    std::fs::write(config_path, &config_str).ok();

    Ok(Json(serde_json::json!({
        "message": "Auth config updated. Restart required to apply changes."
    })))
}

// ─── Audit Log Handlers ───────────────────────────────────────────────

use crate::utils::audit::{AuditFilter, AuditEntry};

#[derive(Serialize)]
pub struct AuditLogResponse {
    pub entries: Vec<AuditEntry>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

pub async fn get_audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<AuditFilter>,
) -> Result<Json<AuditLogResponse>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    let (entries, total) = crate::utils::audit::get_entries(&state.db_pool, &filter)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "read",
        "audit_logs",
        &format!("page={}, per_page={}", filter.page, filter.per_page),
    ).await;

    Ok(Json(AuditLogResponse {
        entries,
        total,
        page: filter.page,
        per_page: filter.per_page,
    }))
}

pub async fn clear_audit_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    crate::utils::audit::clear_all(&state.db_pool).await.ok();

    crate::utils::audit::log_action(&state.db_pool, &claims.sub,
        "clear",
        "audit_logs",
        "All audit logs cleared",
    ).await;

    Ok(StatusCode::OK)
}

// ─── System Info Handler ──────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfoResponse {
    pub version: String,
    pub commit: String,
    pub build_time: String,
    pub database_backend: String,
    pub database_size_bytes: Option<u64>,
    pub uptime_seconds: u64,
    pub server_bind_addr: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub disk_usage: DiskUsageResponse,
    /// Alpha capability flags for UI gating
    pub features: serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsageResponse {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
}

use std::sync::atomic::{AtomicU64, Ordering};

static START_TIME: AtomicU64 = AtomicU64::new(0);

pub fn record_start_time() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    START_TIME.store(duration, Ordering::Relaxed);
}

pub fn get_uptime_seconds() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let start = START_TIME.load(Ordering::Relaxed);
    if start == 0 { now } else { now.saturating_sub(start) }
}

pub async fn get_system_info(
    State(state): State<AppState>,
) -> Json<SystemInfoResponse> {
    let db_config = &state.config.database;
    let db_backend = db_config.backend.to_string();

    let db_size = if db_config.backend == crate::config::DatabaseBackend::Sqlite {
        std::fs::metadata(&db_config.sqlite_path)
            .ok()
            .map(|m| m.len())
    } else {
        None
    };

    let disk_usage = get_disk_usage(&db_config.sqlite_path);

    Json(SystemInfoResponse {
        version: crate::utils::VERSION_INFO.version.clone(),
        commit: crate::utils::VERSION_INFO.commit.clone(),
        build_time: crate::utils::VERSION_INFO.build_time.clone(),
        database_backend: db_backend,
        database_size_bytes: db_size,
        uptime_seconds: get_uptime_seconds(),
        server_bind_addr: state.config.server.bind_addr.clone(),
        http_port: state.config.server.http_port,
        grpc_port: state.config.server.grpc_port,
        disk_usage,
        features: serde_json::json!({
            "clusterImport": true,
            "talosActions": true,
            "etcdBackup": true,
            "etcdRestore": true,
            "configApply": true,
            "machineUpgrade": true,
            "machineServices": true,
            "scheduledBackups": true,
            "clusterRollingUpgrade": true,
            "fleetUpgrade": true,
            "clusterProvision": true,
            "provisionConfigFactory": true,
            "siderolink": true,
            "siderolinkWireguard": true,
            "saml": true,
            "multiTenantBranding": true,
            "postgres": true,
            "multiReplicaHa": true,
            "machineReset": true,
            "provisionLifecycle": true,
        }),
    })
}

fn get_disk_usage(path: &str) -> DiskUsageResponse {
    let metadata = std::fs::metadata(path).ok();
    let total = metadata.as_ref().and_then(|m| {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let dev = m.dev();
            get_fs_usage_linux(dev)
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::MetadataExt;
            let dev = m.dev();
            get_fs_usage_macos(dev)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = dev;
            None
        }
    }).unwrap_or(DiskUsageResponse {
        total_bytes: 0,
        free_bytes: 0,
        used_bytes: 0,
    });
    total
}

#[cfg(target_os = "linux")]
fn get_fs_usage_linux(dev: u64) -> Option<DiskUsageResponse> {
    use std::fs::read_dir;
    let statfs_path = format!("/proc/self/mountinfo");
    let content = std::fs::read_to_string(&statfs_path).ok()?;
    
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() > 5 {
            let major_minor = parts[4].parse::<u64>().ok()?;
            if major_minor == dev {
                let mount_point = parts[5];
                let path = std::path::Path::new(mount_point);
                if let Ok(dir) = read_dir(path) {
                    let mut total = 0u64;
                    let mut free = 0u64;
                    for entry in dir.flatten() {
                        if let Ok(meta) = entry.metadata() {
                            if meta.is_file() {
                                total += meta.len();
                            }
                        }
                    }
                    return Some(DiskUsageResponse {
                        total_bytes: total,
                        free_bytes: free,
                        used_bytes: total,
                    });
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn get_fs_usage_macos(dev: u64) -> Option<DiskUsageResponse> {
    use std::os::unix::fs::MetadataExt;
    let path = std::path::Path::new("/");
    let meta = std::fs::metadata(path).ok()?;
    if meta.dev() == dev {
        Some(DiskUsageResponse {
            total_bytes: meta.len(),
            free_bytes: 0,
            used_bytes: meta.len(),
        })
    } else {
        None
    }
}

// ─── User CRUD Handlers ───────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub password: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserResponse {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
}

pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<CreateUserResponse>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    let password_hash = if let Some(ref pw) = req.password {
        Some(crate::auth::local::hash_password(pw)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?)
    } else {
        None
    };

    let now = chrono::Utc::now();
    let user = crate::db::models::auth::User {
        id: uuid::Uuid::new_v4(),
        email: req.email.clone(),
        display_name: req.display_name.clone(),
        role: req.role.clone(),
        is_active: req.is_active.unwrap_or(true),
        password_hash,
        auth_provider: "local".to_string(),
        ldap_dn: None,
        password_needs_change: req.password.is_some(),
        last_login: None,
        created_at: now,
        updated_at: now,
    };

    let created = repos::user::upsert(&state.db_pool, &user)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::utils::audit::log_action(&state.db_pool, &claims.sub,
        "create_user",
        &created.email,
        &format!("Role: {}", created.role),
    ).await;

    Ok((StatusCode::CREATED, Json(CreateUserResponse {
        id: created.id,
        email: created.email,
        display_name: created.display_name,
        role: created.role,
        is_active: created.is_active,
    })))
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserInfoResponse>, (StatusCode, String)> {
    let user = repos::user::get_by_id(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "User not found".to_string()))?;

    Ok(Json(UserInfoResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        role: user.role,
        is_active: user.is_active,
        last_login: user.last_login,
        auth_provider: user.auth_provider,
        password_needs_change: user.password_needs_change,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub role: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    pub password: Option<String>,
}

pub async fn update_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserInfoResponse>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    let mut user = repos::user::get_by_id(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let mut changes = Vec::new();

    if let Some(ref name) = req.display_name {
        changes.push(format!("display_name: {} -> {}", user.display_name, name));
        user.display_name = name.clone();
    }
    if let Some(ref role) = req.role {
        changes.push(format!("role: {} -> {}", user.role, role));
        user.role = role.clone();
    }
    if let Some(active) = req.is_active {
        if user.is_active != active {
            changes.push(format!("is_active: {} -> {}", user.is_active, active));
            user.is_active = active;
        }
    }
    if let Some(ref pw) = req.password {
        changes.push("password changed".to_string());
        user.password_hash = Some(crate::auth::local::hash_password(pw)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?);
    }

    user.updated_at = chrono::Utc::now();

    let updated = repos::user::upsert(&state.db_pool, &user)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::utils::audit::log_action(&state.db_pool, &claims.sub,
        "update_user",
        &updated.email,
        &changes.join(", "),
    ).await;

    Ok(Json(UserInfoResponse {
        id: updated.id,
        email: updated.email,
        display_name: updated.display_name,
        role: updated.role,
        is_active: updated.is_active,
        last_login: updated.last_login,
        auth_provider: updated.auth_provider,
        password_needs_change: updated.password_needs_change,
    }))
}

/// Admin: set a new password for a local user (lab recovery / force rotate).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminResetPasswordRequest {
    /// If omitted, a random 20-char password is generated and returned once.
    pub password: Option<String>,
    #[serde(default = "default_true_reset")]
    pub force_change: bool,
}

pub async fn admin_reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<AdminResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".into()));
    }
    let mut user = repos::user::get_by_id(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "User not found".into()))?;
    if user.auth_provider != "local" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only local users have passwords".into(),
        ));
    }
    let plain = payload.password.filter(|p| !p.is_empty()).unwrap_or_else(|| {
        let chars: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut rng = fastrand::Rng::new();
        (0..20)
            .map(|_| chars[rng.u32(0..chars.len() as u32) as usize] as char)
            .collect()
    });
    let hash = crate::auth::local::hash_password(&plain)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    user.password_hash = Some(hash);
    user.password_needs_change = payload.force_change;
    user.updated_at = chrono::Utc::now();
    repos::user::upsert(&state.db_pool, &user)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "admin_reset_password",
        &user.email,
        if payload.force_change {
            "force_change=true"
        } else {
            "force_change=false"
        },
    )
    .await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "email": user.email,
        "password": plain,
        "passwordNeedsChange": user.password_needs_change,
        "note": "Copy the password now; it is not stored in plain text.",
    })))
}

pub async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;

    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin role required".to_string()));
    }

    let user = repos::user::get_by_id(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Don't allow deleting the last admin
    let admins = state
        .db_pool
        .fetch_scalar_i64("SELECT COUNT(*) FROM users WHERE role = 'admin'", &[])
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if user.role == "admin" && admins <= 1 {
        return Err((StatusCode::BAD_REQUEST, "Cannot delete the last admin user".to_string()));
    }

    state
        .db_pool
        .execute(
            "DELETE FROM users WHERE id = ?",
            &[crate::db::SqlVal::Uuid(id)],
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::utils::audit::log_action(&state.db_pool, &claims.sub,
        "delete_user",
        &user.email,
        &format!("User {} deleted", user.display_name),
    ).await;

    Ok(StatusCode::NO_CONTENT)
}

// ─── Cluster Sub-routes ────────────────────────────────────────────────

pub async fn get_cluster_nodes(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    match repos::machine::list_by_cluster(&state.db_pool, cluster_id).await {
        Ok(machines) => {
            let vals: Result<Vec<_>, _> = machines.into_iter().map(serde_json::to_value).collect();
            match vals {
                Ok(v) => Ok(Json(v)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn get_cluster_machines(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    match repos::machine::list_by_cluster(&state.db_pool, cluster_id).await {
        Ok(machines) => {
            let mut result = Vec::new();
            for machine in machines {
                let v = machine_to_json(&machine)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                result.push(v);
            }
            Ok(Json(result))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ─── Config Patches ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateConfigPatchRequest {
    pub path: String,
    pub value: String,
    #[serde(default)]
    pub priority: i32,
    pub machine_id: Option<Uuid>,
}

pub async fn list_config_patches(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    match repos::config_patch::list_by_cluster(&state.db_pool, cluster_id).await {
        Ok(patches) => {
            let vals: Result<Vec<_>, _> = patches.into_iter().map(serde_json::to_value).collect();
            match vals {
                Ok(v) => Ok(Json(v)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn create_config_patch(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    Json(payload): Json<CreateConfigPatchRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    let mut patch = crate::db::models::config_patch::ConfigPatch::new(
        cluster_id,
        payload.path,
        payload.value,
        payload.priority,
    );
    patch.machine_id = payload.machine_id;

    match repos::config_patch::create(&state.db_pool, &patch).await {
        Ok(p) => match serde_json::to_value(p) {
            Ok(v) => Ok((StatusCode::CREATED, Json(v))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn delete_config_patch(
    State(state): State<AppState>,
    Path((cluster_id, patch_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    match repos::config_patch::delete(&state.db_pool, patch_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

// ─── Cluster Backups ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateBackupRequest {
    pub name: String,
}

pub async fn list_cluster_backups(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    match repos::cluster_backup::list_by_cluster(&state.db_pool, cluster_id).await {
        Ok(backups) => {
            let vals: Result<Vec<_>, _> = backups.into_iter().map(serde_json::to_value).collect();
            match vals {
                Ok(v) => Ok(Json(v)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn create_cluster_backup(
    State(state): State<AppState>,
    Path(cluster_id): Path<uuid::Uuid>,
    Json(payload): Json<CreateBackupRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller.create_etcd_backup(cluster_id, payload.name).await {
        Ok(b) => match serde_json::to_value(b) {
            Ok(v) => Ok((StatusCode::CREATED, Json(v))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Err(e) => {
            let status = match &e {
                crate::AppError::NotFound(_) => StatusCode::NOT_FOUND,
                crate::AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::BAD_GATEWAY,
            };
            Err((status, e.to_string()))
        }
    }
}

pub async fn download_cluster_backup(
    State(state): State<AppState>,
    Path((cluster_id, backup_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::body::Body;
    use axum::http::header;
    use axum::response::IntoResponse;
    use tokio_util::io::ReaderStream;

    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    let backup = match repos::cluster_backup::get(&state.db_pool, backup_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Backup not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    if backup.cluster_id != cluster_id {
        return Err((StatusCode::NOT_FOUND, "Backup not found".to_string()));
    }

    if backup.status != "ready" {
        return Err((
            StatusCode::CONFLICT,
            format!("Backup is not ready (status: {})", backup.status),
        ));
    }

    let path = backup.file_path.as_ref().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "Backup has no file on disk".to_string(),
        )
    })?;

    let file = tokio::fs::File::open(path).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("Backup file missing: {}", e),
        )
    })?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let filename = format!("{}.snapshot", backup.name);

    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{}\"", filename)
            .parse()
            .unwrap(),
    );
    Ok(response)
}

pub async fn delete_cluster_backup(
    State(state): State<AppState>,
    Path((cluster_id, backup_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, cluster_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Cluster not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }

    let backup = match repos::cluster_backup::get(&state.db_pool, backup_id).await {
        Ok(Some(b)) => b,
        Ok(None) => return Err((StatusCode::NOT_FOUND, "Backup not found".to_string())),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    if let Some(path) = &backup.file_path {
        let _ = tokio::fs::remove_file(path).await;
    }

    match repos::cluster_backup::delete(&state.db_pool, backup_id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

/// Disaster recovery: upload etcd snapshot to a control-plane node, optional bootstrap recover.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBackupRequest {
    /// Must be true — guards against accidental clicks.
    pub confirm: bool,
    /// After EtcdRecover, call Bootstrap with recover_etcd (default true).
    #[serde(default = "default_true")]
    pub run_bootstrap: bool,
    /// Pass recover_skip_hash_check to Bootstrap.
    #[serde(default)]
    pub skip_hash_check: bool,
    /// Optional control-plane machine to target; otherwise first CP with address / talosconfig endpoint.
    pub machine_id: Option<uuid::Uuid>,
}

fn default_true() -> bool {
    true
}

pub async fn restore_cluster_backup(
    State(state): State<AppState>,
    Path((cluster_id, backup_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(payload): Json<RestoreBackupRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = controller_for(&state);
    match controller
        .restore_etcd_backup(
            cluster_id,
            backup_id,
            payload.confirm,
            payload.run_bootstrap,
            payload.skip_hash_check,
            payload.machine_id,
        )
        .await
    {
        Ok(result) => {
            crate::utils::audit::log_action(
                &state.db_pool,
                "system",
                "etcd_restore",
                &cluster_id.to_string(),
                &format!("backup={} result={}", backup_id, result),
            )
            .await;
            Ok(Json(result))
        }
        Err(e) => {
            let status = match &e {
                crate::AppError::NotFound(_) => StatusCode::NOT_FOUND,
                crate::AppError::InvalidInput(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::BAD_GATEWAY,
            };
            Err((status, e.to_string()))
        }
    }
}

// ─── Rolling upgrades ──────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterUpgradeRequest {
    /// Legacy free-text installer image. Retained for backward compatibility;
    /// when `talosVersion`/`modules` are supplied the image is derived
    /// server-side and this field is ignored.
    #[serde(default)]
    pub image: Option<String>,
    /// Target Talos version (e.g. "v1.14.2"). Omit to keep the cluster's current.
    #[serde(default)]
    pub talos_version: Option<String>,
    /// Target Kubernetes version (e.g. "v1.36.4" or "v1.37.x"). Omit to skip
    /// the in-place k8s phase.
    #[serde(default)]
    pub k8s_version: Option<String>,
    /// New cluster-level module set. Omit to keep the stored set.
    #[serde(default)]
    pub modules: Option<Vec<String>>,
    #[serde(default = "default_max_unavail")]
    pub max_unavailable: i32,
    #[serde(default = "default_true")]
    pub control_plane_last: bool,
}

fn default_max_unavail() -> i32 { 1 }

pub async fn start_cluster_upgrade(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
    Json(payload): Json<ClusterUpgradeRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let ctrl = crate::controllers::UpgradeController::new(state.db_pool.clone());

    // Back-compat: a bare `image` with no version/modules means "roll to this
    // exact image". Translate it into a version-only request.
    let (talos_version, modules) = match (&payload.talos_version, &payload.modules) {
        (Some(v), m) => (Some(v.clone()), m.clone()),
        (None, Some(m)) => (None, Some(m.clone())),
        (None, None) => {
            match &payload.image {
                Some(img) if !img.trim().is_empty() => {
                    // Legacy: derive the version from the image tag if it looks
                    // like a plain installer ref; otherwise reject.
                    let tag = img.rsplit(':').next().unwrap_or("");
                    if tag.starts_with('v') || tag.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
                    {
                        (Some(tag.to_string()), None)
                    } else {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            "Provide talosVersion (and optionally modules/k8sVersion) — \
                             free-text installer images are no longer accepted."
                                .into(),
                        ));
                    }
                }
                _ => (None, None),
            }
        }
    };

    let (job, steps) = ctrl
        .start_cluster_upgrade(
            cluster_id,
            talos_version.as_deref(),
            payload.k8s_version.as_deref(),
            modules,
            payload.max_unavailable,
            payload.control_plane_last,
            &state.config.factory,
            Some(claims.sub.clone()),
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "cluster_upgrade_start",
        &cluster_id.to_string(),
        &format!(
            "job={} talos={:?} k8s={:?} steps={:?}",
            job.id, job.target_talos_version, job.target_k8s_version, steps
        ),
    )
    .await;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "job": job, "k8sSteps": steps })),
    ))
}

/// GET /clusters/:id/upgrade-targets — dropdown data for the rolling upgrade
/// panel: factory-buildable Talos versions + live k8s upgrade options.
pub async fn get_upgrade_targets(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let cluster = crate::db::repos::cluster::get(&state.db_pool, cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Cluster not found".to_string()))?;

    let factory = crate::integration::image_factory::ImageFactoryClient::new(
        &state.config.factory.normalized_base(),
    );
    // Degrade gracefully: if the factory version list or the k8s probe fails
    // (egress blocked, talosconfig missing, node flapping), still return what
    // we know so the panel renders. A 502 here blanks the whole UI.
    let (talos_versions, talos_note) = match factory.list_versions().await {
        Ok(v) => (v, String::new()),
        Err(e) => (Vec::new(), format!("Talos version list unavailable: {e}")),
    };

    let ctrl = controller_for(&state);
    let (k8s_targets, k8s_note) = match ctrl.k8s_upgrade_targets(cluster_id).await {
        Ok(v) => (v, String::new()),
        Err(e) => (
            serde_json::json!({ "current": cluster.control_plane_version, "supported": [] }),
            format!("Kubernetes upgrade targets unavailable: {e}"),
        ),
    };

    let mut notes = Vec::new();
    if !talos_note.is_empty() {
        notes.push(talos_note);
    }
    if !k8s_note.is_empty() {
        notes.push(k8s_note);
    }

    Ok(Json(serde_json::json!({
        "talos": {
            "current": cluster.talos_version,
            "versions": talos_versions,
        },
        "k8s": k8s_targets,
        "notes": notes,
    })))
}

pub async fn list_cluster_upgrade_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let jobs = crate::db::repos::upgrade_job::list_jobs_for_cluster(&state.db_pool, cluster_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        jobs.into_iter()
            .filter_map(|j| serde_json::to_value(j).ok())
            .collect(),
    ))
}

pub async fn get_upgrade_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let ctrl = crate::controllers::UpgradeController::new(state.db_pool.clone());
    ctrl.get_job_detail(id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

pub async fn cancel_upgrade_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    crate::db::repos::upgrade_job::request_cancel(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "upgrade_cancel",
        &id.to_string(),
        "",
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ─── SAML ──────────────────────────────────────────────────────────────

pub async fn saml_metadata(State(state): State<AppState>) -> Result<(StatusCode, String), (StatusCode, String)> {
    let cfg = state
        .config
        .auth
        .saml
        .as_ref()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "SAML not configured".into()))?;
    if !cfg.enabled {
        return Err((StatusCode::NOT_FOUND, "SAML disabled".into()));
    }
    let p = crate::auth::saml::SamlProvider::new(cfg.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::OK,
        p.sp_metadata_xml(),
    ))
}

pub async fn saml_login(
    State(state): State<AppState>,
) -> Result<axum::response::Redirect, (StatusCode, String)> {
    let cfg = state
        .config
        .auth
        .saml
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "SAML not configured".into()))?;
    if !cfg.enabled {
        return Err((StatusCode::BAD_GATEWAY, "SAML disabled".into()));
    }
    let p = crate::auth::saml::SamlProvider::new(cfg.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let url = p
        .login_redirect_url("")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::response::Redirect::temporary(&url))
}

#[derive(Deserialize)]
pub struct SamlAcsForm {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(default, rename = "RelayState")]
    pub relay_state: String,
}

pub async fn saml_acs(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<SamlAcsForm>,
) -> Result<axum::response::Html<String>, (StatusCode, String)> {
    let cfg = state
        .config
        .auth
        .saml
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "SAML not configured".into()))?;
    if !cfg.enabled {
        return Err((StatusCode::BAD_GATEWAY, "SAML disabled".into()));
    }
    let p = crate::auth::saml::SamlProvider::new(cfg.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let info = p
        .parse_response(&form.saml_response)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let token = p
        .authenticate_and_issue_jwt(&state.db_pool, info)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    let _ = form.relay_state;
    Ok(sso_token_html(&token))
}

// ─── Provision / greenfield config factory ─────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConfigRequest {
    pub bond_name: String,
    pub bond_interfaces: Vec<String>,
    pub bond_mode: String,
    #[serde(default)]
    pub bond_miimon: u32,
    #[serde(default)]
    pub bond_lacp_rate: String,
    pub vlan_name: String,
    pub vlan_interface: String,
    pub vlan_id: u32,
    pub subnet: String,
    pub gateway: String,
    #[serde(default)]
    pub dns: Vec<String>,
    #[serde(default)]
    pub mtu: Option<u32>,
    #[serde(default)]
    pub hostname: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateConfigRequest {
    pub name: String,
    pub endpoint: String,
    #[serde(default = "default_talos_ver")]
    pub talos_version: String,
    #[serde(default = "default_k8s_ver")]
    pub kubernetes_version: String,
    pub cluster_id: Option<Uuid>,
    #[serde(default)]
    pub network: Option<NetworkConfigRequest>,
    #[serde(default)]
    pub install_disk: Option<String>,
    #[serde(default)]
    pub wipe: bool,
    #[serde(default)]
    pub cert_sans: Vec<String>,
    #[serde(default)]
    pub cluster_domain: String,
}

fn default_talos_ver() -> String { "v1.13.7".into() }
fn default_k8s_ver() -> String { "v1.36.3".into() }

pub async fn generate_cluster_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GenerateConfigRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let ctrl = crate::controllers::ProvisionController::new(
        state.db_pool.clone(),
        state.config.auth.jwt_secret.clone(),
    );
    let network_config = payload.network.as_ref().map(|n|
        crate::controllers::provision::NetworkConfigParams {
            bond_name: n.bond_name.clone(),
            bond_interfaces: n.bond_interfaces.clone(),
            bond_mode: n.bond_mode.clone(),
            bond_miimon: n.bond_miimon,
            bond_lacp_rate: n.bond_lacp_rate.clone(),
            vlan_name: n.vlan_name.clone(),
            vlan_interface: n.vlan_interface.clone(),
            vlan_id: n.vlan_id,
            subnet: n.subnet.clone(),
            gateway: n.gateway.clone(),
            dns: if n.dns.is_empty() { vec!["172.24.16.254".into()] } else { n.dns.clone() },
            mtu: n.mtu,
            hostname: n.hostname.clone(),
        }
    );
    let art = ctrl
        .generate_config(
            &payload.name,
            &payload.endpoint,
            &payload.talos_version,
            &payload.kubernetes_version,
            payload.cluster_id,
            network_config,
            payload.install_disk.as_deref().unwrap_or("/dev/sda"),
            payload.wipe,
            &payload.cert_sans,
            &[],
            if payload.cluster_domain.is_empty() { "cluster.local" } else { &payload.cluster_domain },
            &siderolink_block_for_cluster(
                &state.db_pool,
                payload.cluster_id,
                &state.config.siderolink,
                &state.config.server,
            )
            .await,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "provision_generate",
        &art.id.to_string(),
        &art.name,
    )
    .await;
    // Do not return secrets_enc plaintext; return configs only
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": art.id,
            "name": art.name,
            "talosVersion": art.talos_version,
            "kubernetesVersion": art.kubernetes_version,
            "controlplaneConfig": art.controlplane_config,
            "workerConfig": art.worker_config,
            "hasSecrets": art.secrets_enc.is_some(),
            "createdAt": art.created_at,
        })),
    ))
}

pub async fn list_provision_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let ctrl = crate::controllers::ProvisionController::new(
        state.db_pool.clone(),
        state.config.auth.jwt_secret.clone(),
    );
    let list = ctrl
        .list()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        list.into_iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "talosVersion": a.talos_version,
                    "kubernetesVersion": a.kubernetes_version,
                    "clusterId": a.cluster_id,
                    "createdAt": a.created_at,
                })
            })
            .collect(),
    ))
}

pub async fn get_provision_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let ctrl = crate::controllers::ProvisionController::new(
        state.db_pool.clone(),
        state.config.auth.jwt_secret.clone(),
    );
    let art = ctrl
        .get(id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "id": art.id,
        "name": art.name,
        "talosVersion": art.talos_version,
        "kubernetesVersion": art.kubernetes_version,
        "controlplaneConfig": art.controlplane_config,
        "workerConfig": art.worker_config,
        "hasSecrets": art.secrets_enc.is_some(),
        "createdAt": art.created_at,
    })))
}

// ─── Siderolink inventory ──────────────────────────────────────────────

/// Render the `siderolink:` YAML block (2-space indented, trailing newline) to
/// bake into a cluster's generated machine configs, so provisioned nodes dial
/// in and form the WireGuard tunnel automatically. Returns an empty string when
/// there's no cluster to tie a token to. The block is omitted (empty) if we
/// cannot determine an endpoint, so config generation never breaks.
/// Build the standalone `SideroLinkConfig` machine-config document for a
/// cluster (the form Talos v1.10+ actually reconciles live). Appended to
/// generated machine configs so nodes dial into TCS's SideroLink API and bring
/// up a WireGuard tunnel. Delegates to the shared siderolink helper.
async fn siderolink_block_for_cluster(
    pool: &crate::db::pool::DbPool,
    cluster_id: Option<uuid::Uuid>,
    sl: &crate::config::SideroLinkConfig,
    server: &crate::config::ServerConfig,
) -> String {
    crate::siderolink::siderolink_doc_for_cluster(pool, cluster_id, sl, server).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiderolinkRegisterRequest {
    pub token: String,
    pub system_uuid: String,
    pub public_key: String,
}

pub async fn siderolink_register(
    State(state): State<AppState>,
    Json(payload): Json<SiderolinkRegisterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let ok = crate::db::repos::siderolink::validate_token(&state.db_pool, &payload.token)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !ok {
        return Err((StatusCode::UNAUTHORIZED, "Invalid join token".into()));
    }
    let existing = crate::db::repos::siderolink::find_by_uuid(&state.db_pool, &payload.system_uuid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let now = chrono::Utc::now();
    let peer = if let Some(mut p) = existing {
        p.public_key = payload.public_key;
        p.last_seen = now;
        p
    } else {
        let start = 0x6440_0000u32; // 100.64.0.0
        let ip = crate::db::repos::siderolink::next_ip(&state.db_pool, start)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        crate::db::repos::siderolink::SiderolinkPeer {
            id: Uuid::new_v4(),
            system_uuid: payload.system_uuid.clone(),
            public_key: payload.public_key,
            assigned_ip: ip,
            last_seen: now,
            created_at: now,
        }
    };
    crate::db::repos::siderolink::upsert_peer(&state.db_pool, &peer)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let wg_ok = state
        .siderolink_wg
        .set_peer(&peer.public_key, &peer.assigned_ip)
        .is_ok();
    // Best-effort mark machine connected if inventory exists
    if let Ok(machines) = repos::machine::list(&state.db_pool).await {
        for mut m in machines {
            if m.system_uuid == peer.system_uuid || m.system_uuid.contains(&peer.system_uuid) {
                m.siderolink_connected = true;
                if m.address.is_empty() {
                    m.address = peer.assigned_ip.clone();
                }
                m.updated_at = now;
                let _ = repos::machine::update(&state.db_pool, &m).await;
            }
        }
    }
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "peerId": peer.id,
            "assignedIp": peer.assigned_ip,
            "systemUuid": peer.system_uuid,
            "wireguard": {
                "enabled": state.siderolink_wg.enabled() && wg_ok,
                "serverPublicKey": state.siderolink_wg.server_public_key(),
                "endpoint": state.siderolink_wg.endpoint_hint(),
                "listenPort": state.siderolink_wg.listen_port(),
                "allowedIps": "100.64.0.0/10",
                "persistentKeepalive": 25,
            },
        })),
    ))
}

/// GET /clusters/:id/siderolink → { enabled, peers }
pub async fn get_cluster_siderolink(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (enabled, peers) = crate::controllers::ClusterController::new(state.db_pool.clone())
        .siderolink_status(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "enabled": enabled, "peers": peers })))
}

/// POST /clusters/:id/siderolink/enable — bake SideroLinkConfig into all nodes.
pub async fn enable_cluster_siderolink(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let doc = siderolink_block_for_cluster(&state.db_pool, Some(id), &state.config.siderolink, &state.config.server).await;
    let patched = crate::controllers::ClusterController::new(state.db_pool.clone())
        .siderolink_enable(id, &doc)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = crate::utils::audit::log_action(&state.db_pool, &claims.sub, "cluster_siderolink_enable", &format!("cluster {id}"), &format!("patched={patched}")).await;
    Ok(Json(serde_json::json!({
        "message": format!("Siderolink enabled — {patched} node(s) updated live (no reboot)"),
        "patched": patched,
    })))
}

/// POST /clusters/:id/siderolink/disable — strip SideroLinkConfig from all nodes.
pub async fn disable_cluster_siderolink(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    let patched = crate::controllers::ClusterController::new(state.db_pool.clone())
        .siderolink_disable(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _ = crate::utils::audit::log_action(&state.db_pool, &claims.sub, "cluster_siderolink_disable", &format!("cluster {id}"), &format!("patched={patched}")).await;
    Ok(Json(serde_json::json!({
        "message": format!("Siderolink disabled — {patched} node(s) updated live (no reboot)"),
        "patched": patched,
    })))
}

pub async fn siderolink_peers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let peers = crate::db::repos::siderolink::list_peers(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        peers
            .into_iter()
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect(),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateJoinTokenRequest {
    pub label: Option<String>,
    pub expires_hours: Option<i64>,
}

pub async fn create_siderolink_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreateJoinTokenRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    let token = format!("slj_{}", Uuid::new_v4().simple());
    let exp = payload
        .expires_hours
        .map(|h| chrono::Utc::now() + chrono::Duration::hours(h));
    crate::db::repos::siderolink::create_token(
        &state.db_pool,
        &token,
        payload.label.as_deref(),
        exp,
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "token": token, "expiresAt": exp })),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiderolinkClusterTokenQuery {
    pub cluster_id: Uuid,
}

fn siderolink_advertised_endpoint(sl: &crate::config::SideroLinkConfig, server: &crate::config::ServerConfig) -> String {
    if let Ok(e) = std::env::var("TCS_SIDEROLINK_ENDPOINT") {
        if !e.is_empty() {
            return e;
        }
    }
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

pub async fn get_cluster_siderolink_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SiderolinkClusterTokenQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let token = crate::db::repos::siderolink::get_cluster_token(&state.db_pool, q.cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "clusterId": q.cluster_id,
        "token": token,
        "endpoint": siderolink_advertised_endpoint(&state.config.siderolink, &state.config.server),
    })))
}

pub async fn rotate_cluster_siderolink_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SiderolinkClusterTokenQuery>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    let token = crate::controllers::ClusterController::new(state.db_pool.clone())
        .rotate_cluster_siderolink_token(payload.cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "siderolink_cluster_token_rotated",
        "siderolink",
        &format!("cluster:{}", payload.cluster_id),
    )
    .await;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "clusterId": payload.cluster_id,
            "token": token,
            "endpoint": siderolink_advertised_endpoint(&state.config.siderolink, &state.config.server),
        })),
    ))
}

pub async fn revoke_cluster_siderolink_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SiderolinkClusterTokenQuery>,
) -> Result<StatusCode, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    crate::controllers::ClusterController::new(state.db_pool.clone())
        .revoke_cluster_siderolink_token(payload.cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "siderolink_cluster_token_revoked",
        "siderolink",
        &format!("cluster:{}", payload.cluster_id),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_siderolink_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "Admin required".into()));
    }
    let tokens = crate::db::repos::siderolink::list_tokens(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        tokens
            .into_iter()
            .filter_map(|t| serde_json::to_value(t).ok())
            .collect(),
    ))
}

// ─── Metal / BMC / PXE ─────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBmcRequest {
    pub bmc_address: Option<String>,
    pub bmc_username: Option<String>,
    pub bmc_password: Option<String>,
    pub bmc_type: Option<String>,
    pub bmc_redfish_path: Option<String>,
    pub bmc_tls_insecure: Option<bool>,
}

pub async fn put_machine_bmc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<SetBmcRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let mut m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if let Some(a) = payload.bmc_address {
        m.bmc_address = a;
    }
    if let Some(u) = payload.bmc_username {
        m.bmc_username = u;
    }
    if let Some(t) = payload.bmc_type {
        m.bmc_type = t;
    }
    if let Some(p) = payload.bmc_redfish_path {
        m.bmc_redfish_path = p;
    }
    if let Some(i) = payload.bmc_tls_insecure {
        m.bmc_tls_insecure = i;
    }
    if let Some(pw) = payload.bmc_password.filter(|p| !p.is_empty()) {
        m.bmc_password_enc = Some(
            crate::utils::secrets::encrypt(&state.config.auth.jwt_secret, &pw)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
        );
    }
    m.updated_at = chrono::Utc::now();
    let m = repos::machine::update(&state.db_pool, &m)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "bmcAddress": m.bmc_address,
        "bmcUsername": m.bmc_username,
        "bmcType": m.bmc_type,
        "hasPassword": m.bmc_password_enc.as_ref().map(|p| !p.is_empty()).unwrap_or(false),
    })))
}

pub async fn get_machine_bmc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Ok(Json(serde_json::json!({
            "configured": false,
            "powerState": m.last_power_state,
        })));
    }
    let enc = m.bmc_password_enc.as_ref().unwrap();
    let plain = crate::utils::secrets::decrypt(&state.config.auth.jwt_secret, enc)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = crate::integration::bmc::BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    match crate::integration::bmc::BmcSession::connect(&creds).await {
        Ok(sess) => {
            let power = sess
                .get_power_state()
                .await
                .map(|p| p.as_str().to_string())
                .unwrap_or_else(|_| "unknown".into());
            // best-effort persist
            if let Ok(Some(mut mm)) = repos::machine::get(&state.db_pool, id).await {
                mm.last_power_state = power.clone();
                mm.updated_at = chrono::Utc::now();
                let _ = repos::machine::update(&state.db_pool, &mm).await;
            }
            Ok(Json(serde_json::json!({
                "configured": true,
                "protocol": sess.protocol().as_str(),
                "powerState": power,
                "bmcAddress": m.bmc_address,
                "bmcType": m.bmc_type,
            })))
        }
        Err(e) => Ok(Json(serde_json::json!({
            "configured": true,
            "error": e.to_string(),
            "powerState": m.last_power_state,
            "bmcAddress": m.bmc_address,
        }))),
    }
}

#[derive(Deserialize)]
pub struct PowerRequest {
    pub action: String,
}

pub async fn machine_power(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<PowerRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "BMC not configured".into()));
    }
    let plain = crate::utils::secrets::decrypt(
        &state.config.auth.jwt_secret,
        m.bmc_password_enc.as_ref().unwrap(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = crate::integration::bmc::BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let sess = crate::integration::bmc::BmcSession::connect(&creds)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    sess.power(&payload.action)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "bmc_power",
        &id.to_string(),
        &payload.action,
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "action": payload.action })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootDeviceRequest {
    pub target: String,
    #[serde(default = "default_true")]
    pub once: bool,
}

pub async fn machine_boot_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<BootDeviceRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "BMC not configured".into()));
    }
    let target = match payload.target.to_ascii_lowercase().as_str() {
        "pxe" => crate::integration::bmc::BootTarget::Pxe,
        "disk" | "hdd" => crate::integration::bmc::BootTarget::Disk,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown boot target: {other}"),
            ))
        }
    };
    let plain = crate::utils::secrets::decrypt(
        &state.config.auth.jwt_secret,
        m.bmc_password_enc.as_ref().unwrap(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = crate::integration::bmc::BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let sess = crate::integration::bmc::BmcSession::connect(&creds)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    sess.set_boot(target, payload.once)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "bmc_boot_device",
        &id.to_string(),
        &payload.target,
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoMountRequest {
    pub iso_url: String,
    #[serde(default = "default_cd")]
    pub media: String,
}

fn default_cd() -> String { "CD".to_string() }

pub async fn machine_mount_iso(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<IsoMountRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "BMC not configured".into()));
    }
    let plain = crate::utils::secrets::decrypt(
        &state.config.auth.jwt_secret,
        m.bmc_password_enc.as_ref().unwrap(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = crate::integration::bmc::BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let sess = crate::integration::bmc::BmcSession::connect(&creds)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    sess.mount_iso(&payload.iso_url, &payload.media)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "bmc_mount_iso",
        &id.to_string(),
        &payload.iso_url,
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "iso_url": payload.iso_url, "media": payload.media })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsoUnmountRequest {
    #[serde(default = "default_cd")]
    pub media: String,
}

pub async fn machine_unmount_iso(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<IsoUnmountRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let m = repos::machine::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Machine not found".into()))?;
    if !m.has_bmc() {
        return Err((StatusCode::BAD_REQUEST, "BMC not configured".into()));
    }
    let plain = crate::utils::secrets::decrypt(
        &state.config.auth.jwt_secret,
        m.bmc_password_enc.as_ref().unwrap(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = crate::integration::bmc::BmcCredentials::from_machine(
        &m,
        &plain,
        state.config.metal.bmc.connect_timeout_secs,
        &state.config.metal.bmc.ipmi_interface,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let sess = crate::integration::bmc::BmcSession::connect(&creds)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    sess.unmount_iso(&payload.media)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "bmc_unmount_iso",
        &id.to_string(),
        &payload.media,
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "media": payload.media })))
}

pub async fn metal_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let m = if let Some(rt) = &state.metal_runtime {
        rt.snapshot().await
    } else {
        state.config.metal.clone()
    };
    Ok(Json(serde_json::json!({
        "enabled": m.enabled,
        "liveReload": state.metal_runtime.is_some(),
        "dhcp": {
            "enabled": m.dhcp.enabled,
            "interface": m.dhcp.interface,
            "bindIp": m.dhcp.bind_ip,
            "subnet": m.dhcp.subnet,
            "rangeStart": m.dhcp.range_start,
            "rangeEnd": m.dhcp.range_end,
            "gateway": m.dhcp.gateway,
            "dns": m.dhcp.dns,
            "allowUnknown": m.dhcp.allow_unknown,
            "leaseTtlSecs": m.dhcp.lease_ttl_secs,
        },
        "pxe": {
            "enabled": m.pxe.enabled,
            "httpPort": m.pxe.http_port,
            "assetDir": m.pxe.asset_dir,
            "defaultTalosVersion": m.pxe.default_talos_version,
            "extraCmdline": m.pxe.extra_cmdline,
        },
        "bmc": {
            "connectTimeoutSecs": m.bmc.connect_timeout_secs,
            "preferRedfish": m.bmc.prefer_redfish,
        },
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetalConfigRequest {
    pub enabled: Option<bool>,
    pub dhcp: Option<UpdateMetalDhcp>,
    pub pxe: Option<UpdateMetalPxe>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetalDhcp {
    pub enabled: Option<bool>,
    pub interface: Option<String>,
    pub bind_ip: Option<String>,
    pub subnet: Option<String>,
    pub range_start: Option<String>,
    pub range_end: Option<String>,
    pub gateway: Option<String>,
    pub dns: Option<Vec<String>>,
    pub allow_unknown: Option<bool>,
    pub lease_ttl_secs: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetalPxe {
    pub enabled: Option<bool>,
    pub http_port: Option<u16>,
    pub asset_dir: Option<String>,
    pub default_talos_version: Option<String>,
    pub extra_cmdline: Option<String>,
}

pub async fn update_metal_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateMetalConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin required".into()));
    }
    let Some(rt) = &state.metal_runtime else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Metal runtime not available".into(),
        ));
    };
    let mut m = rt.snapshot().await;
    if let Some(e) = payload.enabled {
        m.enabled = e;
    }
    if let Some(d) = payload.dhcp {
        if let Some(v) = d.enabled {
            m.dhcp.enabled = v;
        }
        if let Some(v) = d.interface {
            m.dhcp.interface = v;
        }
        if let Some(v) = d.bind_ip {
            m.dhcp.bind_ip = v;
        }
        if let Some(v) = d.subnet {
            m.dhcp.subnet = v;
        }
        if let Some(v) = d.range_start {
            m.dhcp.range_start = v;
        }
        if let Some(v) = d.range_end {
            m.dhcp.range_end = v;
        }
        if let Some(v) = d.gateway {
            m.dhcp.gateway = v;
        }
        if let Some(v) = d.dns {
            m.dhcp.dns = v;
        }
        if let Some(v) = d.allow_unknown {
            m.dhcp.allow_unknown = v;
        }
        if let Some(v) = d.lease_ttl_secs {
            m.dhcp.lease_ttl_secs = v;
        }
    }
    if let Some(p) = payload.pxe {
        if let Some(v) = p.enabled {
            m.pxe.enabled = v;
        }
        if let Some(v) = p.http_port {
            m.pxe.http_port = v;
        }
        if let Some(v) = p.asset_dir {
            m.pxe.asset_dir = v;
        }
        if let Some(v) = p.default_talos_version {
            m.pxe.default_talos_version = v;
        }
        if let Some(v) = p.extra_cmdline {
            m.pxe.extra_cmdline = v;
        }
    }
    let m = rt
        .write_overlay_and_apply(m)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "update_metal_config",
        "metal",
        &format!("dhcp={} pxe={}", m.dhcp.enabled, m.pxe.enabled),
    )
    .await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "applied": true,
        "restartRequired": false,
        "enabled": m.enabled,
        "dhcp": {
            "enabled": m.dhcp.enabled,
            "interface": m.dhcp.interface,
            "bindIp": m.dhcp.bind_ip,
            "subnet": m.dhcp.subnet,
            "rangeStart": m.dhcp.range_start,
            "rangeEnd": m.dhcp.range_end,
            "gateway": m.dhcp.gateway,
            "allowUnknown": m.dhcp.allow_unknown,
        },
        "pxe": {
            "enabled": m.pxe.enabled,
            "httpPort": m.pxe.http_port,
            "assetDir": m.pxe.asset_dir,
            "defaultTalosVersion": m.pxe.default_talos_version,
        },
    })))
}

pub async fn list_dhcp_leases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let leases = repos::dhcp_lease::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "leases": leases })))
}

pub async fn list_pxe_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let list = repos::pxe_profile::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "profiles": list })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePxeProfileRequest {
    pub name: String,
    pub talos_version: String,
    pub arch: Option<String>,
    pub cmdline: Option<String>,
}

pub async fn create_pxe_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreatePxeProfileRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin required".into()));
    }
    let now = chrono::Utc::now();
    let p = repos::pxe_profile::PxeProfile {
        id: Uuid::new_v4(),
        name: payload.name,
        talos_version: payload.talos_version,
        arch: payload.arch.unwrap_or_else(|| "amd64".into()),
        kernel_url: String::new(),
        initramfs_url: String::new(),
        cmdline: payload.cmdline.unwrap_or_default(),
        enabled: true,
        assets_ready: false,
        created_at: now,
        updated_at: now,
    };
    repos::pxe_profile::create(&state.db_pool, &p)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(serde_json::to_value(&p).unwrap())))
}

pub async fn sync_pxe_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, "admin required".into()));
    }
    let mut p = repos::pxe_profile::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "profile not found".into()))?;
    crate::network::pxe::sync_profile_assets(&state.config.metal.pxe, &mut p)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    repos::pxe_profile::update(&state.db_pool, &p)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::to_value(&p).unwrap()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMetalProvisionRequest {
    pub machine_ids: Vec<Uuid>,
    pub artifact_id: Option<Uuid>,
    pub install_disk: Option<String>,
    #[serde(default = "default_true")]
    pub auto_bootstrap: bool,
}

pub async fn start_cluster_provision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cluster_id): Path<Uuid>,
    Json(payload): Json<StartMetalProvisionRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    if payload.machine_ids.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "machineIds required".into()));
    }
    if repos::cluster::get(&state.db_pool, cluster_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "cluster not found".into()));
    }
    let metal_payload = crate::runtime::metal_scheduler::MetalJobPayload {
        machine_ids: payload.machine_ids,
        artifact_id: payload.artifact_id,
        install_disk: payload.install_disk,
        auto_bootstrap: payload.auto_bootstrap,
        current_machine_index: 0,
        step: "pending".into(),
        steps_log: vec![format!("{} job created", chrono::Utc::now().to_rfc3339())],
        job_artifact_id: None,
    };
    let now = chrono::Utc::now();
    let job = repos::provision_job::ProvisionJob {
        id: Uuid::new_v4(),
        cluster_id: Some(cluster_id),
        kind: "metal_provision".into(),
        status: "pending".into(),
        desired_workers: 0,
        payload: Some(serde_json::to_string(&metal_payload).unwrap_or_else(|_| "{}".into())),
        error: None,
        created_by: Some(claims.sub.clone()),
        created_at: now,
        updated_at: now,
    };
    repos::provision_job::create(&state.db_pool, &job)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "start_metal_provision",
        &job.id.to_string(),
        &cluster_id.to_string(),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": job.id,
            "status": job.status,
            "kind": job.kind,
        })),
    ))
}

pub async fn list_provision_jobs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let jobs = repos::provision_job::list(&state.db_pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "jobs": jobs })))
}

pub async fn get_provision_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let job = repos::provision_job::get(&state.db_pool, id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "job not found".into()))?;
    let mut val = serde_json::to_value(&job).unwrap();
    if let Some(p) = &job.payload {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(p) {
            val["payloadParsed"] = parsed;
        }
    }
    Ok(Json(val))
}

pub async fn cancel_provision_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    repos::provision_job::update_status(&state.db_pool, id, "cancelled", Some("cancelled by user"), None)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}


// ─── Per-machine Talos config editor ───────────────────────────────────

pub async fn get_machine_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let ctrl = controller_for(&state);
    let desired = ctrl
        .get_desired_machine_config(id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let live_ok = ctrl.get_live_machine_config(id).await.is_ok();
    Ok(Json(serde_json::json!({
        "machineId": id,
        "hasDesired": desired.is_some(),
        "desiredConfig": desired,
        "liveReachable": live_ok,
    })))
}

pub async fn get_machine_config_live(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let ctrl = controller_for(&state);
    let yaml = ctrl
        .get_live_machine_config(id)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "machineId": id,
        "configYaml": yaml,
        "source": "live",
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutMachineConfigRequest {
    pub config_yaml: String,
}

pub async fn put_machine_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<PutMachineConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let ctrl = controller_for(&state);
    ctrl.set_desired_machine_config(id, &payload.config_yaml)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "save_machine_config",
        &id.to_string(),
        "",
    )
    .await;
    Ok(Json(serde_json::json!({ "ok": true, "bytes": payload.config_yaml.len() })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyMachineConfigRequest {
    pub config_yaml: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub reboot: bool,
    /// If true, strategic-merge body/desired onto live config before apply.
    #[serde(default)]
    pub merge_with_live: bool,
}

pub async fn apply_machine_config_editor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ApplyMachineConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let ctrl = controller_for(&state);
    let res = ctrl
        .apply_machine_config_ex(
            id,
            payload.config_yaml.as_deref(),
            payload.dry_run,
            payload.reboot,
            payload.merge_with_live,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "apply_machine_config",
        &id.to_string(),
        if payload.dry_run { "dry_run" } else { "apply" },
    )
    .await;
    Ok(Json(res))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineConfigHelpersRequest {
    pub install_image: Option<String>,
    pub network_yaml: Option<String>,
    pub extra_mounts_yaml: Option<String>,
    pub hostname: Option<String>,
    /// When no desired config exists, pull live as base.
    #[serde(default = "default_true")]
    pub base_from_live: bool,
}

pub async fn machine_config_helpers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<MachineConfigHelpersRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let ctrl = controller_for(&state);
    let yaml = ctrl
        .apply_machine_config_helpers(
            id,
            payload.install_image.as_deref(),
            payload.network_yaml.as_deref(),
            payload.extra_mounts_yaml.as_deref(),
            payload.hostname.as_deref(),
            payload.base_from_live,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "machine_config_helpers",
        &id.to_string(),
        "",
    )
    .await;
    Ok(Json(serde_json::json!({
        "ok": true,
        "desiredConfig": yaml,
        "bytes": yaml.len(),
    })))
}

// ─── Inventory import ──────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryImportRequest {
    pub format: String,
    pub content: String,
    pub cluster_id: Option<Uuid>,
    pub create_cluster: Option<bool>,
    pub create_cluster_name: Option<String>,
    #[serde(default = "default_true")]
    pub upsert_by_mac: bool,
}

pub async fn preview_machine_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<InventoryImportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _ = extract_claims(&headers)?;
    let doc = crate::controllers::inventory::parse_inventory(&payload.format, &payload.content)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let preview = crate::controllers::inventory::preview_inventory(&doc);
    serde_json::to_value(preview)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn import_machines(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<InventoryImportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let claims = extract_claims(&headers)?;
    if claims.role != "admin" && claims.role != "operator" {
        return Err((StatusCode::FORBIDDEN, "operator or admin required".into()));
    }
    let doc = crate::controllers::inventory::parse_inventory(&payload.format, &payload.content)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let create_name = if payload.create_cluster.unwrap_or(false) {
        payload
            .create_cluster_name
            .or_else(|| doc.cluster.as_ref().and_then(|c| c.name.clone()))
    } else {
        None
    };
    let result = crate::controllers::inventory::apply_inventory(
        &state.db_pool,
        &state.config.auth.jwt_secret,
        &doc,
        payload.cluster_id,
        payload.upsert_by_mac,
        create_name.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    crate::utils::audit::log_action(
        &state.db_pool,
        &claims.sub,
        "import_machines",
        &format!("created={} updated={}", result.created, result.updated),
        "",
    )
    .await;
    serde_json::to_value(result)
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}
