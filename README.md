# Talos Control System

**Status: Alpha** — see [docs/STATUS.md](docs/STATUS.md) for an honest feature matrix.

**Talos Control System (TCS)** is a self-hosted web UI for managing [Talos Linux](https://www.talos.dev/) clusters. Import existing clusters, inventory machines, apply config patches, take etcd snapshots, and run limited machine actions over the Talos gRPC API.

TCS runs **on a management host** (systemd binary). It is **not** deployed with Helm and is **not** intended to run inside the managed Kubernetes cluster.

## What works today

- Local auth (Argon2) + JWT; LDAP, OIDC, and SAML SP (alpha — validate in your environment)
- RBAC (admin / operator / reader) + optional per-cluster memberships
- White-label + multi-tenant branding (`X-Tenant-ID` / subdomain)
- Cluster **import** via kubeconfig (+ optional talosconfig)
- Inventory CRUD for clusters and machines
- Config patches applied via Talos (pure-Rust COSI merge + ApplyConfiguration)
- Real etcd snapshots (download, schedule, retention, disaster-recovery restore)
- Machine version probe, reboot, upgrade
- Cluster / fleet **rolling upgrade jobs** (max-unavailable, control-plane-last)
- Greenfield config factory + apply/bootstrap/scale helpers (metal still external)
- Machine reset (Talos wipe) with confirm
- Siderolink inventory + **WireGuard** (host `wg`/`ip` when available)
- **Postgres** dual-backend runtime (SQLite default)
- Multi-replica HA foundation (scheduler locks + shared OIDC state)

## What does **not** work yet

- Full bare-metal PXE/IPMI orchestration
- Automatic SQLite→Postgres data migrator
- In-cluster / Helm deployment (by design)

## Quick start

### Local development

```bash
cd frontend && npm install && npm run build && cd ..
export TCS_ALLOW_INSECURE=1
export TCS_DEFAULT_ADMIN_PASSWORD=admin
cd backend && cargo run
# Open http://localhost:8081  →  admin@tcs.local / admin
```

**Production:** set `TCS_AUTH_JWT_SECRET` (never ship the default secret).

### Host install (self-extracting)

```bash
# After a release is published:
curl -fsSL -o tcs-install.sh \
  "https://github.com/OWNER/REPO/releases/download/vX.Y.Z/tcs-X.Y.Z-linux-x86_64.sh"
chmod +x tcs-install.sh
sudo ./tcs-install.sh
```

Details: [docs/INSTALL.md](docs/INSTALL.md).

### Build installer from source

```bash
export GIT_HASH="$(git rev-parse --short=12 HEAD)"
export BUILD_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cd frontend && npm ci && npm run build && cd ../backend
cargo build --release
cd ..
./scripts/package-installer.sh \
  --binary backend/target/release/talos-control-system \
  --version 0.1.0-dev
sudo ./dist/tcs-0.1.0-dev-linux-*.sh
```

## Architecture

```
SvelteKit UI  →  Axum REST (/api)  →  SQLite on the host
                      │
         outbound gRPC+mTLS → Talos nodes :50000
         outbound HTTPS     → Kubernetes API (import/refresh)
```

## Project layout

```
talos-control-system/
├── backend/           # Rust (Axum + Talos gRPC client)
├── frontend/          # SvelteKit + Tailwind (embedded in binary)
├── scripts/           # package-installer.sh, install.sh.in
├── docs/
├── config.example.toml
└── LICENSE            # Apache-2.0
```

## Documentation

- [Status / feature matrix](docs/STATUS.md)
- [Installation](docs/INSTALL.md)
- [Host deployment](docs/DEPLOYMENT.md)
- [Configuration](docs/CONFIGURATION.md)
- [Talos API control](docs/TALOS.md)
- [Authentication](docs/AUTH.md)
- [TLS](docs/TLS.md)
- [Branding](docs/BRANDING.md)
- [Development](docs/DEVELOPMENT.md)
- [Lab smoke checklist](docs/SMOKE.md)
- [Operations / RBAC scopes](docs/OPS.md)

## License

Apache-2.0 — see [LICENSE](LICENSE).
