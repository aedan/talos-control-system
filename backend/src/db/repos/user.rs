use sqlx::SqlitePool;
use uuid::Uuid;

use crate::AppError;
use crate::db::models::auth::User;

pub async fn get_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE email = ?"
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<User>, AppError> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(user)
}

pub async fn upsert(pool: &SqlitePool, user: &User) -> Result<User, AppError> {
    let existing = get_by_email(pool, &user.email).await?;

    if let Some(existing) = existing {
        sqlx::query(
            "UPDATE users SET
                display_name = ?,
                role = ?,
                is_active = ?,
                password_hash = ?,
                auth_provider = ?,
                ldap_dn = ?,
                updated_at = ?
            WHERE id = ?"
        )
        .bind(&user.display_name)
        .bind(&user.role)
        .bind(user.is_active)
        .bind(&user.password_hash)
        .bind(&user.auth_provider)
        .bind(&user.ldap_dn)
        .bind(&user.updated_at)
        .bind(user.id)
        .execute(pool)
        .await?;

        get_by_id(pool, user.id).await?.ok_or_else(|| {
            AppError::Internal(format!("User {} not found after update", user.id))
        })
    } else {
        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, is_active, password_hash, auth_provider, ldap_dn, last_login, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(user.id)
        .bind(&user.email)
        .bind(&user.display_name)
        .bind(&user.role)
        .bind(user.is_active)
        .bind(&user.password_hash)
        .bind(&user.auth_provider)
        .bind(&user.ldap_dn)
        .bind(&user.last_login)
        .bind(&user.created_at)
        .bind(&user.updated_at)
        .execute(pool)
        .await?;

        Ok(user.clone())
    }
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<User>, AppError> {
    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users ORDER BY created_at DESC"
    )
    .fetch_all(pool)
    .await?;

    Ok(users)
}

pub async fn deactivate(pool: &SqlitePool, id: Uuid) -> Result<(), AppError> {
    let result = sqlx::query(
        "UPDATE users SET is_active = 0, updated_at = ? WHERE id = ?"
    )
    .bind(chrono::Utc::now())
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("User {} not found", id)));
    }

    Ok(())
}

pub async fn update_last_login(pool: &SqlitePool, id: Uuid) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    sqlx::query(
        "UPDATE users SET last_login = ?, updated_at = ? WHERE id = ?"
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn update_password(pool: &SqlitePool, id: Uuid, new_hash: &str) -> Result<(), AppError> {
    let result = sqlx::query(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?"
    )
    .bind(new_hash)
    .bind(chrono::Utc::now())
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Auth(format!("User {} not found", id)));
    }

    Ok(())
}
