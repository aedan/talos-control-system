//! Database vault for TCS secrets.
//!
//! Ciphertext lives in `vault_secrets`. The 32-byte data key is not stored.
//! `vault_meta` holds two AES-GCM wraps of that key:
//!
//! * `host_wrap`, opened by `/var/lib/tcs/vault.hostkey` (mode 0600) at startup
//! * `break_glass_wrap`, opened by a passphrase that is shown once
//!
//! The passphrase itself is stored only as an Argon2id verifier, with a
//! separate Argon2id salt for the wrap key. The verifier hash is not the wrap
//! key. There is no HTTP route and no `vault get` command that prints a secret.
//!
//! The first data key is the historical JWT-derived key from
//! [`super::secrets::key_from_jwt_secret`], so talosconfig, kubeconfig, and BMC
//! blobs written before the vault still decrypt.

use std::path::{Path, PathBuf};

use argon2::Argon2;
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use sqlx::FromRow;

use crate::auth::local::{hash_password, verify_password};
use crate::config::Config;
use crate::db::{DbPool, SqlVal};
use crate::utils::secrets;
use crate::AppError;

pub const JWT_SECRET: &str = "auth.jwt_secret";
pub const OIDC_CLIENT_SECRET: &str = "auth.oidc.client_secret";
pub const LDAP_BIND_PASSWORD: &str = "auth.ldap.bind_password";
pub const DNS_API_KEY: &str = "tls.dns.api_key";
pub const DNS_API_SECRET: &str = "tls.dns.api_secret";
pub const DNS_API_TOKEN: &str = "tls.dns.api_token";

/// Built-in lab secret. Never written into the vault, so a later real secret
/// can be adopted without sealing the data key to this placeholder.
pub const DEFAULT_JWT_SECRET: &str = "talos-control-system-default-secret-change-in-production";

const ALLOWED: &[&str] = &[
    JWT_SECRET,
    OIDC_CLIENT_SECRET,
    LDAP_BIND_PASSWORD,
    DNS_API_KEY,
    DNS_API_SECRET,
    DNS_API_TOKEN,
];

/// Open vault. `break_glass` is `Some` only on the call that created it.
pub struct Opened {
    pub vault: Vault,
    pub break_glass: Option<String>,
}

pub struct Vault {
    dek: [u8; 32],
}

#[derive(FromRow)]
struct MetaRow {
    host_wrap: String,
    break_glass_wrap: String,
    break_glass_salt: String,
    break_glass_hash: String,
}

#[derive(FromRow)]
struct SecretRow {
    ciphertext: String,
}

impl Vault {
    #[cfg(test)]
    pub(crate) fn dek(&self) -> &[u8; 32] {
        &self.dek
    }

    /// Make later `secrets::encrypt` / `decrypt` calls use this data key.
    pub fn install_process_key(&self) {
        secrets::set_vault_key(self.dek);
    }

    pub async fn get(&self, pool: &DbPool, name: &str) -> Result<Option<String>, AppError> {
        ensure_known(name)?;
        let row: Option<SecretRow> = pool
            .fetch_optional_as(
                "SELECT ciphertext FROM vault_secrets WHERE name = ?",
                &[SqlVal::text(name)],
            )
            .await?;
        match row {
            None => Ok(None),
            Some(row) => {
                let plain = secrets::decrypt_with_key(&self.dek, &row.ciphertext)?;
                if plain.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(plain))
                }
            }
        }
    }

    pub async fn put(&self, pool: &DbPool, name: &str, plaintext: &str) -> Result<(), AppError> {
        ensure_known(name)?;
        if plaintext.is_empty() {
            return Err(AppError::InvalidInput(
                "refusing to store an empty secret".into(),
            ));
        }
        let ciphertext = secrets::encrypt_with_key(&self.dek, plaintext)?;
        pool.execute(
            "INSERT INTO vault_secrets (name, ciphertext, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET ciphertext = excluded.ciphertext, updated_at = excluded.updated_at",
            &[
                SqlVal::text(name),
                SqlVal::text(ciphertext),
                SqlVal::text(timestamp()),
            ],
        )
        .await?;
        Ok(())
    }
}

/// Host-key path. Override with `TCS_VAULT_HOST_KEY`. Otherwise the key sits
/// next to the SQLite file (`vault.hostkey`), including when the database
/// itself is Postgres.
pub fn host_key_path(config: &Config) -> PathBuf {
    if let Ok(p) = std::env::var("TCS_VAULT_HOST_KEY") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let sqlite = Path::new(&config.database.sqlite_path);
    match sqlite.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("vault.hostkey"),
        _ => PathBuf::from("/var/lib/tcs/vault.hostkey"),
    }
}

pub async fn open(pool: &DbPool, host_key_path: &Path, seed_jwt: &str) -> Result<Opened, AppError> {
    let existing: Option<MetaRow> = pool
        .fetch_optional_as(
            "SELECT host_wrap, break_glass_wrap, break_glass_salt, break_glass_hash
             FROM vault_meta WHERE id = 1",
            &[],
        )
        .await?;

    let Some(meta) = existing else {
        return create(pool, host_key_path, seed_jwt).await;
    };

    let host_key = read_host_key(host_key_path)?;
    let dek = unwrap_dek(&host_key, &meta.host_wrap).map_err(|_| {
        AppError::Config(
            "Vault host key cannot unlock the database. The key file was not replaced. \
             Run `tcs vault recover` with the break-glass passphrase."
                .into(),
        )
    })?;
    Ok(Opened {
        vault: Vault { dek },
        break_glass: None,
    })
}

async fn create(pool: &DbPool, host_key_path: &Path, seed_jwt: &str) -> Result<Opened, AppError> {
    let mut rng = rand::thread_rng();
    let mut host_key = [0u8; 32];
    rng.fill_bytes(&mut host_key);
    // Seed from the JWT-derived key so ciphertext already in the database
    // (talosconfig, kubeconfig, BMC) still opens. A random key would strand it.
    let dek = secrets::key_from_jwt_secret(seed_jwt);
    let mut salt = [0u8; 16];
    rng.fill_bytes(&mut salt);
    let passphrase = generate_passphrase();
    let break_glass_hash = hash_password(&passphrase)?;
    let wrap_key = derive_break_glass_key(&passphrase, &salt)?;
    let host_wrap = wrap_dek(&host_key, &dek)?;
    let break_glass_wrap = wrap_dek(&wrap_key, &dek)?;
    let salt_b64 = base64::engine::general_purpose::STANDARD.encode(salt);

    pool.execute(
        "INSERT INTO vault_meta
            (id, host_wrap, break_glass_wrap, break_glass_salt, break_glass_hash, created_at)
         VALUES (1, ?, ?, ?, ?, ?)",
        &[
            SqlVal::text(&host_wrap),
            SqlVal::text(&break_glass_wrap),
            SqlVal::text(salt_b64),
            SqlVal::text(&break_glass_hash),
            SqlVal::text(timestamp()),
        ],
    )
    .await?;

    let vault = Vault { dek };
    if !seed_jwt.is_empty() && seed_jwt != DEFAULT_JWT_SECRET {
        if let Err(e) = vault.put(pool, JWT_SECRET, seed_jwt).await {
            let _ = pool
                .execute("DELETE FROM vault_meta WHERE id = 1", &[])
                .await;
            return Err(e);
        }
    }

    if let Err(e) = write_host_key(host_key_path, &host_key) {
        let _ = pool.execute("DELETE FROM vault_secrets", &[]).await;
        let _ = pool
            .execute("DELETE FROM vault_meta WHERE id = 1", &[])
            .await;
        return Err(e);
    }

    tracing::warn!(
        path = %host_key_path.display(),
        "Vault created. The break-glass passphrase was written to stderr once."
    );

    Ok(Opened {
        vault,
        break_glass: Some(passphrase),
    })
}

/// Replace the host key after checking the break-glass passphrase.
/// Does not print or return any stored secret.
pub async fn recover(
    pool: &DbPool,
    host_key_path: &Path,
    passphrase: &str,
) -> Result<(), AppError> {
    let meta: MetaRow = pool
        .fetch_optional_as(
            "SELECT host_wrap, break_glass_wrap, break_glass_salt, break_glass_hash
             FROM vault_meta WHERE id = 1",
            &[],
        )
        .await?
        .ok_or_else(|| {
            AppError::Config("Vault has not been created yet. Start `tcs serve` once first.".into())
        })?;

    if !verify_password(passphrase, &meta.break_glass_hash)? {
        return Err(AppError::Auth(
            "break-glass passphrase does not match".into(),
        ));
    }
    let salt = base64::engine::general_purpose::STANDARD
        .decode(meta.break_glass_salt.as_bytes())
        .map_err(|e| AppError::Internal(format!("Vault salt is corrupt: {e}")))?;
    let wrap_key = derive_break_glass_key(passphrase, &salt)?;
    let dek = unwrap_dek(&wrap_key, &meta.break_glass_wrap)
        .map_err(|_| AppError::Internal("Break-glass wrap could not be opened.".into()))?;

    let mut host_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut host_key);
    let host_wrap = wrap_dek(&host_key, &dek)?;
    pool.execute(
        "UPDATE vault_meta SET host_wrap = ? WHERE id = 1",
        &[SqlVal::text(&host_wrap)],
    )
    .await?;
    if let Err(e) = write_host_key(host_key_path, &host_key) {
        let _ = pool
            .execute(
                "UPDATE vault_meta SET host_wrap = ? WHERE id = 1",
                &[SqlVal::text(meta.host_wrap)],
            )
            .await;
        return Err(e);
    }
    Ok(())
}

/// Copy configured secrets into the vault, then fill `config` from the vault.
/// The vault value wins when both are set. Nothing is logged except the name.
pub async fn apply_config(
    vault: &Vault,
    pool: &DbPool,
    config: &mut Config,
) -> Result<(), AppError> {
    overlay_jwt(vault, pool, &mut config.auth.jwt_secret).await?;
    if let Some(oidc) = config.auth.oidc.as_mut() {
        overlay(vault, pool, OIDC_CLIENT_SECRET, &mut oidc.client_secret).await?;
    }
    if let Some(ldap) = config.auth.ldap.as_mut() {
        overlay(vault, pool, LDAP_BIND_PASSWORD, &mut ldap.bind_password).await?;
    }
    if let Some(le) = config.tls.letsencrypt.as_mut() {
        if let Some(dns) = le.dns_provider.as_mut() {
            overlay(vault, pool, DNS_API_KEY, &mut dns.api_key).await?;
            overlay(vault, pool, DNS_API_SECRET, &mut dns.api_secret).await?;
            overlay(vault, pool, DNS_API_TOKEN, &mut dns.api_token).await?;
        }
    }
    Ok(())
}

async fn overlay_jwt(vault: &Vault, pool: &DbPool, current: &mut String) -> Result<(), AppError> {
    if let Some(stored) = vault.get(pool, JWT_SECRET).await? {
        if !current.is_empty() && current != DEFAULT_JWT_SECRET {
            note_cleartext(JWT_SECRET, false);
        }
        *current = stored;
        return Ok(());
    }
    if !current.is_empty() && current != DEFAULT_JWT_SECRET {
        vault.put(pool, JWT_SECRET, current).await?;
        note_cleartext(JWT_SECRET, true);
    }
    Ok(())
}

async fn overlay(
    vault: &Vault,
    pool: &DbPool,
    name: &str,
    current: &mut String,
) -> Result<(), AppError> {
    if let Some(stored) = vault.get(pool, name).await? {
        if !current.is_empty() {
            note_cleartext(name, false);
        }
        *current = stored;
        return Ok(());
    }
    if !current.is_empty() {
        vault.put(pool, name, current).await?;
        note_cleartext(name, true);
    }
    Ok(())
}

fn note_cleartext(name: &str, adopted: bool) {
    if adopted {
        tracing::warn!(
            secret = name,
            "Copied this secret into the vault. Remove the cleartext copy from config and the environment."
        );
    } else {
        tracing::warn!(
            secret = name,
            "Cleartext copy is still configured. The vault value is the one in use."
        );
    }
}

/// Remove secret assignments from config.toml and `/etc/tcs/env` once the vault
/// holds them. Missing files are skipped. The current process keeps any value
/// systemd already put in the environment until the next restart.
pub async fn redact_cleartext_files(
    vault: &Vault,
    pool: &DbPool,
    config_path: Option<&Path>,
    env_path: Option<&Path>,
) -> Result<(), AppError> {
    let mut config_keys: Vec<&str> = Vec::new();
    let mut env_keys: Vec<&str> = Vec::new();
    for (name, config_key, env_key) in [
        (JWT_SECRET, "jwt_secret", "TCS_AUTH_JWT_SECRET"),
        (
            OIDC_CLIENT_SECRET,
            "client_secret",
            "TCS_AUTH_OIDC_CLIENT_SECRET",
        ),
        (
            LDAP_BIND_PASSWORD,
            "bind_password",
            "TCS_AUTH_LDAP_BIND_PASSWORD",
        ),
        (
            DNS_API_KEY,
            "api_key",
            "TCS_TLS_LETSENCRYPT_DNS_PROVIDER_API_KEY",
        ),
        (
            DNS_API_SECRET,
            "api_secret",
            "TCS_TLS_LETSENCRYPT_DNS_PROVIDER_API_SECRET",
        ),
        (
            DNS_API_TOKEN,
            "api_token",
            "TCS_TLS_LETSENCRYPT_DNS_PROVIDER_API_TOKEN",
        ),
    ] {
        if vault
            .get(pool, name)
            .await?
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            config_keys.push(config_key);
            env_keys.push(env_key);
        }
    }
    if let Some(path) = config_path {
        redact_file(path, &config_keys)?;
    }
    if let Some(path) = env_path {
        redact_file(path, &env_keys)?;
    }
    Ok(())
}

fn redact_file(path: &Path, keys: &[&str]) -> Result<(), AppError> {
    if keys.is_empty() || !path.is_file() {
        return Ok(());
    }
    let original = std::fs::read_to_string(path)?;
    let (updated, changed) = redact_assignment_lines(&original, keys);
    if !changed {
        return Ok(());
    }
    rewrite_preserving_mode(path, &updated)?;
    tracing::warn!(
        path = %path.display(),
        "Removed cleartext secrets from this file. A value already loaded into this process stays until restart."
    );
    Ok(())
}

/// Comment out assignment lines for `keys`. Comment lines and other keys stay.
pub(crate) fn redact_assignment_lines(text: &str, keys: &[&str]) -> (String, bool) {
    let mut changed = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        if keys.iter().any(|key| line_assigns(body, key)) {
            changed = true;
            let key = keys
                .iter()
                .find(|key| line_assigns(body, key))
                .copied()
                .unwrap_or("secret");
            out.push_str("# ");
            out.push_str(key);
            out.push_str(" removed: stored in the TCS vault");
            out.push_str(nl);
        } else {
            out.push_str(line);
        }
    }
    (out, changed)
}

fn line_assigns(line: &str, key: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return false;
    }
    let trimmed = trimmed
        .strip_prefix("export ")
        .unwrap_or(trimmed)
        .trim_start();
    let Some(rest) = trimmed.strip_prefix(key) else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with('=')
}

fn rewrite_preserving_mode(path: &Path, body: &str) -> Result<(), AppError> {
    let mode = std::fs::metadata(path)?.permissions();
    let tmp = path.with_extension("redact-tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::set_permissions(&tmp, mode)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn known_secret_names() -> &'static [&'static str] {
    ALLOWED
}

fn ensure_known(name: &str) -> Result<(), AppError> {
    if ALLOWED.contains(&name) {
        Ok(())
    } else {
        Err(AppError::InvalidInput(format!(
            "unknown vault secret `{name}`. Known names: {}",
            ALLOWED.join(", ")
        )))
    }
}

fn generate_passphrase() -> String {
    const ALPH: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    let mut raw = [0u8; 24];
    for byte in &mut raw {
        let idx = (rng.next_u32() as usize) % ALPH.len();
        *byte = ALPH[idx];
    }
    let flat = std::str::from_utf8(&raw).expect("ascii passphrase");
    flat.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).expect("ascii"))
        .collect::<Vec<_>>()
        .join("-")
}

fn derive_break_glass_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], AppError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Internal(format!("break-glass key derivation failed: {e}")))?;
    Ok(key)
}

fn encode_dek(dek: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(dek)
}

fn decode_dek(stored: &str) -> Result<[u8; 32], AppError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(stored)
        .map_err(|e| AppError::Internal(format!("Vault key decode failed: {e}")))?;
    if bytes.len() != 32 {
        return Err(AppError::Internal("Vault key has the wrong length".into()));
    }
    let mut dek = [0u8; 32];
    dek.copy_from_slice(&bytes);
    Ok(dek)
}

fn wrap_dek(key: &[u8; 32], dek: &[u8; 32]) -> Result<String, AppError> {
    secrets::encrypt_with_key(key, &encode_dek(dek))
}

fn unwrap_dek(key: &[u8; 32], stored: &str) -> Result<[u8; 32], AppError> {
    decode_dek(&secrets::decrypt_with_key(key, stored)?)
}

fn read_host_key(path: &Path) -> Result<[u8; 32], AppError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::Config(format!(
                "Vault host key is missing at {}. Run `tcs vault recover` with the break-glass passphrase.",
                path.display()
            )));
        }
        Err(e) => return Err(AppError::Io(e)),
    };
    if bytes.len() != 32 {
        return Err(AppError::Config(
            "Vault host key file is corrupt. Run `tcs vault recover`.".into(),
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn write_host_key(path: &Path, key: &[u8; 32]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = path.with_extension("hostkey.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(key)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn timestamp() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::auth::{LdapConfig, OidcConfig};
    use crate::config::tls::{DnsProviderConfig, LetsEncryptConfig, TlsConfig, TlsMode};
    use crate::config::DatabaseConfig;
    use crate::db::{init_pool, run_migrations};

    #[test]
    fn redacts_assignments_and_keeps_comments() {
        let src = "\
# jwt_secret = \"keep-comment\"\n\
jwt_secret = \"real\"\n\
client_secret = \"oidc\"\n\
other = \"stay\"\n\
export TCS_AUTH_JWT_SECRET=from-env\n";
        let (out, changed) =
            redact_assignment_lines(src, &["jwt_secret", "client_secret", "TCS_AUTH_JWT_SECRET"]);
        assert!(changed);
        assert!(out.contains("# jwt_secret = \"keep-comment\""));
        assert!(out.contains("other = \"stay\""));
        assert!(!out.contains("real"));
        assert!(!out.contains("oidc"));
        assert!(!out.contains("from-env"));
        assert!(out.contains("jwt_secret removed: stored in the TCS vault"));
        assert!(out.contains("TCS_AUTH_JWT_SECRET removed: stored in the TCS vault"));
    }

    #[tokio::test]
    async fn vault_seals_existing_key_and_recovers() {
        let dir = std::env::temp_dir().join(format!(
            "tcs-vault-{}-{}",
            std::process::id(),
            Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("data.db");
        let key_path = dir.join("vault.hostkey");
        let mut db = DatabaseConfig::default();
        db.sqlite_path = db_path.display().to_string();
        let pool = init_pool(&db).await.unwrap();
        run_migrations(&pool).await.unwrap();

        let jwt = "ord-bootstrap-jwt-not-default";
        let opened = open(&pool, &key_path, jwt).await.unwrap();
        let phrase = opened.break_glass.clone().expect("shown once");
        assert_eq!(phrase.chars().filter(|c| *c == '-').count(), 5);
        assert_eq!(opened.vault.dek(), &secrets::key_from_jwt_secret(jwt));

        let blob = secrets::encrypt(jwt, "talosconfig-body").unwrap();
        assert_eq!(
            secrets::decrypt_with_key(opened.vault.dek(), &blob).unwrap(),
            "talosconfig-body"
        );

        opened
            .vault
            .put(&pool, OIDC_CLIENT_SECRET, "oidc-value")
            .await
            .unwrap();
        assert!(opened.vault.put(&pool, "not.a.secret", "x").await.is_err());

        let again = open(&pool, &key_path, "ignored").await.unwrap();
        assert!(again.break_glass.is_none());
        assert_eq!(
            again.vault.get(&pool, JWT_SECRET).await.unwrap().unwrap(),
            jwt
        );
        assert_eq!(
            again
                .vault
                .get(&pool, OIDC_CLIENT_SECRET)
                .await
                .unwrap()
                .unwrap(),
            "oidc-value"
        );

        std::fs::remove_file(&key_path).unwrap();
        let missing = match open(&pool, &key_path, jwt).await {
            Err(e) => e,
            Ok(_) => panic!("missing host key must not open the vault"),
        };
        assert!(missing.to_string().contains("vault recover"));
        assert!(!key_path.exists());

        let bad = match recover(&pool, &key_path, "not-the-phrase").await {
            Err(e) => e,
            Ok(()) => panic!("wrong passphrase must not restore the host key"),
        };
        assert!(bad.to_string().contains("does not match"));
        assert!(!key_path.exists());

        recover(&pool, &key_path, &phrase).await.unwrap();
        let restored = open(&pool, &key_path, jwt).await.unwrap();
        assert_eq!(
            restored
                .vault
                .get(&pool, OIDC_CLIENT_SECRET)
                .await
                .unwrap()
                .unwrap(),
            "oidc-value"
        );

        let mut config = Config::default();
        config.auth.jwt_secret = jwt.to_string();
        config.auth.oidc = Some(OidcConfig {
            enabled: true,
            client_secret: String::new(),
            ..OidcConfig::default()
        });
        config.auth.ldap = Some(LdapConfig {
            bind_password: "ldap-bind".into(),
            ..LdapConfig::default()
        });
        config.tls = TlsConfig {
            enabled: true,
            mode: TlsMode::LetsEncrypt,
            letsencrypt: Some(LetsEncryptConfig {
                domains: vec!["tcs.example.com".into()],
                email: "a@example.com".into(),
                challenge_type: crate::config::tls::ChallengeType::Dns01,
                dns_provider: Some(DnsProviderConfig {
                    provider: "godaddy".into(),
                    api_key: "godaddy-key".into(),
                    api_secret: "godaddy-secret".into(),
                    api_token: String::new(),
                    zone_id: String::new(),
                    dns_zone: "example.com".into(),
                }),
            }),
            self_signed: None,
            provided: None,
        };
        apply_config(&restored.vault, &pool, &mut config)
            .await
            .unwrap();
        assert_eq!(config.auth.jwt_secret, jwt);
        assert_eq!(
            config.auth.oidc.as_ref().unwrap().client_secret,
            "oidc-value"
        );
        assert_eq!(
            config.auth.ldap.as_ref().unwrap().bind_password,
            "ldap-bind"
        );
        let dns = config
            .tls
            .letsencrypt
            .as_ref()
            .unwrap()
            .dns_provider
            .as_ref()
            .unwrap();
        assert_eq!(dns.api_key, "godaddy-key");
        assert_eq!(dns.api_secret, "godaddy-secret");

        let cfg_path = dir.join("config.toml");
        let env_path = dir.join("env");
        std::fs::write(
            &cfg_path,
            "jwt_secret = \"ord-bootstrap-jwt-not-default\"\nclient_secret = \"file-copy\"\nbind_addr = \"0.0.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            &env_path,
            "TCS_AUTH_JWT_SECRET=ord-bootstrap-jwt-not-default\nTCS_HTTP_PORT=0\n",
        )
        .unwrap();
        redact_cleartext_files(&restored.vault, &pool, Some(&cfg_path), Some(&env_path))
            .await
            .unwrap();
        let cfg = std::fs::read_to_string(&cfg_path).unwrap();
        let env = std::fs::read_to_string(&env_path).unwrap();
        assert!(!cfg.contains("ord-bootstrap"));
        assert!(!cfg.contains("file-copy"));
        assert!(cfg.contains("bind_addr = \"0.0.0.0\""));
        assert!(!env.contains("ord-bootstrap"));
        assert!(env.contains("TCS_HTTP_PORT=0"));

        // A second open still serves the vault copy after the files were cleared.
        config.auth.jwt_secret = DEFAULT_JWT_SECRET.to_string();
        config.auth.oidc.as_mut().unwrap().client_secret.clear();
        apply_config(&restored.vault, &pool, &mut config)
            .await
            .unwrap();
        assert_eq!(config.auth.jwt_secret, jwt);
        assert_eq!(
            config.auth.oidc.as_ref().unwrap().client_secret,
            "oidc-value"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
