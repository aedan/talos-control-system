use std::collections::HashMap;
use std::sync::Arc;

use crate::config::BrandingConfig;
use crate::db::models::branding::TenantBranding;
use crate::db::pool::DbPool;
use crate::db::repos;
use crate::AppError;

pub struct BrandingManager {
    defaults: BrandingConfig,
    pool: DbPool,
    cache: Arc<tokio::sync::RwLock<HashMap<String, BrandingConfig>>>,
}

impl BrandingManager {
    pub async fn new(defaults: &BrandingConfig, pool: &DbPool) -> Result<Self, AppError> {
        let manager = Self {
            defaults: defaults.clone(),
            pool: pool.clone(),
            cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        };

        manager.load_from_db().await?;
        tracing::info!("Branding manager initialized");
        Ok(manager)
    }

    pub async fn get_branding(&self, tenant_id: &str) -> BrandingConfig {
        {
            let cache = self.cache.read().await;
            if let Some(b) = cache.get(tenant_id) {
                return b.clone();
            }
        }

        match repos::branding::get_tenant_branding(&self.pool, tenant_id).await {
            Ok(Some(tenant)) => {
                let b = tenant.merge_with_defaults(&self.defaults);
                let mut cache = self.cache.write().await;
                cache.insert(tenant_id.to_string(), b.clone());
                b
            }
            _ => self.defaults.clone(),
        }
    }

    pub async fn update_branding(
        &self,
        tenant_id: &str,
        branding: &TenantBranding,
    ) -> Result<(), AppError> {
        repos::branding::upsert_tenant_branding(&self.pool, branding).await?;
        {
            let mut cache = self.cache.write().await;
            cache.remove(tenant_id);
        }
        tracing::info!(tenant_id = %branding.tenant_id, "Branding updated and cache invalidated");
        Ok(())
    }

    pub async fn reload(&self) -> Result<(), AppError> {
        let mut cache = self.cache.write().await;
        cache.clear();
        self.load_from_db_internal(&mut *cache).await?;
        Ok(())
    }

    async fn load_from_db(&self) -> Result<(), AppError> {
        let mut cache = self.cache.write().await;
        self.load_from_db_internal(&mut *cache).await
    }

    async fn load_from_db_internal(
        &self,
        cache: &mut HashMap<String, BrandingConfig>,
    ) -> Result<(), AppError> {
        // List via direct query helper
        let rows: Vec<TenantBranding> = self
            .pool
            .fetch_all_as("SELECT * FROM tenant_branding", &[])
            .await
            .unwrap_or_default();

        for tenant in rows {
            let branding = tenant.merge_with_defaults(&self.defaults);
            cache.insert(tenant.tenant_id.clone(), branding);
        }

        tracing::info!(count = cache.len(), "Loaded tenant branding from database");
        Ok(())
    }

    pub fn get_defaults(&self) -> &BrandingConfig {
        &self.defaults
    }
}
