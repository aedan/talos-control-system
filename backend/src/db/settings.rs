//! Console settings stored in the database.
//!
//! `config.toml` keeps what the process needs before the database is open:
//! the database connection, the bind address, and the SideroLink subnet.
//! Auth, TLS, and metal settings that the UI saves live here. Secrets are
//! not written into these documents. They stay in the vault.

use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::auth::{LdapConfig, OidcConfig, SamlConfig};
use crate::config::tls::TlsConfig;
use crate::config::Config;
use crate::db::{DbPool, SqlVal};
use crate::AppError;

pub const AUTH: &str = "auth";
pub const TLS: &str = "tls";
pub const METAL: &str = "metal";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthSettings {
    #[serde(default)]
    pub ldap: Option<LdapConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub saml: Option<SamlConfig>,
}

#[derive(sqlx::FromRow)]
struct ValueRow {
    value: String,
}

pub async fn get_json<T: DeserializeOwned>(pool: &DbPool, key: &str) -> Result<Option<T>, AppError> {
    let row = pool
        .fetch_optional_as::<ValueRow>("SELECT value FROM settings WHERE key = ?", &[SqlVal::text(key)])
        .await?;
    match row {
        None => Ok(None),
        Some(row) => Ok(Some(serde_json::from_str(&row.value)?)),
    }
}

pub async fn put_json<T: Serialize>(pool: &DbPool, key: &str, value: &T) -> Result<(), AppError> {
    let body = serde_json::to_string(value)?;
    let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    pool.execute(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        &[SqlVal::text(key), SqlVal::text(body), SqlVal::text(now)],
    )
    .await?;
    Ok(())
}

/// Load auth and TLS from the database when a row exists. Otherwise store the
/// current config, with secrets removed, and leave the in-memory secrets in
/// place so the vault can adopt them.
pub async fn adopt_auth_and_tls(pool: &DbPool, config: &mut Config) -> Result<(), AppError> {
    match get_json::<AuthSettings>(pool, AUTH).await? {
        Some(doc) => {
            config.auth.ldap = doc.ldap;
            config.auth.oidc = doc.oidc;
            config.auth.saml = doc.saml;
        }
        None => {
            let mut doc = AuthSettings {
                ldap: config.auth.ldap.clone(),
                oidc: config.auth.oidc.clone(),
                saml: config.auth.saml.clone(),
            };
            strip_auth_secrets(&mut doc);
            put_json(pool, AUTH, &doc).await?;
        }
    }
    match get_json::<TlsConfig>(pool, TLS).await? {
        Some(mut tls) => {
            strip_tls_secrets(&mut tls);
            config.tls = tls;
        }
        None => {
            let mut tls = config.tls.clone();
            strip_tls_secrets(&mut tls);
            put_json(pool, TLS, &tls).await?;
        }
    }
    Ok(())
}

/// Use the database value when present. Otherwise store `current`.
pub async fn adopt_or_replace<T>(pool: &DbPool, key: &str, current: &mut T) -> Result<(), AppError>
where
    T: Serialize + DeserializeOwned,
{
    if let Some(stored) = get_json::<T>(pool, key).await? {
        *current = stored;
    } else {
        put_json(pool, key, current).await?;
    }
    Ok(())
}

pub fn strip_auth_secrets(doc: &mut AuthSettings) {
    if let Some(ldap) = doc.ldap.as_mut() {
        ldap.bind_password.clear();
    }
    if let Some(oidc) = doc.oidc.as_mut() {
        oidc.client_secret.clear();
    }
}

pub fn strip_tls_secrets(tls: &mut TlsConfig) {
    if let Some(le) = tls.letsencrypt.as_mut() {
        if let Some(dns) = le.dns_provider.as_mut() {
            dns.api_key.clear();
            dns.api_secret.clear();
            dns.api_token.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DatabaseConfig;
    use crate::db::{init_pool, run_migrations};

    #[tokio::test]
    async fn auth_document_does_not_keep_secrets() {
        let dir = std::env::temp_dir().join(format!(
            "tcs-settings-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut db = DatabaseConfig::default();
        db.sqlite_path = dir.join("data.db").display().to_string();
        let pool = init_pool(&db).await.unwrap();
        run_migrations(&pool).await.unwrap();

        let mut config = Config::default();
        config.auth.oidc = Some(OidcConfig {
            enabled: true,
            client_id: "app".into(),
            client_secret: "super-secret".into(),
            issuer_url: "https://login.example/v2.0".into(),
            ..OidcConfig::default()
        });
        config.auth.ldap = Some(LdapConfig {
            bind_password: "bind-secret".into(),
            ..LdapConfig::default()
        });
        adopt_auth_and_tls(&pool, &mut config).await.unwrap();
        assert_eq!(
            config.auth.oidc.as_ref().unwrap().client_secret,
            "super-secret",
            "in-memory secret stays so the vault can adopt it"
        );

        let stored = get_json::<AuthSettings>(&pool, AUTH).await.unwrap().unwrap();
        assert_eq!(stored.oidc.unwrap().client_secret, "");
        assert_eq!(stored.ldap.unwrap().bind_password, "");

        config.auth.oidc.as_mut().unwrap().client_secret.clear();
        config.auth.ldap.as_mut().unwrap().bind_password.clear();
        adopt_auth_and_tls(&pool, &mut config).await.unwrap();
        assert_eq!(config.auth.oidc.as_ref().unwrap().client_id, "app");
        assert!(config.auth.oidc.as_ref().unwrap().client_secret.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
