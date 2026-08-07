use sqlx::SqlitePool;
use crate::db::models::branding::TenantBranding;
use crate::AppError;

pub async fn get_tenant_branding(pool: &SqlitePool, tenant_id: &str) -> Result<Option<TenantBranding>, AppError> {
    let branding = sqlx::query_as(
        "SELECT * FROM tenant_branding WHERE tenant_id = ?"
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(branding)
}

pub async fn get_default_branding(pool: &SqlitePool) -> Result<Option<TenantBranding>, AppError> {
    get_tenant_branding(pool, "default").await
}

pub async fn upsert_tenant_branding(pool: &SqlitePool, branding: &TenantBranding) -> Result<(), AppError> {
    let now = chrono::Utc::now();

    let existing = get_tenant_branding(pool, &branding.tenant_id).await?;

    if existing.is_some() {
        sqlx::query(
            "UPDATE tenant_branding SET
                name = COALESCE(?, name),
                short_name = COALESCE(?, short_name),
                tagline = COALESCE(?, tagline),
                primary_color = COALESCE(?, primary_color),
                secondary_color = COALESCE(?, secondary_color),
                background_color = COALESCE(?, background_color),
                surface_color = COALESCE(?, surface_color),
                text_color = COALESCE(?, text_color),
                text_muted_color = COALESCE(?, text_muted_color),
                font_family = COALESCE(?, font_family),
                logo_data = COALESCE(?, logo_data),
                favicon_data = COALESCE(?, favicon_data),
                docs_url = COALESCE(?, docs_url),
                support_url = COALESCE(?, support_url),
                updated_at = ?
            WHERE tenant_id = ?"
        )
        .bind(&branding.name)
        .bind(&branding.short_name)
        .bind(&branding.tagline)
        .bind(&branding.primary_color)
        .bind(&branding.secondary_color)
        .bind(&branding.background_color)
        .bind(&branding.surface_color)
        .bind(&branding.text_color)
        .bind(&branding.text_muted_color)
        .bind(&branding.font_family)
        .bind(&branding.logo_data)
        .bind(&branding.favicon_data)
        .bind(&branding.docs_url)
        .bind(&branding.support_url)
        .bind(now)
        .bind(&branding.tenant_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO tenant_branding (id, tenant_id, name, short_name, tagline, primary_color, secondary_color,
                background_color, surface_color, text_color, text_muted_color, font_family,
                logo_data, favicon_data, docs_url, support_url, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(branding.id)
        .bind(&branding.tenant_id)
        .bind(&branding.name)
        .bind(&branding.short_name)
        .bind(&branding.tagline)
        .bind(&branding.primary_color)
        .bind(&branding.secondary_color)
        .bind(&branding.background_color)
        .bind(&branding.surface_color)
        .bind(&branding.text_color)
        .bind(&branding.text_muted_color)
        .bind(&branding.font_family)
        .bind(&branding.logo_data)
        .bind(&branding.favicon_data)
        .bind(&branding.docs_url)
        .bind(&branding.support_url)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    }

    tracing::info!(tenant_id = %branding.tenant_id, "Tenant branding upserted");
    Ok(())
}
