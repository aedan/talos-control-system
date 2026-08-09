use std::collections::VecDeque;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

const MAX_ENTRIES: usize = 10000;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub timestamp: DateTime<Utc>,
    pub user_email: String,
    pub action: String,
    pub resource: String,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

fn default_page() -> usize { 1 }
fn default_per_page() -> usize { 50 }

static AUDIT_LOG: LazyLock<RwLock<VecDeque<AuditEntry>>> = LazyLock::new(|| {
    RwLock::new(VecDeque::with_capacity(MAX_ENTRIES))
});

pub fn log_action(email: &str, action: &str, resource: &str, details: &str) {
    let entry = AuditEntry {
        timestamp: Utc::now(),
        user_email: email.to_string(),
        action: action.to_string(),
        resource: resource.to_string(),
        details: details.to_string(),
    };

    tracing::info!(
        audit_action = %action,
        audit_resource = %resource,
        audit_user = %email,
        "Audit"
    );

    tokio::spawn(async move {
        let mut guard = AUDIT_LOG.write().await;
        if guard.len() >= MAX_ENTRIES {
            guard.pop_front();
        }
        guard.push_back(entry);
    });
}

pub async fn get_entries(filter: &AuditFilter) -> (Vec<AuditEntry>, usize) {
    let log = AUDIT_LOG.read().await;
    let total = log.len();

    let filtered: Vec<AuditEntry> = log
        .iter()
        .rev()
        .filter(|entry| {
            if !filter.user.is_empty() && entry.user_email != filter.user {
                return false;
            }
            if !filter.action.is_empty() && entry.action != filter.action {
                return false;
            }
            if !filter.from.is_empty() {
                if let Ok(from) = chrono::DateTime::parse_from_rfc3339(&filter.from) {
                    if entry.timestamp < from.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            if !filter.to.is_empty() {
                if let Ok(to) = chrono::DateTime::parse_from_rfc3339(&filter.to) {
                    if entry.timestamp > to.with_timezone(&Utc) {
                        return false;
                    }
                }
            }
            true
        })
        .cloned()
        .collect();

    let start = (filter.page.saturating_sub(1)) * filter.per_page;
    let end = start.saturating_add(filter.per_page).min(filtered.len());
    let page_entries = if start < filtered.len() {
        filtered[start..end].to_vec()
    } else {
        vec![]
    };

    (page_entries, total)
}

pub async fn clear_all() {
    let mut log = AUDIT_LOG.write().await;
    log.clear();
}

pub async fn count() -> usize {
    let log = AUDIT_LOG.read().await;
    log.len()
}
