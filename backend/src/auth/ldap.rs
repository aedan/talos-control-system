use ldap3::{LdapConnAsync, Scope, SearchEntry};
use sqlx::SqlitePool;
use crate::db::models::auth::User;
use crate::db::repos::user;
use crate::config::auth::LdapConfig;
use crate::AppError;
use tracing::info;
use uuid::Uuid;

pub struct LdapClient {
    config: LdapConfig,
}

impl LdapClient {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    /// Authenticate user via LDAP/AD simple bind
    pub async fn authenticate(
        &self,
        pool: &SqlitePool,
        username: &str,
        password: &str,
    ) -> Result<User, AppError> {
        // Connect to LDAP
        let (mut conn, mut ldap) = LdapConnAsync::new(&self.config.url)
            .await
            .map_err(|e| AppError::Ldap(format!("Failed to connect to LDAP: {}", e)))?;
        
        tokio::pin!(conn);

        // Search for user DN
        let user_filter = self.config.user_search_filter
            .replace("{0}", username);
        
        let rs = ldap.search(
            &self.config.user_search_base,
            Scope::Subtree,
            &user_filter,
            vec!["distinguishedName", "mail", "cn", "memberOf"],
        ).await
        .map_err(|e| AppError::Ldap(format!("User search failed: {}", e)))?;

        let (entries, _) = rs.success()
            .map_err(|e| AppError::Ldap(format!("User search failed: {}", e)))?;

        let entries: Vec<SearchEntry> = entries
            .into_iter()
            .map(|entry| SearchEntry::construct(entry))
            .collect();

        if entries.is_empty() {
            return Err(AppError::Ldap("User not found in LDAP".to_string()));
        }

        let user_entry = &entries[0];
        let user_dn = user_entry.dn.clone();
        
        // Extract email
        let email = user_entry.attrs.get("mail")
            .and_then(|vals| vals.first())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}@ldap", username));

        // Bind as user to verify password
        let bind_result = ldap.simple_bind(&user_dn, password).await
            .map_err(|e| AppError::Ldap(format!("LDAP bind failed: {}", e)))?;
        
        if bind_result.rc != 0 {
            return Err(AppError::Auth("Invalid username or password".to_string()));
        }

        // Get group memberships and resolve role
        let groups: Vec<String> = user_entry.attrs.get("memberOf")
            .cloned()
            .unwrap_or_default();

        let role = self.resolve_role(&groups);

        // Upsert user in local DB
        let user = self.upsert_user(pool, &email, &user_entry, &user_dn, role).await?;

        // Update last login
        user::update_last_login(pool, user.id).await.ok();

        info!(user_id = %user.id, email = %email, dn = %user_dn, "LDAP authentication successful");

        Ok(user)
    }

    fn resolve_role(&self, groups: &[String]) -> String {
        for mapping in &self.config.group_role_mappings {
            for group in groups {
                if self.match_group(group, &mapping.group_dn_pattern) {
                    return mapping.role.clone();
                }
            }
        }
        self.config.default_role.clone()
    }

    fn match_group(&self, group_dn: &str, pattern: &str) -> bool {
        // Simple wildcard matching: "CN=TCS-*" matches "CN=TCS-Admins,OU=..."
        if !pattern.contains('*') {
            return group_dn.contains(pattern);
        }
        
        let parts: Vec<&str> = pattern.splitn(2, '*').collect();
        if parts.len() != 2 {
            return false;
        }
        
        group_dn.starts_with(parts[0])
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

        let display_name = entry.attrs.get("cn")
            .and_then(|vals| vals.first())
            .map(|s| s.to_string())
            .unwrap_or_else(|| email.to_string());

        let now = chrono::Utc::now();

        match existing {
            Some(mut u) => {
                u.display_name = display_name;
                u.role = role;
                u.auth_provider = "ldap".to_string();
                u.ldap_dn = Some(dn.to_string());
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
