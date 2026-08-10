use crate::db::models::branding::TenantBranding;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn get_tenant_branding(
    pool: &DbPool,
    tenant_id: &str,
) -> Result<Option<TenantBranding>, AppError> {
    pool.fetch_optional_as(
        "SELECT * FROM tenant_branding WHERE tenant_id = ?",
        &[SqlVal::text(tenant_id)],
    )
    .await
}

pub async fn get_default_branding(pool: &DbPool) -> Result<Option<TenantBranding>, AppError> {
    get_tenant_branding(pool, "default").await
}

pub async fn upsert_tenant_branding(
    pool: &DbPool,
    branding: &TenantBranding,
) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    let existing = get_tenant_branding(pool, &branding.tenant_id).await?;

    if existing.is_some() {
        pool.execute(
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
            WHERE tenant_id = ?",
            &[
                SqlVal::OptText(branding.name.clone()),
                SqlVal::OptText(branding.short_name.clone()),
                SqlVal::OptText(branding.tagline.clone()),
                SqlVal::OptText(branding.primary_color.clone()),
                SqlVal::OptText(branding.secondary_color.clone()),
                SqlVal::OptText(branding.background_color.clone()),
                SqlVal::OptText(branding.surface_color.clone()),
                SqlVal::OptText(branding.text_color.clone()),
                SqlVal::OptText(branding.text_muted_color.clone()),
                SqlVal::OptText(branding.font_family.clone()),
                SqlVal::OptBytes(branding.logo_data.clone()),
                SqlVal::OptBytes(branding.favicon_data.clone()),
                SqlVal::OptText(branding.docs_url.clone()),
                SqlVal::OptText(branding.support_url.clone()),
                SqlVal::DateTime(now),
                SqlVal::text(&branding.tenant_id),
            ],
        )
        .await?;
    } else {
        pool.execute(
            "INSERT INTO tenant_branding (id, tenant_id, name, short_name, tagline, primary_color, secondary_color,
                background_color, surface_color, text_color, text_muted_color, font_family,
                logo_data, favicon_data, docs_url, support_url, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(branding.id),
                SqlVal::text(&branding.tenant_id),
                SqlVal::OptText(branding.name.clone()),
                SqlVal::OptText(branding.short_name.clone()),
                SqlVal::OptText(branding.tagline.clone()),
                SqlVal::OptText(branding.primary_color.clone()),
                SqlVal::OptText(branding.secondary_color.clone()),
                SqlVal::OptText(branding.background_color.clone()),
                SqlVal::OptText(branding.surface_color.clone()),
                SqlVal::OptText(branding.text_color.clone()),
                SqlVal::OptText(branding.text_muted_color.clone()),
                SqlVal::OptText(branding.font_family.clone()),
                SqlVal::OptBytes(branding.logo_data.clone()),
                SqlVal::OptBytes(branding.favicon_data.clone()),
                SqlVal::OptText(branding.docs_url.clone()),
                SqlVal::OptText(branding.support_url.clone()),
                SqlVal::DateTime(now),
                SqlVal::DateTime(now),
            ],
        )
        .await?;
    }
    tracing::info!(tenant_id = %branding.tenant_id, "Tenant branding upserted");
    Ok(())
}
