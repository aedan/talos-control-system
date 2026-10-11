use std::time::Duration;

use serde::{Deserialize, Serialize};
use crate::db::pool::DbPool;
use uuid::Uuid;

use crate::auth::jwt::{Claims, create_claims, create_jwt, verify_jwt};
use crate::config::auth::OidcConfig as ConfigOidcConfig;
use crate::db::models::auth::User;
use crate::db::repos::user;
use crate::AppError;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcUserInfo {
    pub subject: String,
    pub email: String,
    pub display_name: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
}

fn empty_oidc_user() -> OidcUserInfo {
    OidcUserInfo {
        subject: String::new(),
        email: String::new(),
        display_name: String::new(),
        groups: Vec::new(),
        roles: Vec::new(),
    }
}

fn claim_strings(value: &serde_json::Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn user_from_claims(claims: &serde_json::Value) -> OidcUserInfo {
    let mut info = empty_oidc_user();
    info.subject = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let email = claims
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    info.email = if email.is_empty() {
        claims
            .get("preferred_username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        email.to_string()
    };
    info.display_name = claims
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    info.groups = claim_strings(claims, "groups");
    info.roles = claim_strings(claims, "roles");
    info
}

fn known_role(role: &str) -> Option<String> {
    match role.trim().to_ascii_lowercase().as_str() {
        "admin" => Some("admin".to_string()),
        "operator" => Some("operator".to_string()),
        "reader" => Some("reader".to_string()),
        _ => None,
    }
}

fn role_rank(role: &str) -> u8 {
    match role {
        "admin" => 3,
        "operator" => 2,
        "reader" => 1,
        _ => 0,
    }
}

/// Role from an email row, an Entra app role, or a group mapping.
/// `None` means nothing was assigned, so a new user becomes reader and an
/// existing user's stored role is left alone.
fn resolve_assigned_role(
    config: &ConfigOidcConfig,
    email: &str,
    groups: &[String],
    roles: &[String],
) -> Option<String> {
    let email_key = email.trim().to_ascii_lowercase();
    if !email_key.is_empty() {
        if let Some(mapping) = config.user_role_mappings.iter().find(|m| {
            m.email.trim().to_ascii_lowercase() == email_key
        }) {
            if let Some(role) = known_role(&mapping.role) {
                return Some(role);
            }
        }
    }

    let mut best = 0u8;
    let mut chosen = None;
    for role in roles {
        let tail = role.rsplit(['.', ':']).next().unwrap_or(role);
        if let Some(name) = known_role(tail) {
            let rank = role_rank(&name);
            if rank > best {
                best = rank;
                chosen = Some(name);
            }
        }
    }
    for mapping in &config.group_role_mappings {
        let pattern = mapping.group_pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let Some(name) = known_role(&mapping.role) else {
            continue;
        };
        let rank = role_rank(&name);
        if rank <= best {
            continue;
        }
        let pat = pattern.to_ascii_lowercase();
        if groups.iter().any(|g| g.to_ascii_lowercase().contains(&pat)) {
            best = rank;
            chosen = Some(name);
        }
    }
    chosen
}

pub struct TcsOidcProvider {
    config: ConfigOidcConfig,
    http: reqwest::Client,
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
    jwks_uri: String,
}

impl TcsOidcProvider {
    pub async fn new(config: ConfigOidcConfig) -> Result<Self, AppError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Config(format!("Failed to build HTTP client for OIDC: {}", e)))?;

        // Discover provider metadata from .well-known/openid-configuration
        let issuer = config.issuer_url.trim_end_matches('/');
        let discovery_url = format!("{}/.well-known/openid-configuration", issuer);

        let metadata: serde_json::Value = http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| AppError::Config(format!("OIDC provider discovery failed: {}", e)))?
            .json()
            .await
            .map_err(|e| AppError::Config(format!("Failed to parse OIDC discovery: {}", e)))?;

        let authorize_url = metadata
            .get("authorization_endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Config("No authorization_endpoint in OIDC discovery".to_string()))?
            .to_string();

        let token_url = metadata
            .get("token_endpoint")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Config("No token_endpoint in OIDC discovery".to_string()))?
            .to_string();

        let userinfo_url = metadata
            .get("userinfo_endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let jwks_uri = metadata
            .get("jwks_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Self {
            config,
            http,
            authorize_url,
            token_url,
            userinfo_url,
            jwks_uri,
        })
    }

    pub fn authorize_url(&self, state: &str) -> Result<String, AppError> {
        use url::form_urlencoded::Serializer;

        let scope = self.config.scopes.join(" ");
        let nonce = Uuid::new_v4().to_string();
        let mut ser = Serializer::new(String::new());
        ser.append_pair("response_type", "code");
        ser.append_pair("client_id", &self.config.client_id);
        ser.append_pair("redirect_uri", &self.config.redirect_url);
        ser.append_pair("state", state);
        ser.append_pair("scope", &scope);
        ser.append_pair("nonce", &nonce);
        let params = ser.finish();

        if self.authorize_url.contains('?') {
            Ok(format!("{}&{}", self.authorize_url, params))
        } else {
            Ok(format!("{}?{}", self.authorize_url, params))
        }
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        redirect_url: &str,
    ) -> Result<OidcUserInfo, AppError> {
        // Exchange code for tokens
        let token_response: serde_json::Value = self
            .http
            .post(&self.token_url)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_url),
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
            ])
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("Token exchange request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("Failed to parse token response: {}", e)))?;

        let access_token = token_response
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Auth("No access_token in response".to_string()))?;

        let id_token_str = token_response
            .get("id_token")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Prefer verified ID token claims (JWKS) when available.
        let from_id_token = if !id_token_str.is_empty() {
            match self.verify_id_token(id_token_str).await {
                Ok(info) => Some(info),
                Err(e) => {
                    warn!(error = %e, "ID token JWKS verification failed; trying userinfo/fallback");
                    None
                }
            }
        } else {
            None
        };

        let user_info = if let Some(mut info) = from_id_token {
            let needs_more = info.display_name.is_empty()
                || info.email.is_empty()
                || info.groups.is_empty()
                || info.roles.is_empty();
            if needs_more && !self.userinfo_url.is_empty() {
                if let Ok(ui) = self.fetch_user_info(access_token).await {
                    if info.display_name.is_empty() {
                        info.display_name = ui.display_name;
                    }
                    if info.email.is_empty() {
                        info.email = ui.email;
                    }
                    if info.groups.is_empty() {
                        info.groups = ui.groups;
                    }
                    if info.roles.is_empty() {
                        info.roles = ui.roles;
                    }
                }
            }
            info
        } else if !self.userinfo_url.is_empty() {
            match self.fetch_user_info(access_token).await {
                Ok(info) => info,
                Err(e) => {
                    warn!(error = %e, "Userinfo endpoint failed, falling back to unverified ID token");
                    self.parse_id_token_claims(id_token_str)
                }
            }
        } else {
            self.parse_id_token_claims(id_token_str)
        };

        if user_info.email.is_empty() {
            return Err(AppError::Auth(
                "OIDC did not provide an email claim".to_string(),
            ));
        }

        Ok(user_info)
    }

    /// Verify `id_token` signature against the provider JWKS and extract claims.
    pub async fn verify_id_token(&self, id_token: &str) -> Result<OidcUserInfo, AppError> {
        use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};

        if self.jwks_uri.is_empty() {
            return Err(AppError::Auth("OIDC provider has no jwks_uri".to_string()));
        }

        let header = decode_header(id_token)
            .map_err(|e| AppError::Auth(format!("Invalid ID token header: {}", e)))?;
        let kid = header
            .kid
            .ok_or_else(|| AppError::Auth("ID token missing kid".to_string()))?;

        let jwks: serde_json::Value = self
            .http
            .get(&self.jwks_uri)
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("JWKS fetch failed: {}", e)))?
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("JWKS parse failed: {}", e)))?;

        let keys = jwks
            .get("keys")
            .and_then(|k| k.as_array())
            .ok_or_else(|| AppError::Auth("JWKS has no keys".to_string()))?;

        let jwk = keys
            .iter()
            .find(|k| k.get("kid").and_then(|v| v.as_str()) == Some(kid.as_str()))
            .ok_or_else(|| AppError::Auth(format!("No JWK for kid {}", kid)))?;

        let n = jwk
            .get("n")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Auth("JWK missing n".to_string()))?;
        let e = jwk
            .get("e")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Auth("JWK missing e".to_string()))?;

        let key = DecodingKey::from_rsa_components(n, e)
            .map_err(|err| AppError::Auth(format!("Invalid RSA JWK: {}", err)))?;

        let mut validation = Validation::new(Algorithm::RS256);
        let issuer = self.config.issuer_url.trim_end_matches('/');
        validation.set_issuer(&[issuer, &format!("{}/", issuer)]);
        validation.set_audience(&[&self.config.client_id]);
        validation.validate_aud = true;

        #[derive(Debug, Deserialize)]
        struct IdClaims {
            sub: String,
            #[serde(default)]
            email: String,
            #[serde(default)]
            name: String,
            #[serde(default)]
            preferred_username: String,
            #[serde(default, deserialize_with = "flexible_string_list")]
            groups: Vec<String>,
            #[serde(default, deserialize_with = "flexible_string_list")]
            roles: Vec<String>,
        }

        let token_data = match decode::<IdClaims>(id_token, &key, &validation) {
            Ok(t) => t,
            Err(e) => {
                // Retry without audience if provider uses azp-only.
                validation.validate_aud = false;
                decode::<IdClaims>(id_token, &key, &validation).map_err(|e2| {
                    AppError::Auth(format!(
                        "ID token verification failed: {} (retry: {})",
                        e, e2
                    ))
                })?
            }
        };

        let email = if token_data.claims.email.is_empty() {
            token_data.claims.preferred_username.clone()
        } else {
            token_data.claims.email.clone()
        };

        Ok(OidcUserInfo {
            subject: token_data.claims.sub,
            email,
            display_name: token_data.claims.name,
            groups: token_data.claims.groups,
            roles: token_data.claims.roles,
        })
    }

    async fn fetch_user_info(&self, access_token: &str) -> Result<OidcUserInfo, AppError> {
        let resp: serde_json::Value = self
            .http
            .get(&self.userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| AppError::Auth(format!("Userinfo request failed: {}", e)))?
            .json()
            .await
            .map_err(|e| AppError::Auth(format!("Failed to parse userinfo: {}", e)))?;

        Ok(user_from_claims(&resp))
    }

    fn parse_id_token_claims(&self, id_token_str: &str) -> OidcUserInfo {
        let parts: Vec<&str> = id_token_str.split('.').collect();
        if parts.len() != 3 {
            warn!("Invalid ID token format");
            return empty_oidc_user();
        }

        let payload_bytes = match base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            parts[1],
        ) {
            Ok(b) => b,
            Err(_) => {
                warn!("Failed to decode ID token payload");
                return empty_oidc_user();
            }
        };

        let claims: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
            Ok(v) => v,
            Err(_) => {
                warn!("Failed to parse ID token claims as JSON");
                return empty_oidc_user();
            }
        };

        user_from_claims(&claims)
    }

    pub async fn authenticate_and_issue_jwt(
        &self,
        pool: &DbPool,
        user_info: OidcUserInfo,
    ) -> Result<String, AppError> {
        let email = if user_info.email.is_empty() {
            return Err(AppError::Auth(
                "OIDC provider did not return an email address".to_string(),
            ));
        } else {
            user_info.email.clone()
        };

        let display_name = if user_info.display_name.is_empty() {
            email.split('@').next().unwrap_or(&email).to_string()
        } else {
            user_info.display_name.clone()
        };

        let now = chrono::Utc::now();

        let assigned = resolve_assigned_role(
            &self.config,
            &email,
            &user_info.groups,
            &user_info.roles,
        );
        let user = match user::get_by_email(pool, &email).await? {
            Some(mut existing) => {
                existing.display_name = display_name;
                existing.auth_provider = "oidc".to_string();
                existing.is_active = true;
                existing.updated_at = now;
                if let Some(role) = assigned {
                    existing.role = role;
                }
                user::upsert(pool, &existing).await?
            }
            None => {
                let role = assigned.unwrap_or_else(|| "reader".to_string());
                let new_user = User {
                    id: Uuid::new_v4(),
                    email: email.clone(),
                    display_name,
                    role,
                    is_active: true,
                    password_hash: None,
                    auth_provider: "oidc".to_string(),
                    ldap_dn: None,
                    password_needs_change: false,
                    last_login: Some(now),
                    created_at: now,
                    updated_at: now,
                };
                user::upsert(pool, &new_user).await?
            }
        };

        user::update_last_login(pool, user.id).await.ok();

        let claims = create_claims(&user.email, &user.role, Duration::from_secs(3600));
        let token = create_jwt(&claims)?;

        info!(
            user_id = %user.id,
            email = %user.email,
            auth_provider = "oidc",
            "OIDC authentication successful"
        );

        Ok(token)
    }

    pub async fn validate_token(&self, token: &str) -> Result<Claims, AppError> {
        let token_data = verify_jwt(token)?;
        Ok(token_data.claims)
    }

    pub async fn create_service_token(
        &self,
        subject: &str,
        role: crate::db::models::auth::UserRole,
    ) -> Result<String, AppError> {
        let claims = create_claims(subject, &role.to_string(), Duration::from_secs(86400));
        create_jwt(&claims)
    }

    /// In-memory OIDC `state` parameter store (CSRF protection for auth code flow).
    pub fn remember_state(state: &str) {
        oidc_state_store().insert(state.to_string(), std::time::Instant::now());
        // Opportunistic prune (>10 minutes).
        oidc_state_store().retain(|_, t| t.elapsed() < Duration::from_secs(600));
    }

    pub fn take_state(state: &str) -> bool {
        oidc_state_store().remove(state).is_some()
    }
}

fn flexible_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
            .collect(),
        serde_json::Value::String(s) if !s.is_empty() => vec![s],
        _ => Vec::new(),
    })
}

fn oidc_state_store() -> &'static dashmap::DashMap<String, std::time::Instant> {
    use std::sync::OnceLock;
    static STORE: OnceLock<dashmap::DashMap<String, std::time::Instant>> = OnceLock::new();
    STORE.get_or_init(dashmap::DashMap::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_id_token_claims_roundtrip() {
        // header.payload.sig — payload is base64url JSON
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            br#"{"sub":"u1","email":"u@example.com","name":"User One","groups":["admins"],"roles":["TCS.Operator"]}"#,
        );
        let token = format!("e30.{}.sig", payload);
        let provider = TcsOidcProvider {
            config: ConfigOidcConfig::default(),
            http: reqwest::Client::new(),
            authorize_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            jwks_uri: String::new(),
        };
        let info = provider.parse_id_token_claims(&token);
        assert_eq!(info.subject, "u1");
        assert_eq!(info.email, "u@example.com");
        assert_eq!(info.display_name, "User One");
        assert_eq!(info.groups, vec!["admins".to_string()]);
        assert_eq!(info.roles, vec!["TCS.Operator".to_string()]);
    }

    #[test]
    fn unassigned_login_has_no_role_and_email_beats_group() {
        use crate::config::auth::{OidcGroupMapping, OidcUserMapping};

        let mut config = ConfigOidcConfig::default();
        assert!(resolve_assigned_role(&config, "new@example.com", &[], &[]).is_none());

        config.group_role_mappings.push(OidcGroupMapping {
            group_pattern: "readers".into(),
            role: "reader".into(),
        });
        config.group_role_mappings.push(OidcGroupMapping {
            group_pattern: "admins".into(),
            role: "admin".into(),
        });
        assert_eq!(
            resolve_assigned_role(&config, "a@example.com", &["admins".into(), "readers".into()], &[]).as_deref(),
            Some("admin")
        );
        assert_eq!(
            resolve_assigned_role(&config, "a@example.com", &[], &["TCS.Operator".into()]).as_deref(),
            Some("operator")
        );

        config.user_role_mappings.push(OidcUserMapping {
            email: "A@Example.com".into(),
            role: "reader".into(),
        });
        assert_eq!(
            resolve_assigned_role(&config, "a@example.com", &["admins".into()], &["admin".into()]).as_deref(),
            Some("reader")
        );
    }

    #[test]
    fn state_store_single_use() {
        TcsOidcProvider::remember_state("abc");
        assert!(TcsOidcProvider::take_state("abc"));
        assert!(!TcsOidcProvider::take_state("abc"));
    }
}
