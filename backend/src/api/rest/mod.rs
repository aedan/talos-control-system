use axum::Router;
use axum::routing::{get, post, put, delete};
use tower_http::cors::{CorsLayer, Any, AllowHeaders};
use tower_http::trace::TraceLayer;

use crate::AppState;
use crate::config::BrandingConfig;

pub mod handlers;

pub fn create_rest_router(state: AppState, branding: &BrandingConfig) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(AllowHeaders::mirror_request());

    let api_routes = Router::new()
        .route("/health", get(handlers::health_check))
        .route("/branding", get(handlers::get_branding))
        .route("/branding", put(handlers::update_branding))
        .route("/clusters", get(handlers::list_clusters))
        .route("/clusters", post(handlers::create_cluster))
        .route("/clusters/:id", get(handlers::get_cluster))
        .route("/clusters/:id", put(handlers::update_cluster))
        .route("/clusters/:id", delete(handlers::delete_cluster))
        .route("/machines", get(handlers::list_machines))
        .route("/machines/:id", get(handlers::get_machine))
        .route("/machines/:id", delete(handlers::delete_machine))
        .route("/metrics", get(handlers::get_metrics))
        .route("/auth/login", post(handlers::login))
        .route("/auth/logout", post(handlers::logout))
        .route("/auth/token", post(handlers::refresh_token))
        .route("/branding/css", get(handlers::get_branding_css))
        .route("/branding/logo", get(handlers::get_logo))
        .route("/branding/favicon", get(handlers::get_favicon));

    Router::new()
        .nest("/api", api_routes)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
