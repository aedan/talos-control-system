use axum::extract::State;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::Json;
use serde::{Serialize, Deserialize};

use crate::AppState;
use crate::db::repos;
use crate::db::models::branding::TenantBranding;

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
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn login(
    Json(payload): Json<LoginRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let token = crate::auth::jwt::create_jwt(&crate::auth::jwt::Claims {
        sub: payload.email,
        role: "reader".to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp(),
        iat: chrono::Utc::now().timestamp(),
        cluster_scopes: None,
        permissions: None,
    }).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "token": token })))
}

pub async fn logout() -> StatusCode {
    StatusCode::OK
}

pub async fn refresh_token() -> StatusCode {
    StatusCode::OK
}
