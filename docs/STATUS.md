# TCS Status (Alpha)

**Last updated:** 2026-08-10 · **Release target: v0.2.0**

Talos Control System is an **alpha** management UI for Talos Linux clusters.
It runs as a **host systemd service** (or bare binary), **outside** the managed
cluster.

## Feature matrix

| Feature | Status | Notes |
|---------|--------|--------|
| Local auth + JWT | **Supported** | `TCS_AUTH_JWT_SECRET` required in production |
| OIDC | **Supported (alpha)** | Code flow + JWKS; **DB-backed CSRF state** for multi-replica |
| LDAP / AD | **Supported (alpha)** | Service bind + search + user bind |
| SAML 2.0 SP | **Supported (alpha)** | AuthnRequest + ACS; XML-DSig best-effort |
| RBAC + per-cluster memberships | **Supported** | |
| White-label + multi-tenant branding | **Supported (alpha)** | `X-Tenant-ID` / subdomain |
| Cluster import | **Supported** | kubeconfig + talosconfig |
| Config apply (pure-Rust COSI) | **Supported** | |
| Etcd backup / restore / schedule | **Supported** | Leader-only when multi-replica |
| Rolling upgrade jobs (cluster/fleet) | **Supported (alpha)** | Leader-elected scheduler |
| Greenfield config factory | **Supported (alpha)** | talosctl or template stub |
| Apply provision config to machine | **Supported (alpha)** | `POST /api/provision/apply-config` |
| Bootstrap control-plane machine | **Supported (alpha)** | Talos Bootstrap RPC |
| Scale workers (desired size) | **Supported (alpha)** | Inventory target; metal still external |
| Machine reset / wipe | **Supported (alpha)** | Talos Reset RPC; requires confirm |
| Siderolink inventory | **Supported** | Join tokens + register |
| Siderolink WireGuard | **Supported (alpha)** | Host `wg`/`ip` when available; graceful degrade |
| **Postgres runtime** | **Supported (alpha)** | Dual `DbPool`; SQLite default |
| Multi-replica HA foundation | **Supported (alpha)** | DB locks for schedulers; OIDC state in DB |
| Host installer | **Supported** | Self-extracting |
| Helm / in-cluster | **Not provided** | By design |
| Full bare-metal PXE/redfish provision | **Not provided** | Out of band; TCS assists configs + apply |

## Explicitly still later

- Production-grade SAML XML-DSig  
- Automatic metal discovery / IPMI / PXE orchestration  
- Full multi-region HA story (sticky sessions, shared object store for backups)  

See [INSTALL](INSTALL.md), [CONFIGURATION](CONFIGURATION.md), [AUTH](AUTH.md), [POSTGRES](POSTGRES.md), [TALOS](TALOS.md).
