use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sqlx::SqlitePool;
use crate::db::models::auth::User;
use crate::db::repos::user;
use crate::AppError;
use tracing::info;

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default(); // Argon2id
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed_hash = PasswordHash::new(hash).map_err(|e| {
        AppError::Internal(format!("Failed to parse password hash: {}", e))
    })?;
    let result = Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok();
    Ok(result)
}

pub async fn authenticate_local(
    pool: &SqlitePool,
    email: &str,
    password: &str,
) -> Result<User, AppError> {
    let user = user::get_by_email(pool, email).await?
        .ok_or_else(|| AppError::Auth("Invalid email or password".to_string()))?;

    // Must be a local user
    if user.auth_provider != "local" {
        return Err(AppError::Auth("Invalid email or password".to_string()));
    }

    // Must be active
    if !user.is_active {
        return Err(AppError::Auth("Account is disabled".to_string()));
    }

    // Verify password hash exists and matches
    let stored_hash = user.password_hash.as_ref().ok_or_else(|| {
        AppError::Auth("Invalid email or password".to_string())
    })?;

    if !verify_password(password, stored_hash)? {
        return Err(AppError::Auth("Invalid email or password".to_string()));
    }

    // Update last login
    user::update_last_login(pool, user.id).await.ok();
    info!(user_id = %user.id, email = %email, "Local authentication successful");

    Ok(user)
}

pub async fn change_password(
    pool: &SqlitePool,
    user_id: uuid::Uuid,
    new_password: &str,
) -> Result<(), AppError> {
    let new_hash = hash_password(new_password)?;
    user::update_password(pool, user_id, &new_hash).await?;
    info!(user_id = %user_id, "Password changed successfully");
    Ok(())
}
