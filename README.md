# Talos Control System

**Status: Alpha** — see [docs/STATUS.md](docs/STATUS.md) for an honest feature matrix.

**Talos Control System (TCS)** is a self-hosted web UI for managing [Talos Linux](https://www.talos.dev/) clusters. Import existing clusters, inventory machines, apply config patches, take etcd snapshots, and run limited machine actions (version, reboot, upgrade) over the Talos gRPC API.

## What works today

- Local auth (Argon2) + JWT, basic RBAC (admin / operator / reader)
- OIDC and LDAP (implemented; validate in your environment)
- White-label branding
- Cluster **import** via kubeconfig (+ optional talosconfig)
- Inventory CRUD for clusters and machines
- Config patches stored and **applied** via Talos `ApplyConfiguration`
- **Real etcd snapshots** (download, retention, disaster-recovery restore)
- Machine version probe, reboot, upgrade

## What does **not** work yet

- Greenfield cluster create / scale / destroy (UI “create” is inventory-only)
- Siderolink machine discovery
- SAML
- Postgres (SQLite only)
- Docker image / Compose (binary-first distribution)
- Multi-tenant branding

## Quick start

### Local development

```bash
# Backend (serves API + embedded UI after frontend build)
cd frontend && npm install && npm run build && cd ..
cd backend && cargo run
# Open http://localhost:8081
```

Default admin is created on first boot (`admin@tcs.local`). Password is random
unless `TCS_DEFAULT_ADMIN_PASSWORD` is set — check process logs.

**Required for production:**

```bash
export TCS_AUTH_JWT_SECRET="$(openssl rand -hex 32)"
# or set TCS_ALLOW_INSECURE=1 only for local lab use
```

### Binary install

See [docs/INSTALL.md](docs/INSTALL.md). Release assets are built by GitHub Actions
(musl where configured).

### Helm

Chart sources live under `deploy/helm` and are **experimental**. There is no
public chart repo at `charts.talos.dev`.

## Architecture

```
SvelteKit UI  →  Axum REST (/api)  →  SQLite
                      │
         outbound gRPC+mTLS → Talos nodes :50000
         outbound HTTPS     → Kubernetes API (import/refresh)
```

## Project layout

```
talos-control-system/
├── backend/          # Rust (Axum + Talos gRPC client)
├── frontend/         # SvelteKit + Tailwind (embedded in binary)
├── deploy/helm/      # Experimental Helm chart
├── docs/             # Guides + STATUS.md
├── config.example.toml
└── LICENSE           # Apache-2.0
```

## Documentation

- [Status / feature matrix](docs/STATUS.md)
- [Installation](docs/INSTALL.md)
- [Configuration](docs/CONFIGURATION.md)
- [Talos API control](docs/TALOS.md)
- [Authentication](docs/AUTH.md)
- [TLS](docs/TLS.md)
- [Branding](docs/BRANDING.md)
- [Deployment](docs/DEPLOYMENT.md)
- [Development](docs/DEVELOPMENT.md)
- [Lab smoke checklist](docs/SMOKE.md)

## License

Apache-2.0 — see [LICENSE](LICENSE).
