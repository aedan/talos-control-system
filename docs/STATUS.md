# TCS Status (Alpha)

**Last updated:** 2026-08-09

Talos Control System is an **alpha** management UI for Talos Linux clusters.
It runs as a **host systemd service** (or bare binary), **outside** the managed
cluster. Prefer **import + observe + day-2 Talos actions** over greenfield claims.

## Feature matrix

| Feature | Status | Notes |
|---------|--------|--------|
| Local auth (Argon2) + JWT | **Supported** | Set `TCS_AUTH_JWT_SECRET` in production |
| OIDC | **Supported (alpha)** | Code flow + discovery + CSRF + JWKS; browser callback stores JWT |
| LDAP / AD | **Supported (alpha)** | Service bind + search + user bind |
| SAML 2.0 SP | **Supported (alpha)** | AuthnRequest + ACS parse; metadata URL; full XML-DSig best-effort |
| RBAC (admin / operator / reader) | **Supported** | Global + optional per-cluster memberships |
| White-label branding | **Supported** | Config + Settings UI |
| Per-tenant branding | **Supported (alpha)** | `X-Tenant-ID` or subdomain → `tenant_branding` |
| Cluster import (kubeconfig) | **Supported** | Encrypted kubeconfig when provided |
| Talosconfig attach | **Supported** | Required for machine API; encrypted at rest |
| Cluster / machine inventory CRUD | **Supported** | |
| Config patches (store + apply) | **Supported** | Pure-Rust COSI get + multi-doc merge |
| Etcd snapshot backup / restore | **Supported** | Scheduled backups supported |
| Machine version / reboot / upgrade | **Supported** | Parallel cluster probe |
| Cluster / fleet rolling upgrade jobs | **Supported (alpha)** | Scheduler + max-unavailable + CP-last |
| Greenfield config factory | **Supported (alpha)** | `talosctl gen config` or template stub; no metal provision |
| Siderolink inventory | **Supported (alpha)** | Register + join tokens; **no WireGuard data path** |
| Postgres | **Schema bootstrap only** | Runtime is SQLite; see [POSTGRES](POSTGRES.md) |
| Host installer (self-extracting) | **Supported** | `scripts/package-installer.sh` |
| Helm / in-cluster deploy | **Not provided** | Intentionally out of scope |

## Network requirements

TCS host must reach machine addresses on **TCP 50000** with talosconfig mTLS for
Talos actions, and the Kubernetes API for import/refresh.

## Explicitly still later

- Siderolink WireGuard tunnels / full discovery plane  
- Bare-metal provision / scale / destroy  
- Full dual-backend Postgres runtime + multi-replica HA  
- Production-grade SAML XML-DSig (validate with your IdP)  

See [INSTALL](INSTALL.md), [CONFIGURATION](CONFIGURATION.md), [AUTH](AUTH.md), [TALOS](TALOS.md), [POSTGRES](POSTGRES.md).
