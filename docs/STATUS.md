# TCS Status (Alpha)

**Last updated:** 2026-08-09

Talos Control System is an **alpha** management UI for Talos Linux clusters.
It runs as a **host systemd service** (or bare binary), **outside** the managed
cluster. Prefer **import + observe + limited Talos actions** over greenfield claims.

## Feature matrix

| Feature | Status | Notes |
|---------|--------|--------|
| Local auth (Argon2) + JWT | **Supported** | Set `TCS_AUTH_JWT_SECRET` in production |
| OIDC | **Supported (alpha)** | Code flow + discovery + CSRF state; validate against your IdP |
| LDAP / AD | **Supported (alpha)** | Service bind + search + user bind; validate against your directory |
| SAML | **Not available** | UI disabled |
| RBAC (admin / operator / reader) | **Supported** | Coarse route-level; no cluster scopes yet |
| White-label branding | **Supported** | Single tenant (`default`) |
| Per-tenant branding | **Planned** | Schema exists; not multi-tenant |
| Cluster import (kubeconfig) | **Supported** | Encrypted kubeconfig when provided |
| Talosconfig attach | **Supported** | Required for machine API; encrypted at rest |
| Cluster / machine inventory CRUD | **Supported** | |
| Config patches (store) | **Supported** | |
| Config patches (apply via Talos) | **Supported** | Pure-Rust COSI get + multi-doc merge + ApplyConfiguration |
| Etcd snapshot backup / download | **Supported** | Control-plane node; retention configurable |
| Scheduled etcd backups | **Supported** | Per-cluster hours; ~15m scheduler tick |
| Etcd restore | **Supported** | Destructive; requires confirm |
| Machine version probe | **Supported** | Parallel cluster probe |
| Machine hostname / service list | **Supported** | |
| Machine reboot / upgrade | **Supported** | |
| Cluster create (provision) | **Not implemented** | UI inventory record only |
| Scale / destroy machines | **Not implemented** | |
| Siderolink discovery | **Not implemented** | Config stub only |
| In-process gRPC server | **Not implemented** | Outbound Talos client only |
| Postgres | **Not implemented** | SQLite only |
| Audit log | **Supported** | Durable in SQLite |
| Host installer (self-extracting) | **Supported** | `scripts/package-installer.sh` → `tcs-*-linux-*.sh` |
| Helm / in-cluster deploy | **Not provided** | Intentionally out of scope |
| Docker image | **Not provided** | Binary + systemd first |

## Network requirements

TCS host must reach machine addresses on **TCP 50000** with talosconfig mTLS for
Talos actions, and the Kubernetes API for import/refresh.

## Deferred (explicitly not alpha)

- Greenfield bootstrap / metal provisioning  
- Siderolink WireGuard tunnels  
- SAML SSO  
- HA multi-replica TCS + Postgres  
- Fleet multi-cluster upgrade waves  
- Full multi-tenancy  
- Running TCS inside the managed Kubernetes cluster  

See [INSTALL](INSTALL.md), [CONFIGURATION](CONFIGURATION.md), [AUTH](AUTH.md), [TALOS](TALOS.md).
