use crate::auth::jwt::{Claims, create_jwt, verify_jwt};
use crate::db::models::auth::UserRole;
use crate::AppError;

#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    pub scopes: Vec<String>,
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            issuer: "https://accounts.google.com".to_string(),
            client_id: "".to_string(),
            client_secret: "".to_string(),
            redirect_url: "http://localhost:8081/api/auth/callback".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string(), "profile".to_string()],
        }
    }
}

pub struct TcsOidcProvider {
    config: OidcConfig,
}

impl TcsOidcProvider {
    pub fn new(config: OidcConfig) -> Self {
        Self { config }
    }

    pub async fn authorize_url(&self, state: &str) -> Result<String, AppError> {
        let auth_url = format!(
            "{}authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            self.config.issuer,
            self.config.client_id,
            self.config.redirect_url,
            self.config.scopes.join("+"),
            state
        );

        Ok(auth_url)
    }

    pub async fn exchange_token(&self, _code: &str) -> Result<String, AppError> {
        Err(AppError::Auth("OIDC token exchange not yet implemented".to_string()))
    }

    pub async fn validate_token(&self, token: &str) -> Result<Claims, AppError> {
        let token_data = verify_jwt(token)?;
        Ok(token_data.claims)
    }

    pub async fn create_service_token(&self, subject: &str, role: UserRole) -> Result<String, AppError> {
        let claims = crate::auth::jwt::create_claims(
            subject,
            &role.to_string(),
            std::time::Duration::from_secs(86400),
        );

        create_jwt(&claims)
    }
}
