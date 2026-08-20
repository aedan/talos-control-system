//! Durable audit log backed by the `audit_logs` table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub user_email: String,
    pub action: String,
    pub resource: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFilter {
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
}

fn default_page() -> usize {
    1
}
fn default_per_page() -> usize {
    50
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AuditRow {
    id: String,
    action: String,
    resource_type: String,
    details: String,
    created_at: String,
}

pub async fn log_action(
    pool: &DbPool,
    email: &str,
    action: &str,
    resource: &str,
    details: &str,
) {
    let id = Uuid::new_v4();
    let now = Utc::now();

    tracing::info!(
        audit_action = %action,
        audit_resource = %resource,
        audit_user = %email,
        "Audit"
    );

    // The column is TEXT; store the hyphenated string (not a Uuid BLOB) so
    // reads back as String work on both SQLite and Postgres.
    if let Err(e) = pool
        .execute(
            "INSERT INTO audit_logs (id, user_id, resource_type, resource_id, action, details, created_at)
             VALUES (?, NULL, ?, ?, ?, ?, ?)",
            &[
                SqlVal::text(id.to_string()),
                SqlVal::text(resource),
                SqlVal::text(""),
                SqlVal::text(action),
                SqlVal::text(format!("{} | user={}", details, email)),
                SqlVal::DateTime(now),
            ],
        )
        .await
    {
        tracing::warn!(error = %e, "Failed to write audit log");
    }
}

pub async fn get_entries(
    pool: &DbPool,
    filter: &AuditFilter,
) -> Result<(Vec<AuditEntry>, usize), AppError> {
    let rows: Vec<AuditRow> = pool
        .fetch_all_as(
            "SELECT id, action, resource_type, COALESCE(details, '') as details, created_at
             FROM audit_logs
             ORDER BY created_at DESC
             LIMIT 5000",
            &[],
        )
        .await?;

    let mut entries: Vec<AuditEntry> = Vec::new();
    for row in rows {
        let user_email = row
            .details
            .split(" | user=")
            .nth(1)
            .unwrap_or("system")
            .to_string();
        let clean_details = row
            .details
            .split(" | user=")
            .next()
            .unwrap_or(&row.details)
            .to_string();
        let timestamp = DateTime::parse_from_rfc3339(&row.created_at)
            .map(|d| d.with_timezone(&Utc))
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&row.created_at, "%Y-%m-%d %H:%M:%S")
                    .map(|n| DateTime::from_naive_utc_and_offset(n, Utc))
            })
            .unwrap_or_else(|_| Utc::now());

        let uuid = Uuid::parse_str(&row.id).unwrap_or_else(|_| Uuid::new_v4());
        entries.push(AuditEntry {
            id: uuid,
            timestamp,
            user_email,
            action: row.action,
            resource: row.resource_type,
            details: clean_details,
        });
    }

    let filtered: Vec<AuditEntry> = entries
        .into_iter()
        .filter(|entry| {
            if !filter.user.is_empty() && entry.user_email != filter.user {
                return false;
            }
            if !filter.action.is_empty() && entry.action != filter.action {
                return false;
            }
            if !filter.from.is_empty() {
                if let Ok(from) = DateTime::parse_from_rfc3339(&filter.from) {
                    if entry.timestamp < from.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            if !filter.to.is_empty() {
                if let Ok(to) = DateTime::parse_from_rfc3339(&filter.to) {
                    if entry.timestamp > to.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            true
        })
        .collect();

    let total = filtered.len();
    let start = (filter.page.saturating_sub(1)) * filter.per_page;
    let end = start.saturating_add(filter.per_page).min(filtered.len());
    let page_entries = if start < filtered.len() {
        filtered[start..end].to_vec()
    } else {
        vec![]
    };

    Ok((page_entries, total))
}

pub async fn clear_all(pool: &DbPool) -> Result<(), AppError> {
    pool.execute("DELETE FROM audit_logs", &[]).await?;
    Ok(())
}

pub async fn count(pool: &DbPool) -> Result<usize, AppError> {
    let c = pool
        .fetch_scalar_i64("SELECT COUNT(*) FROM audit_logs", &[])
        .await?;
    Ok(c as usize)
}
