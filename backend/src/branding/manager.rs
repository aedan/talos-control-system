use std::collections::HashMap;
use std::sync::Arc;

use crate::config::BrandingConfig;
use crate::db::models::branding::TenantBranding;
use crate::db::repos;
use crate::AppError;

pub struct BrandingManager {
    defaults: BrandingConfig,
    pool: sqlx::SqlitePool,
}

impl BrandingManager {
    pub async fn new(defaults: &BrandingConfig, pool: &sqlx::SqlitePool) -> Result<Self, AppError> {
        let manager = Self {
            defaults: defaults.clone(),
            pool: pool.clone(),
        };

        manager.load_from_db().await?;
        tracing::info!("Branding manager initialized");
        Ok(manager)
    }

    pub fn get_branding(&self, _tenant_id: &str) -> BrandingConfig {
        self.defaults.clone()
    }

    pub async fn update_branding(&self, _tenant_id: &str, branding: &TenantBranding) -> Result<(), AppError> {
        repos::branding::upsert_tenant_branding(&self.pool, branding).await?;
        tracing::info!(tenant_id = %branding.tenant_id, "Branding updated");
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), AppError> {
        self.load_from_db().await
    }

    async fn load_from_db(&self) -> Result<(), AppError> {
        let _ = repos::branding::get_default_branding(&self.pool).await;
        Ok(())
    }

    pub fn get_defaults(&self) -> &BrandingConfig {
        &self.defaults
    }
}
