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
