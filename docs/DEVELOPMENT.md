# Development Guide

## Requirements

- **Rust** 1.97+ (via rustup)
- **Node.js** 24+ (via nvm or fnm)
- **npm** 10+
- **Docker** (optional, for containerized builds)

## Setup

### 1. Clone the repository

```bash
git clone https://github.com/aedan/talos-control-system.git
cd talos-control-system
```

### 2. Install dependencies

```bash
# Backend
cd backend
cargo build

# Frontend
cd ../frontend
npm install
```

## Running the Backend

### Development mode

```bash
cd backend
cargo run
```

The backend starts on:
- **HTTP (REST + embedded Web UI)**: `localhost:8081` (`server.http_port`)

That is the only listener in HTTP-only mode. With TLS enabled, TCS additionally
binds port **80** for ACME HTTP-01 challenges and redirects to HTTPS on
`http_port`. The `server.grpc_port` (8080) and `server.metrics_port` (9090)
config keys are **reserved** — TCS talks to Talos nodes *outbound* over their
`:50000` gRPC API and does not run its own gRPC/metrics listeners.

### With custom config

```bash
cargo run -- --config ../config.toml
```

Or via environment variables:

```bash
TCS_SERVER_HTTP_PORT=9090 cargo run
```

### With debug logging

```bash
RUST_LOG=debug cargo run
```

Log levels: `trace`, `debug`, `info` (default), `warn`, `error`

### Run tests

```bash
cd backend
cargo test

# With output
cargo test -- --nocapture
```

### Lint and format

```bash
cargo fmt
cargo clippy -- -D warnings
```

## Running the Frontend

### Development mode (with hot reload)

```bash
cd frontend
npm run dev
```

The frontend starts on `http://localhost:5173` with:
- Hot module replacement
- Vite proxy for `/api` (and `/ws`) to the backend at `localhost:8081`

### Build for production

```bash
npm run build
```

Build output goes to `backend/frontend-dist/` for the backend to serve.

### Preview production build

```bash
npm run preview
```

### Type checking

```bash
npm run check
```

### Run tests

```bash
npm test
```

## Running Both (Recommended)

Use two terminal windows or a task runner:

```bash
# Terminal 1: Backend
cd backend && cargo run

# Terminal 2: Frontend
cd frontend && npm run dev
```

The frontend proxies API requests to the backend automatically via Vite's dev server proxy configuration.

## Database

TCS uses SQLite by default for local development. The database file is created at `/var/lib/tcs/data.db` or the path specified in config.

### Reset database

```bash
rm /var/lib/tcs/data.db
cargo run  # Migrations run automatically on startup
```

### PostgreSQL

Set `database.backend = "postgres"` and `postgres_url`. Dual-backend runtime is
supported (alpha) — see [POSTGRES.md](POSTGRES.md).

## Adding a New Page

### 1. Create the route file

```bash
mkdir -p frontend/src/routes/my-feature/
touch frontend/src/routes/my-feature/+page.svelte
```

### 2. Add to the Settings dropdown

There is no sidebar. Top-level pages live in the top bar. To surface a new
settings page, add a link in the Settings dropdown in
`frontend/src/routes/+layout.svelte`:

```svelte
<li><a href="/settings/my-feature">My Feature</a></li>
```

For a standalone top-level page, add a link in the top bar or link to it from
the Dashboard.

### 3. Create API endpoint (if needed)

Add handler in `backend/src/api/rest/handlers.rs` and register the route in `backend/src/api/rest/mod.rs`.

## Container image

No Dockerfile is shipped in alpha (binary-first distribution). See `docs/STATUS.md`.

## Tests

```bash
cd backend && cargo test --lib
cd frontend && npm run check
```

Lab JWT:

```bash
export TCS_ALLOW_INSECURE=1
export TCS_DEFAULT_ADMIN_PASSWORD=admin
```

## Git Hooks (Optional)

Set up pre-commit hooks for formatting:

```bash
mkdir -p .git/hooks

# .git/hooks/pre-commit
#!/bin/sh
cd backend && cargo fmt --check && cargo clippy -- -D warnings
cd ../frontend && npm run check
```

```bash
chmod +x .git/hooks/pre-commit
```

## Project Conventions

### Rust (Backend)

- Use `tracing` for logging, not `println!`
- All error types implement `thiserror::Error`
- API handlers return `Result<Json<T>, StatusCode>`
- Database access goes through repository pattern in `src/db/repos/`
- Async functions use `#[tokio::main]` or `#[tokio::test]`

### Svelte (Frontend)

- Use Svelte 5 runes (`$state`, `$derived`, `$effect`)
- CSS is scoped per-component (Svelte's default)
- Use CSS variables for all colors (`var(--tcs-*)`)
- API calls use the `TcsClient` from `$lib/api/client`
- Stores are in `$lib/stores/`
- Shared components are in `$lib/components/`

### File Naming

- Rust: `snake_case.rs`
- Svelte: `PascalCase.svelte` for components, `+page.svelte` for routes
- CSS: `kebab-case.css`
