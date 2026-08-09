use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
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

        // Fetch user info from userinfo endpoint
        let user_info = if !self.userinfo_url.is_empty() {
            match self.fetch_user_info(access_token).await {
                Ok(info) => info,
                Err(e) => {
                    warn!(error = %e, "Userinfo endpoint failed, falling back to ID token");
                    self.parse_id_token_claims(id_token_str)
                }
            }
        } else {
            self.parse_id_token_claims(id_token_str)
        };

        Ok(user_info)
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

        Ok(OidcUserInfo {
            subject: resp
                .get("sub")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            email: resp
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            display_name: resp
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    fn parse_id_token_claims(&self, id_token_str: &str) -> OidcUserInfo {
        let parts: Vec<&str> = id_token_str.split('.').collect();
        if parts.len() != 3 {
            warn!("Invalid ID token format");
            return OidcUserInfo {
                subject: String::new(),
                email: String::new(),
                display_name: String::new(),
            };
        }

        let payload_bytes = match base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            parts[1],
        ) {
            Ok(b) => b,
            Err(_) => {
                warn!("Failed to decode ID token payload");
                return OidcUserInfo {
                    subject: String::new(),
                    email: String::new(),
                    display_name: String::new(),
                };
            }
        };

        let claims: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
            Ok(v) => v,
            Err(_) => {
                warn!("Failed to parse ID token claims as JSON");
                return OidcUserInfo {
                    subject: String::new(),
                    email: String::new(),
                    display_name: String::new(),
                };
            }
        };

        OidcUserInfo {
            subject: claims
                .get("sub")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            email: claims
                .get("email")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            display_name: claims
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }
    }

    pub async fn authenticate_and_issue_jwt(
        &self,
        pool: &SqlitePool,
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

        let user = match user::get_by_email(pool, &email).await? {
            Some(mut existing) => {
                existing.display_name = display_name;
                existing.auth_provider = "oidc".to_string();
                existing.is_active = true;
                existing.updated_at = now;
                user::upsert(pool, &existing).await?
            }
            None => {
                let role = if self.config.default_role.trim().is_empty() {
                    "reader".to_string()
                } else {
                    self.config.default_role.clone()
                };
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
            br#"{"sub":"u1","email":"u@example.com","name":"User One"}"#,
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
    }

    #[test]
    fn state_store_single_use() {
        TcsOidcProvider::remember_state("abc");
        assert!(TcsOidcProvider::take_state("abc"));
        assert!(!TcsOidcProvider::take_state("abc"));
    }
}
