pub mod config;
pub mod db;
pub mod api;
pub mod controllers;
pub mod runtime;
pub mod network;
pub mod auth;
pub mod branding;
pub mod integration;
pub mod utils;
pub mod cert;

use std::sync::Arc;
use crate::branding::manager::BrandingManager;
use crate::runtime::event::EventBus;
use crate::runtime::cache::AppCache;

pub struct AppState {
    pub config: Arc<crate::config::Config>,
    pub db_pool: sqlx::SqlitePool,
    pub branding: Arc<BrandingManager>,
    pub event_bus: Arc<EventBus>,
    pub cache: AppCache,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            config: Arc::clone(&self.config),
            db_pool: self.db_pool.clone(),
            branding: Arc::clone(&self.branding),
            event_bus: Arc::clone(&self.event_bus),
            cache: self.cache.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("gRPC error: {0}")]
    Grpc(String),

    #[error("LDAP error: {0}")]
    Ldap(String),

    #[error("Network error: {0}")]
    Network(String),
}

pub type Result<T> = std::result::Result<T, AppError>;
