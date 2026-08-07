use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Serialize, Deserialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use crate::AppError;

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
    let secret = get_jwt_secret();
    let key = EncodingKey::from_secret(secret.as_bytes());

    encode(&Header::default(), claims, &key)
        .map_err(|e| AppError::Auth(format!("Failed to create JWT: {}", e)))
}

pub fn verify_jwt(token: &str) -> Result<jsonwebtoken::TokenData<Claims>, AppError> {
    let secret = get_jwt_secret();
    let key = DecodingKey::from_secret(secret.as_bytes());

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

fn get_jwt_secret() -> String {
    std::env::var("TCS_JWT_SECRET")
        .unwrap_or_else(|_| "talos-control-system-default-secret-change-in-production".to_string())
}
