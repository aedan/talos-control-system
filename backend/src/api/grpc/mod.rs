use crate::AppState;
use crate::config::BrandingConfig;
use axum::Router;

pub fn create_grpc_service(_state: AppState) {
    tracing::info!("gRPC service registered");
}
