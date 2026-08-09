use axum::Router;
use axum::middleware::from_fn;
use axum::routing::{delete, get, post, put};
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::static_server;
use crate::AppState;
use crate::config::BrandingConfig;

pub mod handlers;
pub mod middleware;

pub fn create_rest_router(state: AppState, branding: &BrandingConfig) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request());

    let public_routes = Router::new()
        .route("/health", get(handlers::health_check))
        .route("/metrics", get(handlers::get_metrics))
        .route("/auth/login", post(handlers::login))
        .route("/auth/logout", post(handlers::logout))
        .route("/auth/token", post(handlers::refresh_token))
        .route("/auth/oidc", get(handlers::oidc_authorize))
        .route("/auth/oidc/callback", get(handlers::oidc_callback));

    let protected_routes = Router::new()
        .route("/auth/me", get(handlers::get_user_info))
        .route("/auth/password", post(handlers::change_password))
        .route("/users", get(handlers::list_users))
        .route("/users", post(handlers::create_user))
        .route("/users/:id", get(handlers::get_user))
        .route("/users/:id", put(handlers::update_user))
        .route("/users/:id", delete(handlers::delete_user))
        .route("/settings/certificates/status", get(handlers::get_cert_status))
        .route("/settings/certificates/config", put(handlers::update_cert_config))
        .route("/settings/certificates/renew", post(handlers::renew_certificate))
        .route("/settings/auth/config", get(handlers::get_auth_config))
        .route("/settings/auth/config", put(handlers::update_auth_config))
        .route("/branding", get(handlers::get_branding))
        .route("/branding", put(handlers::update_branding))
        .route("/branding/css", get(handlers::get_branding_css))
        .route("/branding/logo", get(handlers::get_logo))
        .route("/branding/favicon", get(handlers::get_favicon))
        .route("/clusters", get(handlers::list_clusters))
        .route("/clusters", post(handlers::create_cluster))
        .route("/clusters/import", post(handlers::import_cluster))
        .route("/clusters/import/preview", post(handlers::preview_import))
        .route("/clusters/:id", get(handlers::get_cluster))
        .route("/clusters/:id", put(handlers::update_cluster))
        .route("/clusters/:id", delete(handlers::delete_cluster))
        .route("/machines", get(handlers::list_machines))
        .route("/machines/:id", get(handlers::get_machine))
        .route("/machines/:id", delete(handlers::delete_machine))
        .layer(from_fn(middleware::rbac_middleware));

    let acme_routes = Router::new()
        .route("/.well-known/acme-challenge/*challenge", get(handlers::health_check));

    let api_routes = Router::new()
        .merge(acme_routes)
        .merge(public_routes)
        .merge(protected_routes);

    Router::new()
        .nest("/api", api_routes)
        .fallback(static_server::serve_static)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
