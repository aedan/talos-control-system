use uuid::Uuid;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TenantBranding {
    #[sqlx(rename = "id")]
    pub id: Uuid,
    #[sqlx(rename = "tenant_id")]
    pub tenant_id: String,
    #[sqlx(rename = "name")]
    pub name: Option<String>,
    #[sqlx(rename = "short_name")]
    pub short_name: Option<String>,
    #[sqlx(rename = "tagline")]
    pub tagline: Option<String>,
    #[sqlx(rename = "primary_color")]
    pub primary_color: Option<String>,
    #[sqlx(rename = "secondary_color")]
    pub secondary_color: Option<String>,
    #[sqlx(rename = "background_color")]
    pub background_color: Option<String>,
    #[sqlx(rename = "surface_color")]
    pub surface_color: Option<String>,
    #[sqlx(rename = "text_color")]
    pub text_color: Option<String>,
    #[sqlx(rename = "text_muted_color")]
    pub text_muted_color: Option<String>,
    #[sqlx(rename = "font_family")]
    pub font_family: Option<String>,
    #[sqlx(rename = "logo_data")]
    pub logo_data: Option<Vec<u8>>,
    #[sqlx(rename = "favicon_data")]
    pub favicon_data: Option<Vec<u8>>,
    #[sqlx(rename = "docs_url")]
    pub docs_url: Option<String>,
    #[sqlx(rename = "support_url")]
    pub support_url: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[sqlx(rename = "updated_at")]
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Default for TenantBranding {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            tenant_id: "default".to_string(),
            name: None,
            short_name: None,
            tagline: None,
            primary_color: None,
            secondary_color: None,
            background_color: None,
            surface_color: None,
            text_color: None,
            text_muted_color: None,
            font_family: None,
            logo_data: None,
            favicon_data: None,
            docs_url: None,
            support_url: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}

impl TenantBranding {
    pub fn for_tenant(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            ..Default::default()
        }
    }

    pub fn merge_with_defaults(&self, defaults: &crate::config::BrandingConfig) -> crate::config::BrandingConfig {
        use crate::config::BrandingConfig;
        BrandingConfig {
            name: self.name.clone().unwrap_or_else(|| defaults.name.clone()),
            short_name: self.short_name.clone().unwrap_or_else(|| defaults.short_name.clone()),
            tagline: self.tagline.clone().unwrap_or_else(|| defaults.tagline.clone()),
            primary_color: self.primary_color.clone().unwrap_or_else(|| defaults.primary_color.clone()),
            secondary_color: self.secondary_color.clone().unwrap_or_else(|| defaults.secondary_color.clone()),
            background_color: self.background_color.clone().unwrap_or_else(|| defaults.background_color.clone()),
            surface_color: self.surface_color.clone().unwrap_or_else(|| defaults.surface_color.clone()),
            text_color: self.text_color.clone().unwrap_or_else(|| defaults.text_color.clone()),
            text_muted_color: self.text_muted_color.clone().unwrap_or_else(|| defaults.text_muted_color.clone()),
            font_family: self.font_family.clone().unwrap_or_else(|| defaults.font_family.clone()),
            logo_path: defaults.logo_path.clone(),
            favicon_path: defaults.favicon_path.clone(),
            docs_url: self.docs_url.clone().unwrap_or_else(|| defaults.docs_url.clone()),
            support_url: self.support_url.clone().unwrap_or_else(|| defaults.support_url.clone()),
        }
    }
}