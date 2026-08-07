# Talos Control System

**Talos Control System (TCS)** is a web-based management platform for [Talos Linux](https://www.talos.dev/) clusters. Deploy, configure, monitor, and backup your Kubernetes infrastructure from a single white-labelable interface.

## Features

- **Cluster Lifecycle Management** — Create, scale, and destroy Talos clusters from the UI
- **Machine Discovery** — Automatic machine registration via siderolink tunnels
- **Config Patches** — Apply Talos configuration overrides per cluster or per machine
- **Backup & Restore** — Etcd snapshots with download and retention policies
- **White-Label Branding** — Full color, logo, and identity customization via UI or config
- **Per-Tenant Branding** — Serve different branding to different tenants
- **RBAC** — Role-based access control with OIDC and SAML support
- **Audit Logging** — Full trail of user actions and system events
- **Minimal Image** — Production builds on `scratch` with Rust backend + SvelteKit frontend

## Screenshots

<!-- TODO: Add screenshots -->

| Dashboard | Cluster Detail | Branding Editor |
|-----------|----------------|-----------------|
| *(screenshot)* | *(screenshot)* | *(screenshot)* |

## Quick Start

### Docker Compose

```bash
docker compose up -d
# Open http://localhost:8081
```

### Kubernetes (Helm)

```bash
helm repo add tcs https://charts.talos.dev
helm install tcs tcs/tcs --set ingress.enabled=true --set ingress.hosts[0].host=tcs.example.com
```

### Local Development

```bash
# Backend
cd backend && cargo run

# Frontend
cd frontend && npm install && npm run dev
# Open http://localhost:5173
```

## Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  SvelteKit  │────▶│   Axum API   │────▶│  SQLite/PG  │
│  Frontend   │     │  (Rust)      │     │  Database   │
└─────────────┘     └──────┬───────┘     └─────────────┘
                           │
              ┌────────────┼────────────┐
              ▼            ▼            ▼
         ┌────────┐  ┌──────────┐  ┌────────┐
         │  gRPC  │  │gRPC+TLS  │  │ Sidero │
         │ Talos  │  │K8s API   │  │ link   │
         │ Client │  │  Client  │  │ Tunnel │
         └────────┘  └──────────┘  └────────┘
```

## Project Structure

```
talos-control-system/
├── Dockerfile
├── deploy/
│   └── helm/              # Helm chart
├── docs/                  # Documentation
├── backend/               # Rust (Axum + Tonic)
│   ├── src/
│   │   ├── api/           # REST + gRPC layers
│   │   ├── auth/          # JWT, OIDC, SAML, RBAC
│   │   ├── branding/      # White-label engine
│   │   ├── config/        # TOML config loader
│   │   ├── controllers/   # Business logic
│   │   ├── db/            # Models + repositories
│   │   ├── integration/   # Talos + K8s clients
│   │   ├── network/       # Siderolink, proxy, DNS
│   │   ├── runtime/       # Event bus, DAG, cache
│   │   └── utils/         # Metrics, logging, version
│   └── migrations/
└── frontend/              # SvelteKit + Tailwind CSS
    └── src/
        ├── lib/
        │   ├── api/       # REST client
        │   ├── branding/  # Logo component
        │   ├── components/
        │   ├── stores/    # Svelte stores
        │   └── styles/
        └── routes/
```

## Documentation

- [Installation Guide](docs/INSTALL.md)
- [Configuration Reference](docs/CONFIGURATION.md)
- [White-Label Branding](docs/BRANDING.md)
- [Kubernetes Deployment](docs/DEPLOYMENT.md)
- [Local Development](docs/DEVELOPMENT.md)

## License

Apache-2.0 — See [LICENSE](LICENSE) for details.
