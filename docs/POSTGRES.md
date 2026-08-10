# PostgreSQL (single-instance)

**Status:** schema bootstrap only. Application runtime remains **SQLite**.

## What works today

If you set:

```toml
[database]
backend = "postgres"
postgres_url = "postgresql://tcs:secret@db.example:5432/tcs"
```

TCS will:

1. Connect to Postgres
2. Apply a greenfield schema (`postgres_schema_v1`) for core tables (clusters, machines, users, upgrade jobs, siderolink, provision artifacts, branding, audit, …)
3. **Exit** with a clear error asking you to use `backend = "sqlite"` for the running control plane

This lets operators prepare a Postgres instance (roles, network, schema) ahead of a future cutover without half-running the API against an incomplete dual-backend.

## Why runtime is still SQLite

Repositories use SQLite-style `?` placeholders and `SqlitePool` end-to-end. A full dual-backend needs either:

- dialect abstraction (`$1` vs `?`) on every query, or
- a compile-time split (`sqlx` offline / separate modules)

That work is tracked as the next Postgres milestone (HA multi-replica TCS).

## Recommended production (alpha)

```toml
[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"
```

Back up the SQLite file with your host backup tooling (see [OPS](OPS.md)).

## Future cutover sketch

1. Finish `DbPool` enum + placeholder rewrite (or `sqlx` dual modules)
2. Point `AppState.db_pool` at Postgres
3. Optional one-shot migrator: SQLite → Postgres export/import
4. Enable multi-replica only after JWT/OIDC state and schedulers are externalized
