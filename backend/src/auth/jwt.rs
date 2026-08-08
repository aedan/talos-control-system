use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Serialize, Deserialize};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::AppError;

static JWT_SECRET: OnceLock<String> = OnceLock::new();

pub fn set_jwt_secret(secret: &str) {
    JWT_SECRET.set(secret.to_string()).ok();
}

fn jwt_secret() -> &'static str {
    JWT_SECRET.get().map(|s| s.as_str()).unwrap_or("talos-control-system-default-secret-change-in-production")
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: i64,
    pub iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Vec<String>>,
}

pub fn create_jwt(claims: &Claims) -> Result<String, AppError> {
    let key = EncodingKey::from_secret(jwt_secret().as_bytes());

    encode(&Header::default(), claims, &key)
        .map_err(|e| AppError::Auth(format!("Failed to create JWT: {}", e)))
}

pub fn verify_jwt(token: &str) -> Result<jsonwebtoken::TokenData<Claims>, AppError> {
    let key = DecodingKey::from_secret(jwt_secret().as_bytes());

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.leeway = 30;

    decode::<Claims>(token, &key, &validation)
        .map_err(|e| AppError::Auth(format!("Failed to verify JWT: {}", e)))
}

pub fn create_claims(
    subject: &str,
    role: &str,
    ttl: Duration,
) -> Claims {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    Claims {
        sub: subject.to_string(),
        role: role.to_string(),
        exp: now + ttl.as_secs() as i64,
        iat: now,
        cluster_scopes: None,
        permissions: None,
    }
}
