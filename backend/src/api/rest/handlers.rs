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
use crate::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: crate::utils::version::VERSION_INFO.version.clone(),
    })
}

#[derive(Serialize)]
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

pub async fn get_branding(
    State(state): State<AppState>,
) -> Json<BrandingResponse> {
    let branding = state.branding.get_branding("default").await;

    Json(BrandingResponse {
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
    })
}

#[derive(Deserialize)]
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
    Json(payload): Json<UpdateBrandingRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let branding = TenantBranding {
        tenant_id: "default".to_string(),
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
) -> (StatusCode, String) {
    let branding = state.branding.get_branding("default").await;
    let css = crate::branding::theme::generate_css_variables(&branding);

    (StatusCode::OK, css)
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
pub struct CreateClusterRequest {
    pub name: String,
    #[serde(alias = "controlPlaneVersion")]
    pub control_plane_version: String,
    #[serde(alias = "talosVersion")]
    pub talos_version: String,
}

pub async fn create_cluster(
    State(state): State<AppState>,
    Json(payload): Json<CreateClusterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    // Inventory-only: does not provision Talos or Kubernetes.
    let cluster = crate::db::models::cluster::Cluster::new(
        payload.name,
        payload.control_plane_version,
        payload.talos_version,
    );

    match repos::cluster::create(&state.db_pool, &cluster).await {
        Ok(c) => Ok((StatusCode::CREATED, Json(cluster_public_json(c)))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn list_clusters(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::list(&state.db_pool).await {
        Ok(clusters) => Ok(Json(
            clusters.into_iter().map(cluster_public_json).collect(),
        )),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
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
    match repos::cluster::delete(&state.db_pool, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

pub async fn list_machines(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::machine::list(&state.db_pool).await {
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

pub async fn get_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::machine::get(&state.db_pool, id).await {
        Ok(Some(machine)) => match serde_json::to_value(machine) {
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

#[derive(Deserialize)]
pub struct UpdateMachineRequest {
    pub address: Option<String>,
}

pub async fn update_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<UpdateMachineRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(addr) = payload.address {
        let controller = controller_for(&state);
        match controller.update_machine_address(id, addr).await {
            Ok(m) => match serde_json::to_value(m) {
                Ok(v) => Ok(Json(v)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            },
            Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
        }
    } else {
        Err((StatusCode::BAD_REQUEST, "No fields to update".to_string()))
    }
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
    let user = repos::user::get_by_email(&state.db_pool, &payload.email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid email or password".to_string()))?;

    if !user.is_active {
        return Err((StatusCode::UNAUTHORIZED, "Account is disabled".to_string()));
    }

    let authenticated_user = match user.auth_provider.as_str() {
        "local" => {
            authenticate_local(&state.db_pool, &payload.email, &payload.password)
                .await
                .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?
        }
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
        _ => {
            return Err((StatusCode::UNAUTHORIZED,
                format!("Auth provider '{}' not supported for login", user.auth_provider)));
        }
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

    let state_param = Uuid::new_v4().to_string();

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
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let oidc_config = state.config.auth.oidc
        .as_ref()
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "OIDC is not configured".to_string()))?;

    let provider = crate::auth::TcsOidcProvider::new(oidc_config.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_info = provider.exchange_code(&params.code, &oidc_config.redirect_url)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let token = provider.authenticate_and_issue_jwt(&state.db_pool, user_info.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = repos::user::get_by_email(&state.db_pool, &user_info.email)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::INTERNAL_SERVER_ERROR, "User not found after OIDC auth".to_string()))?;

    Ok(Json(LoginResponse { token, user }))
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
    let tls = &state.config.tls;
    let mode = match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => "letsencrypt".to_string(),
        crate::config::TlsMode::SelfSigned => "self-signed".to_string(),
        crate::config::TlsMode::Provided => "provided".to_string(),
        crate::config::TlsMode::Disabled => "disabled".to_string(),
    };

    let (domains, issuer, expires_at) = match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => {
            let le = tls.letsencrypt.as_ref();
            (
                le.map(|c| c.domains.clone()).unwrap_or_default(),
                "Let's Encrypt".to_string(),
                None,
            )
        }
        crate::config::TlsMode::SelfSigned => (
            tls.self_signed.as_ref().map(|c| c.domains.clone()).unwrap_or_else(|| vec!["localhost".to_string()]),
            "Self-Signed".to_string(),
            None,
        ),
        crate::config::TlsMode::Provided => {
            let prov = tls.provided.as_ref();
            (
                vec![],
                "Custom".to_string(),
                None::<String>,
            )
        }
        crate::config::TlsMode::Disabled => (vec![], "None".to_string(), None),
    };

    // Try to compute days remaining from cert on disk
    let cert_path = "/var/lib/tcs/certs/cert.pem";
    let days_remaining = if let Ok(pem) = std::fs::read_to_string(cert_path) {
        if let Some(exp) = crate::cert::provided::parse_expiry_from_cert_pem(&pem) {
            let diff = exp - chrono::Utc::now();
            diff.num_days()
        } else {
            -1
        }
    } else {
        -1
    };

    Ok(Json(CertStatusResponse {
        mode,
        domains,
        issuer,
        expires_at: None,
        days_remaining,
        error: None,
    }))
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

pub async fn update_cert_config(
    State(state): State<AppState>,
    Json(req): Json<CertConfigRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Write TLS config to TOML file
    let config_path = "/etc/tcs/config.toml";
    
    let mut config_data = toml::value::Table::new();
    let mut tls_table = toml::value::Table::new();
    tls_table.insert("enabled".to_string(), toml::Value::Boolean(req.mode != "disabled"));
    tls_table.insert("mode".to_string(), toml::Value::String(req.mode.clone()));

    if req.mode == "letsencrypt" {
        if let Some(le) = req.letsencrypt {
            let mut le_table = toml::value::Table::new();
            le_table.insert("domains".to_string(), toml::Value::Array(
                req.domains.unwrap_or_default().iter().map(|d| toml::Value::String(d.clone())).collect()
            ));
            le_table.insert("email".to_string(), toml::Value::String(le.email));
            le_table.insert("challenge_type".to_string(), toml::Value::String(le.challenge_type));
            tls_table.insert("letsencrypt".to_string(), toml::Value::Table(le_table));
        }
    } else if req.mode == "self-signed" {
        if let Some(ss) = req.self_signed {
            let mut ss_table = toml::value::Table::new();
            ss_table.insert("domains".to_string(), toml::Value::Array(
                ss.domains.iter().map(|d| toml::Value::String(d.clone())).collect()
            ));
            tls_table.insert("self-signed".to_string(), toml::Value::Table(ss_table));
        }
    } else if req.mode == "provided" {
        if let Some(prov) = req.provided {
            let mut prov_table = toml::value::Table::new();
            prov_table.insert("cert_path".to_string(), toml::Value::String(prov.cert_path));
            prov_table.insert("key_path".to_string(), toml::Value::String(prov.key_path));
            if let Some(ca) = prov.ca_path {
                prov_table.insert("ca_path".to_string(), toml::Value::String(ca));
            }
            tls_table.insert("provided".to_string(), toml::Value::Table(prov_table));
        }
    }

    config_data.insert("tls".to_string(), toml::Value::Table(tls_table));

    if let Some(path) = std::path::Path::new(config_path).parent() {
        std::fs::create_dir_all(path).ok();
    }
    let config_str = toml::to_string_pretty(&config_data)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    std::fs::write(config_path, &config_str).ok();

    // Restart would be needed for changes to take effect — note this in response
    Ok(Json(serde_json::json!({
        "message": "TLS config updated. Restart required to apply changes.",
        "mode": req.mode
    })))
}

pub async fn renew_certificate(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let tls = &state.config.tls;
    
    match &tls.mode {
        crate::config::TlsMode::LetsEncrypt => {
            let le = tls.letsencrypt.as_ref()
                .ok_or_else(|| (StatusCode::BAD_REQUEST, "Let's Encrypt not configured".to_string()))?;
            
            // Trigger ACME renewal
            let acme = crate::cert::acme::AcmeClient::new(
                &le.email,
                le.dns_provider.as_ref().map(|d| crate::config::tls::DnsProviderConfig {
                    provider: d.provider.clone(),
                    api_key: d.api_key.clone(),
                    api_secret: d.api_secret.clone(),
                    api_token: d.api_token.clone(),
                    zone_id: d.zone_id.clone(),
                }),
                le.challenge_type.clone(),
            )
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            
            let result = acme.renew_certificate(&le.domains).await;
            match result {
                Ok(_) => Ok(Json(serde_json::json!({
                    "message": "Certificate renewed successfully",
                    "mode": "letsencrypt"
                }))),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        crate::config::TlsMode::SelfSigned => {
            let domains = tls.self_signed.as_ref()
                .map(|c| c.domains.clone())
                .unwrap_or_else(|| vec!["localhost".to_string()]);
            
            let (cert, key) = crate::cert::self_signed::generate_self_signed(&domains)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            
            // Write to disk
            std::fs::create_dir_all("/var/lib/tcs/certs/").ok();
            std::fs::write("/var/lib/tcs/certs/cert.pem", &cert).ok();
            std::fs::write("/var/lib/tcs/certs/key.pem", &key).ok();
            
            Ok(Json(serde_json::json!({
                "message": "Self-signed certificate regenerated",
                "mode": "self-signed"
            })))
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
pub struct AuthConfigResponse {
    pub ldap: Option<LdapConfigResponse>,
    pub oidc: Option<OidcConfigResponse>,
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
        enabled: true,
        issuer_url: o.issuer_url.clone(),
        client_id: o.client_id.clone(),
        redirect_url: o.redirect_url.clone(),
        scopes: o.scopes.clone(),
    });

    Ok(Json(AuthConfigResponse { ldap, oidc }))
}

#[derive(Deserialize)]
pub struct AuthConfigRequest {
    pub ldap: Option<AuthLdapRequest>,
    pub oidc: Option<AuthOidcRequest>,
}

#[derive(Deserialize)]
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
pub struct AuthGroupMappingRequest {
    pub group_dn_pattern: String,
    pub role: String,
}

#[derive(Deserialize)]
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
        ldap_table.insert("server".to_string(), toml::Value::String(ldap_req.server));
        ldap_table.insert("bind_dn".to_string(), toml::Value::String(ldap_req.bind_dn));
        ldap_table.insert("bind_password".to_string(), toml::Value::String(ldap_req.bind_password));
        ldap_table.insert("user_search_base".to_string(), toml::Value::String(ldap_req.user_search_base));
        ldap_table.insert("user_search_filter".to_string(), toml::Value::String(ldap_req.user_search_filter));
        ldap_table.insert("default_role".to_string(), toml::Value::String(ldap_req.default_role));
        
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
            "clusterProvision": false,
            "siderolink": false,
            "saml": false,
            "postgres": false,
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
pub struct CreateUserRequest {
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub password: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Serialize)]
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
    let admins: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE role = 'admin'"
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if user.role == "admin" && admins <= 1 {
        return Err((StatusCode::BAD_REQUEST, "Cannot delete the last admin user".to_string()));
    }

    sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&state.db_pool)
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
                let mut map = serde_json::Map::new();
                let mval = serde_json::to_value(&machine)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if let Some(obj) = mval.as_object() {
                    for (k, v) in obj {
                        map.insert(k.clone(), v.clone());
                    }
                }
                map.insert("cluster_id".to_string(), serde_json::Value::String(cluster_id.to_string()));
                result.push(serde_json::Value::Object(map));
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

    let patch = crate::db::models::config_patch::ConfigPatch::new(
        cluster_id,
        payload.path,
        payload.value,
        payload.priority,
    );

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

// ─── Machine Classes ───────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMachineClassRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub min_cpu: i32,
    pub min_memory: i64,
    pub min_disk: i64,
    pub arch: String,
    #[serde(default)]
    pub secure_boot: bool,
    #[serde(default)]
    pub allowed_roles: Vec<String>,
}

pub async fn create_machine_class(
    State(state): State<AppState>,
    Json(payload): Json<CreateMachineClassRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let mc = crate::db::models::machine_class::MachineClass::new(
        payload.name,
        payload.description,
        payload.min_cpu,
        payload.min_memory,
        payload.min_disk,
        payload.arch,
        payload.secure_boot,
        payload.allowed_roles,
    );

    match repos::machine_class::create(&state.db_pool, &mc).await {
        Ok(c) => match serde_json::to_value(c) {
            Ok(v) => Ok((StatusCode::CREATED, Json(v))),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn list_machine_classes(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::machine_class::list(&state.db_pool).await {
        Ok(classes) => {
            let vals: Result<Vec<_>, _> = classes.into_iter().map(serde_json::to_value).collect();
            match vals {
                Ok(v) => Ok(Json(v)),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn get_machine_class(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::machine_class::get(&state.db_pool, id).await {
        Ok(Some(mc)) => match serde_json::to_value(mc) {
            Ok(v) => Ok(Json(v)),
            Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        },
        Ok(None) => Err((StatusCode::NOT_FOUND, "Machine class not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn update_machine_class(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<CreateMachineClassRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::machine_class::get(&state.db_pool, id).await {
        Ok(Some(mut mc)) => {
            mc.name = payload.name;
            mc.description = payload.description;
            mc.min_cpu = payload.min_cpu;
            mc.min_memory = payload.min_memory;
            mc.min_disk = payload.min_disk;
            mc.arch = payload.arch;
            mc.secure_boot = payload.secure_boot;
            mc.allowed_roles = payload.allowed_roles;
            mc.updated_at = chrono::Utc::now();
            match repos::machine_class::update(&state.db_pool, &mc).await {
                Ok(c) => match serde_json::to_value(c) {
                    Ok(v) => Ok(Json(v)),
                    Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                },
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Machine class not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn delete_machine_class(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    match repos::machine_class::delete(&state.db_pool, id).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}
