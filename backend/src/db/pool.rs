//! Dual-backend pool: SQLite (default) and PostgreSQL runtime.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{FromRow, PgPool, Postgres, Sqlite, SqlitePool};
use uuid::Uuid;

use crate::config::{DatabaseBackend, DatabaseConfig};
use crate::AppError;

/// Shared application database pool.
#[derive(Clone)]
pub enum DbPool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl DbPool {
    pub fn backend(&self) -> DatabaseBackend {
        match self {
            DbPool::Sqlite(_) => DatabaseBackend::Sqlite,
            DbPool::Postgres(_) => DatabaseBackend::Postgres,
        }
    }

    pub fn is_postgres(&self) -> bool {
        matches!(self, DbPool::Postgres(_))
    }

    /// Rewrite `?` placeholders to `$1..$n` for Postgres. Leaves quoted strings alone (best-effort).
    pub fn adapt_sql(&self, sql: &str) -> String {
        if !self.is_postgres() {
            return sql.to_string();
        }
        rewrite_placeholders(sql)
    }

    pub async fn execute(&self, sql: &str, vals: &[SqlVal]) -> Result<u64, AppError> {
        let sql = self.adapt_sql(sql);
        match self {
            DbPool::Sqlite(p) => {
                let mut q = sqlx::query(&sql);
                for v in vals {
                    q = v.bind_sqlite(q);
                }
                Ok(q.execute(p).await?.rows_affected())
            }
            DbPool::Postgres(p) => {
                let mut q = sqlx::query(&sql);
                for v in vals {
                    q = v.bind_pg(q);
                }
                Ok(q.execute(p).await?.rows_affected())
            }
        }
    }

    pub async fn fetch_optional_as<T>(&self, sql: &str, vals: &[SqlVal]) -> Result<Option<T>, AppError>
    where
        T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow>
            + for<'r> FromRow<'r, sqlx::postgres::PgRow>
            + Send
            + Unpin,
    {
        let sql = self.adapt_sql(sql);
        match self {
            DbPool::Sqlite(p) => {
                let mut q = sqlx::query_as::<_, T>(&sql);
                for v in vals {
                    q = v.bind_sqlite_as(q);
                }
                Ok(q.fetch_optional(p).await?)
            }
            DbPool::Postgres(p) => {
                let mut q = sqlx::query_as::<_, T>(&sql);
                for v in vals {
                    q = v.bind_pg_as(q);
                }
                Ok(q.fetch_optional(p).await?)
            }
        }
    }

    pub async fn fetch_all_as<T>(&self, sql: &str, vals: &[SqlVal]) -> Result<Vec<T>, AppError>
    where
        T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow>
            + for<'r> FromRow<'r, sqlx::postgres::PgRow>
            + Send
            + Unpin,
    {
        let sql = self.adapt_sql(sql);
        match self {
            DbPool::Sqlite(p) => {
                let mut q = sqlx::query_as::<_, T>(&sql);
                for v in vals {
                    q = v.bind_sqlite_as(q);
                }
                Ok(q.fetch_all(p).await?)
            }
            DbPool::Postgres(p) => {
                let mut q = sqlx::query_as::<_, T>(&sql);
                for v in vals {
                    q = v.bind_pg_as(q);
                }
                Ok(q.fetch_all(p).await?)
            }
        }
    }

    pub async fn fetch_one_as<T>(&self, sql: &str, vals: &[SqlVal]) -> Result<T, AppError>
    where
        T: for<'r> FromRow<'r, sqlx::sqlite::SqliteRow>
            + for<'r> FromRow<'r, sqlx::postgres::PgRow>
            + Send
            + Unpin,
    {
        self.fetch_optional_as(sql, vals)
            .await?
            .ok_or_else(|| AppError::NotFound("row not found".into()))
    }

    pub async fn fetch_scalar_i64(&self, sql: &str, vals: &[SqlVal]) -> Result<i64, AppError> {
        let sql = self.adapt_sql(sql);
        match self {
            DbPool::Sqlite(p) => {
                let mut q = sqlx::query_as::<_, (i64,)>(&sql);
                for v in vals {
                    q = v.bind_sqlite_as(q);
                }
                Ok(q.fetch_one(p).await?.0)
            }
            DbPool::Postgres(p) => {
                // COUNT(*) may come back as i64
                let mut q = sqlx::query_as::<_, (i64,)>(&sql);
                for v in vals {
                    q = v.bind_pg_as(q);
                }
                Ok(q.fetch_one(p).await?.0)
            }
        }
    }

    /// Run multi-statement SQL (migrations). Splits on `;` carefully enough for our DDL.
    pub async fn execute_script(&self, script: &str) -> Result<(), AppError> {
        for stmt in split_sql_statements(script) {
            if stmt.is_empty() {
                continue;
            }
            // DDL rarely uses placeholders; still adapt for safety.
            self.execute(&stmt, &[]).await?;
        }
        Ok(())
    }
}

/// Bound SQL value for dual-backend queries.
#[derive(Debug, Clone)]
pub enum SqlVal {
    Bool(bool),
    I32(i32),
    I64(i64),
    Text(String),
    Uuid(Uuid),
    DateTime(DateTime<Utc>),
    OptText(Option<String>),
    OptI32(Option<i32>),
    OptI64(Option<i64>),
    OptUuid(Option<Uuid>),
    OptDateTime(Option<DateTime<Utc>>),
    OptBool(Option<bool>),
    Bytes(Vec<u8>),
    OptBytes(Option<Vec<u8>>),
}

impl SqlVal {
    pub fn text(s: impl Into<String>) -> Self {
        SqlVal::Text(s.into())
    }
    pub fn opt_text(s: Option<impl Into<String>>) -> Self {
        SqlVal::OptText(s.map(|x| x.into()))
    }

    fn bind_sqlite<'q>(
        &self,
        q: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    ) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        match self {
            SqlVal::Bool(v) => q.bind(*v),
            SqlVal::I32(v) => q.bind(*v),
            SqlVal::I64(v) => q.bind(*v),
            SqlVal::Text(v) => q.bind(v.clone()),
            SqlVal::Uuid(v) => q.bind(*v),
            SqlVal::DateTime(v) => q.bind(*v),
            SqlVal::OptText(v) => q.bind(v.clone()),
            SqlVal::OptI32(v) => q.bind(*v),
            SqlVal::OptI64(v) => q.bind(*v),
            SqlVal::OptUuid(v) => q.bind(*v),
            SqlVal::OptDateTime(v) => q.bind(*v),
            SqlVal::OptBool(v) => q.bind(*v),
            SqlVal::Bytes(v) => q.bind(v.clone()),
            SqlVal::OptBytes(v) => q.bind(v.clone()),
        }
    }

    fn bind_pg<'q>(
        &self,
        q: sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::Query<'q, Postgres, sqlx::postgres::PgArguments> {
        match self {
            SqlVal::Bool(v) => q.bind(*v),
            SqlVal::I32(v) => q.bind(*v),
            SqlVal::I64(v) => q.bind(*v),
            SqlVal::Text(v) => q.bind(v.clone()),
            SqlVal::Uuid(v) => q.bind(*v),
            SqlVal::DateTime(v) => q.bind(*v),
            SqlVal::OptText(v) => q.bind(v.clone()),
            SqlVal::OptI32(v) => q.bind(*v),
            SqlVal::OptI64(v) => q.bind(*v),
            SqlVal::OptUuid(v) => q.bind(*v),
            SqlVal::OptDateTime(v) => q.bind(*v),
            SqlVal::OptBool(v) => q.bind(*v),
            SqlVal::Bytes(v) => q.bind(v.clone()),
            SqlVal::OptBytes(v) => q.bind(v.clone()),
        }
    }

    fn bind_sqlite_as<'q, T>(
        &self,
        q: sqlx::query::QueryAs<'q, Sqlite, T, sqlx::sqlite::SqliteArguments<'q>>,
    ) -> sqlx::query::QueryAs<'q, Sqlite, T, sqlx::sqlite::SqliteArguments<'q>> {
        match self {
            SqlVal::Bool(v) => q.bind(*v),
            SqlVal::I32(v) => q.bind(*v),
            SqlVal::I64(v) => q.bind(*v),
            SqlVal::Text(v) => q.bind(v.clone()),
            SqlVal::Uuid(v) => q.bind(*v),
            SqlVal::DateTime(v) => q.bind(*v),
            SqlVal::OptText(v) => q.bind(v.clone()),
            SqlVal::OptI32(v) => q.bind(*v),
            SqlVal::OptI64(v) => q.bind(*v),
            SqlVal::OptUuid(v) => q.bind(*v),
            SqlVal::OptDateTime(v) => q.bind(*v),
            SqlVal::OptBool(v) => q.bind(*v),
            SqlVal::Bytes(v) => q.bind(v.clone()),
            SqlVal::OptBytes(v) => q.bind(v.clone()),
        }
    }

    fn bind_pg_as<'q, T>(
        &self,
        q: sqlx::query::QueryAs<'q, Postgres, T, sqlx::postgres::PgArguments>,
    ) -> sqlx::query::QueryAs<'q, Postgres, T, sqlx::postgres::PgArguments> {
        match self {
            SqlVal::Bool(v) => q.bind(*v),
            SqlVal::I32(v) => q.bind(*v),
            SqlVal::I64(v) => q.bind(*v),
            SqlVal::Text(v) => q.bind(v.clone()),
            SqlVal::Uuid(v) => q.bind(*v),
            SqlVal::DateTime(v) => q.bind(*v),
            SqlVal::OptText(v) => q.bind(v.clone()),
            SqlVal::OptI32(v) => q.bind(*v),
            SqlVal::OptI64(v) => q.bind(*v),
            SqlVal::OptUuid(v) => q.bind(*v),
            SqlVal::OptDateTime(v) => q.bind(*v),
            SqlVal::OptBool(v) => q.bind(*v),
            SqlVal::Bytes(v) => q.bind(v.clone()),
            SqlVal::OptBytes(v) => q.bind(v.clone()),
        }
    }
}

pub async fn connect(config: &DatabaseConfig) -> Result<DbPool, AppError> {
    match config.backend {
        DatabaseBackend::Sqlite => {
            if let Some(parent) = std::path::Path::new(&config.sqlite_path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        AppError::Config(format!(
                            "Cannot create database directory {}: {}",
                            parent.display(),
                            e
                        ))
                    })?;
                }
            }
            let pool = SqlitePool::connect_with(
                SqliteConnectOptions::new()
                    .filename(&config.sqlite_path)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::default())
                    .foreign_keys(true),
            )
            .await?;
            tracing::info!(backend = "sqlite", path = %config.sqlite_path, "Database pool initialized");
            Ok(DbPool::Sqlite(pool))
        }
        DatabaseBackend::Postgres => {
            if config.postgres_url.trim().is_empty() {
                return Err(AppError::Config(
                    "database.backend = \"postgres\" requires database.postgres_url".into(),
                ));
            }
            let pool = PgPoolOptions::new()
                .max_connections(config.max_connections.max(1))
                .acquire_timeout(std::time::Duration::from_secs(config.connection_timeout.max(1)))
                .connect(&config.postgres_url)
                .await
                .map_err(|e| AppError::Config(format!("PostgreSQL connection failed: {}", e)))?;
            tracing::info!(backend = "postgres", "Database pool initialized");
            Ok(DbPool::Postgres(pool))
        }
    }
}

fn rewrite_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len() + 16);
    let mut n = 0u32;
    let mut chars = sql.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(c);
            }
            '?' if !in_single && !in_double => {
                n += 1;
                out.push('$');
                out.push_str(&n.to_string());
            }
            _ => out.push(c),
        }
    }
    // Common SQLite → Postgres tweaks for DML used in app code
    out.replace("datetime('now')", "NOW()")
        .replace("AUTOINCREMENT", "")
}

fn split_sql_statements(script: &str) -> Vec<String> {
    // Our migrations don't embed `;` inside strings in complex ways.
    // Strip full-line `--` comments so a header comment cannot swallow the
    // following CREATE TABLE in the same `;`-separated chunk.
    script
        .split(';')
        .map(|chunk| {
            chunk
                .lines()
                .filter(|l| {
                    let t = l.trim();
                    !t.is_empty() && !t.starts_with("--")
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Translate SQLite DDL fragments for Postgres.
pub fn sqlite_ddl_to_postgres(sql: &str) -> String {
    let mut s = sql.to_string();
    s = s.replace("INTEGER PRIMARY KEY AUTOINCREMENT", "SERIAL PRIMARY KEY");
    s = s.replace("AUTOINCREMENT", "");
    s = s.replace("datetime('now')", "NOW()");
    // SQLite BOOLEAN affinity often stored as INTEGER — keep INTEGER for compatibility
    s
}
