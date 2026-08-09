use ldap3::{LdapConnAsync, Scope, SearchEntry};
use sqlx::SqlitePool;
use tracing::info;
use uuid::Uuid;

use crate::config::auth::LdapConfig;
use crate::db::models::auth::User;
use crate::db::repos::user;
use crate::AppError;

pub struct LdapClient {
    config: LdapConfig,
}

impl LdapClient {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    /// Authenticate via LDAP/AD: optional service bind → search user DN → user bind.
    ///
    /// `login` may be a bare username or an email; `{0}` in `user_search_filter`
    /// is replaced with the username form (local-part when email-shaped).
    pub async fn authenticate(
        &self,
        pool: &SqlitePool,
        login: &str,
        password: &str,
    ) -> Result<User, AppError> {
        if password.is_empty() {
            return Err(AppError::Auth("Password required".to_string()));
        }

        let username = login_username(login);

        let (conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| AppError::Ldap(format!("Failed to connect to LDAP: {}", e)))?;
        ldap3::drive!(conn);

        // Service-account bind for search (typical AD setup).
        if !self.config.bind_dn.is_empty() {
            let bind = ldap
                .simple_bind(&self.config.bind_dn, &self.config.bind_password)
                .await
                .map_err(|e| AppError::Ldap(format!("Service bind failed: {}", e)))?;
            if bind.rc != 0 {
                return Err(AppError::Ldap(format!(
                    "Service bind rejected (rc={})",
                    bind.rc
                )));
            }
        }

        let user_filter = self
            .config
            .user_search_filter
            .replace("{0}", &username)
            .replace("{}", &username);

        let rs = ldap
            .search(
                &self.config.user_search_base,
                Scope::Subtree,
                &user_filter,
                vec!["distinguishedName", "mail", "cn", "memberOf", "uid", "sAMAccountName"],
            )
            .await
            .map_err(|e| AppError::Ldap(format!("User search failed: {}", e)))?;

        let (entries, _) = rs
            .success()
            .map_err(|e| AppError::Ldap(format!("User search failed: {}", e)))?;

        let entries: Vec<SearchEntry> = entries
            .into_iter()
            .map(SearchEntry::construct)
            .collect();

        if entries.is_empty() {
            return Err(AppError::Auth("Invalid username or password".to_string()));
        }

        let user_entry = &entries[0];
        let user_dn = user_entry.dn.clone();

        // Verify credentials with a user bind.
        let bind_result = ldap
            .simple_bind(&user_dn, password)
            .await
            .map_err(|e| AppError::Ldap(format!("User bind failed: {}", e)))?;

        if bind_result.rc != 0 {
            return Err(AppError::Auth("Invalid username or password".to_string()));
        }

        let email = user_entry
            .attrs
            .get("mail")
            .and_then(|vals| vals.first())
            .cloned()
            .unwrap_or_else(|| {
                if login.contains('@') {
                    login.to_string()
                } else {
                    format!("{}@ldap.local", username)
                }
            });

        let groups: Vec<String> = user_entry
            .attrs
            .get("memberOf")
            .cloned()
            .unwrap_or_default();

        let role = self.resolve_role(&groups);
        let user = self.upsert_user(pool, &email, user_entry, &user_dn, role).await?;
        user::update_last_login(pool, user.id).await.ok();

        info!(
            user_id = %user.id,
            email = %email,
            dn = %user_dn,
            "LDAP authentication successful"
        );

        Ok(user)
    }

    pub fn resolve_role(&self, groups: &[String]) -> String {
        for mapping in &self.config.group_role_mappings {
            for group in groups {
                if match_group_dn(group, &mapping.group_dn_pattern) {
                    return mapping.role.clone();
                }
            }
        }
        self.config.default_role.clone()
    }

    async fn upsert_user(
        &self,
        pool: &SqlitePool,
        email: &str,
        entry: &SearchEntry,
        dn: &str,
        role: String,
    ) -> Result<User, AppError> {
        let existing = user::get_by_email(pool, email).await.ok().flatten();

        let display_name = entry
            .attrs
            .get("cn")
            .and_then(|vals| vals.first())
            .cloned()
            .unwrap_or_else(|| email.to_string());

        let now = chrono::Utc::now();

        match existing {
            Some(mut u) => {
                u.display_name = display_name;
                u.role = role;
                u.auth_provider = "ldap".to_string();
                u.ldap_dn = Some(dn.to_string());
                u.is_active = true;
                u.updated_at = now;
                user::upsert(pool, &u).await
            }
            None => {
                let u = User {
                    id: Uuid::new_v4(),
                    email: email.to_string(),
                    display_name,
                    role,
                    auth_provider: "ldap".to_string(),
                    password_hash: None,
                    ldap_dn: Some(dn.to_string()),
                    is_active: true,
                    last_login: None,
                    password_needs_change: false,
                    created_at: now,
                    updated_at: now,
                };
                user::upsert(pool, &u).await
            }
        }
    }
}

/// Login identifier for LDAP filter: use local-part when an email is provided.
pub fn login_username(login: &str) -> String {
    let login = login.trim();
    if let Some((local, _)) = login.split_once('@') {
        if !local.is_empty() {
            return local.to_string();
        }
    }
    login.to_string()
}

/// Match group DN against a pattern. Supports a single `*` wildcard.
pub fn match_group_dn(group_dn: &str, pattern: &str) -> bool {
    let group_dn_l = group_dn.to_ascii_lowercase();
    let pattern_l = pattern.to_ascii_lowercase();

    if !pattern_l.contains('*') {
        return group_dn_l.contains(&pattern_l);
    }

    let parts: Vec<&str> = pattern_l.splitn(2, '*').collect();
    if parts.len() != 2 {
        return false;
    }
    let (prefix, suffix) = (parts[0], parts[1]);
    group_dn_l.starts_with(prefix)
        && group_dn_l.ends_with(suffix)
        && group_dn_l.len() >= prefix.len() + suffix.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::auth::GroupRoleMapping;

    #[test]
    fn username_from_email() {
        assert_eq!(login_username("alice@corp.example"), "alice");
        assert_eq!(login_username("bob"), "bob");
        assert_eq!(login_username("  carol@x  "), "carol");
    }

    #[test]
    fn group_match_exact_and_wildcard() {
        assert!(match_group_dn(
            "CN=TCS-Admins,OU=Groups,DC=example,DC=com",
            "CN=TCS-Admins,OU=Groups,DC=example,DC=com"
        ));
        assert!(match_group_dn(
            "CN=TCS-Admins,OU=Groups,DC=example,DC=com",
            "CN=TCS-*"
        ));
        assert!(match_group_dn(
            "CN=TCS-Admins,OU=Groups,DC=example,DC=com",
            "CN=TCS-*,DC=example,DC=com"
        ));
        assert!(!match_group_dn(
            "CN=Other,OU=Groups,DC=example,DC=com",
            "CN=TCS-*"
        ));
    }

    #[test]
    fn role_resolution_order() {
        let client = LdapClient::new(LdapConfig {
            group_role_mappings: vec![
                GroupRoleMapping {
                    group_dn_pattern: "CN=TCS-Admins*".to_string(),
                    role: "admin".to_string(),
                },
                GroupRoleMapping {
                    group_dn_pattern: "CN=TCS-Ops*".to_string(),
                    role: "operator".to_string(),
                },
            ],
            default_role: "reader".to_string(),
            ..Default::default()
        });
        assert_eq!(
            client.resolve_role(&["CN=TCS-Admins,DC=x".to_string()]),
            "admin"
        );
        assert_eq!(
            client.resolve_role(&["CN=TCS-Ops,DC=x".to_string()]),
            "operator"
        );
        assert_eq!(client.resolve_role(&["CN=Other".to_string()]), "reader");
    }
}
