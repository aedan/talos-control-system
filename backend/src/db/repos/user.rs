use uuid::Uuid;

use crate::db::models::auth::User;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

pub async fn get_by_email(pool: &DbPool, email: &str) -> Result<Option<User>, AppError> {
    pool.fetch_optional_as("SELECT * FROM users WHERE email = ?", &[SqlVal::text(email)])
        .await
}

pub async fn get_by_id(pool: &DbPool, id: Uuid) -> Result<Option<User>, AppError> {
    pool.fetch_optional_as("SELECT * FROM users WHERE id = ?", &[SqlVal::Uuid(id)])
        .await
}

pub async fn upsert(pool: &DbPool, user: &User) -> Result<User, AppError> {
    let existing = get_by_email(pool, &user.email).await?;

    if existing.is_some() {
        pool.execute(
            "UPDATE users SET
                display_name = ?,
                role = ?,
                is_active = ?,
                password_hash = ?,
                auth_provider = ?,
                ldap_dn = ?,
                updated_at = ?
            WHERE id = ?",
            &[
                SqlVal::text(&user.display_name),
                SqlVal::text(&user.role),
                SqlVal::Bool(user.is_active),
                SqlVal::OptText(user.password_hash.clone()),
                SqlVal::text(&user.auth_provider),
                SqlVal::OptText(user.ldap_dn.clone()),
                SqlVal::DateTime(user.updated_at),
                SqlVal::Uuid(user.id),
            ],
        )
        .await?;
        get_by_id(pool, user.id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("User {} not found after update", user.id)))
    } else {
        pool.execute(
            "INSERT INTO users (id, email, display_name, role, is_active, password_hash, auth_provider, ldap_dn, password_needs_change, last_login, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                SqlVal::Uuid(user.id),
                SqlVal::text(&user.email),
                SqlVal::text(&user.display_name),
                SqlVal::text(&user.role),
                SqlVal::Bool(user.is_active),
                SqlVal::OptText(user.password_hash.clone()),
                SqlVal::text(&user.auth_provider),
                SqlVal::OptText(user.ldap_dn.clone()),
                SqlVal::Bool(user.password_needs_change),
                SqlVal::OptDateTime(user.last_login),
                SqlVal::DateTime(user.created_at),
                SqlVal::DateTime(user.updated_at),
            ],
        )
        .await?;
        Ok(user.clone())
    }
}

pub async fn list(pool: &DbPool) -> Result<Vec<User>, AppError> {
    pool.fetch_all_as("SELECT * FROM users ORDER BY created_at DESC", &[])
        .await
}

pub async fn deactivate(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE users SET is_active = 0, updated_at = ? WHERE id = ?",
            &[SqlVal::DateTime(chrono::Utc::now()), SqlVal::Uuid(id)],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("User {} not found", id)));
    }
    Ok(())
}

pub async fn update_last_login(pool: &DbPool, id: Uuid) -> Result<(), AppError> {
    let now = chrono::Utc::now();
    pool.execute(
        "UPDATE users SET last_login = ?, updated_at = ? WHERE id = ?",
        &[
            SqlVal::DateTime(now),
            SqlVal::DateTime(now),
            SqlVal::Uuid(id),
        ],
    )
    .await?;
    Ok(())
}

pub async fn update_password(pool: &DbPool, id: Uuid, new_hash: &str) -> Result<(), AppError> {
    let n = pool
        .execute(
            "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(new_hash),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Auth(format!("User {} not found", id)));
    }
    Ok(())
}
