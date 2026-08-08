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
    let branding = state.branding.get_branding("default");

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
    let branding = state.branding.get_branding("default");
    let css = crate::branding::theme::generate_css_variables(&branding);

    (StatusCode::OK, css)
}

pub async fn get_logo(
    State(state): State<AppState>,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let branding = state.branding.get_branding("default");
    let svg = crate::branding::generator::generate_logo_svg(&branding);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/svg+xml".parse().unwrap());

    (StatusCode::OK, headers, svg)
}

pub async fn get_favicon(
    State(state): State<AppState>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let branding = state.branding.get_branding("default");
    let png = crate::branding::generator::generate_favicon_png(&branding);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "image/png".parse().unwrap());

    (StatusCode::OK, headers, png)
}

#[derive(Deserialize)]
pub struct CreateClusterRequest {
    pub name: String,
    pub control_plane_version: String,
    pub talos_version: String,
}

pub async fn create_cluster(
    State(state): State<AppState>,
    Json(payload): Json<CreateClusterRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let cluster = crate::db::models::cluster::Cluster::new(
        payload.name,
        payload.control_plane_version,
        payload.talos_version,
    );

    match repos::cluster::create(&state.db_pool, &cluster).await {
        Ok(c) => Ok((StatusCode::CREATED, Json(serde_json::to_value(c).unwrap()))),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn list_clusters(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    match repos::cluster::list(&state.db_pool).await {
        Ok(clusters) => Ok(Json(clusters.into_iter().map(|c| serde_json::to_value(c).unwrap()).collect())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn get_cluster(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::cluster::get(&state.db_pool, id).await {
        Ok(Some(cluster)) => Ok(Json(serde_json::to_value(cluster).unwrap())),
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
                Ok(c) => Ok(Json(serde_json::to_value(c).unwrap())),
                Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
            }
        },
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
        Ok(machines) => Ok(Json(machines.into_iter().map(|m| serde_json::to_value(m).unwrap()).collect())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

pub async fn get_machine(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match repos::machine::get(&state.db_pool, id).await {
        Ok(Some(machine)) => Ok(Json(serde_json::to_value(machine).unwrap())),
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
}

#[derive(Serialize)]
pub struct ImportClusterResponse {
    pub cluster: serde_json::Value,
    pub machines_imported: i32,
}

pub async fn import_cluster(
    State(state): State<AppState>,
    Json(payload): Json<ImportClusterRequest>,
) -> Result<(StatusCode, Json<ImportClusterResponse>), (StatusCode, String)> {
    let controller = crate::controllers::cluster::ClusterController::new(state.db_pool.clone());

    match controller.import_cluster(payload.name, payload.kubeconfig).await {
        Ok(cluster) => {
            let machines = crate::db::repos::machine::list_by_cluster(&state.db_pool, cluster.id).await
                .unwrap_or_default();

            Ok((StatusCode::CREATED, Json(ImportClusterResponse {
                cluster: serde_json::to_value(cluster).unwrap(),
                machines_imported: machines.len() as i32,
            })))
        }
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn preview_import(
    State(state): State<AppState>,
    Json(payload): Json<ImportClusterRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let controller = crate::controllers::cluster::ClusterController::new(state.db_pool.clone());

    match controller.preview_import(payload.kubeconfig).await {
        Ok(discovered) => Ok(Json(serde_json::to_value(discovered).unwrap())),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
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

    let token = create_jwt(&create_claims(
        &authenticated_user.email,
        &authenticated_user.role,
        std::time::Duration::from_secs(3600),
    )).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        user: authenticated_user,
    }))
}

pub async fn logout() -> StatusCode {
    StatusCode::OK
}

pub async fn refresh_token() -> StatusCode {
    StatusCode::OK
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
