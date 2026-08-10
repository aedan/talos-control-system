# PostgreSQL

**Status:** dual-backend **runtime** supported (alpha). SQLite remains the default.

## Configure

```toml
[database]
backend = "postgres"
postgres_url = "postgresql://tcs:secret@db.example:5432/tcs"
max_connections = 10
```

TCS connects with `sqlx` `PgPool`, rewrites `?` placeholders to `$1..$n`, and applies migrations (SQLite DDL is translated; a consolidated schema is used as fallback).

## Multi-replica notes

- Point every replica at the **same** Postgres URL.
- Set unique `TCS_INSTANCE_ID` per process (optional; auto-generated UUID otherwise).
- Backup and upgrade schedulers use **DB locks** (`ha_locks`) so only one replica runs work.
- OIDC CSRF `state` is stored in `oidc_states` (shared across replicas).

## SQLite (default)

```toml
[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"
```

## Migration

There is no automatic SQLite → Postgres data migrator yet. Greenfield Postgres installs apply schema on first boot. For production cutover, export/import is a follow-up.
