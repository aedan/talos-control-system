# TCS Status (Alpha)

**Last updated:** 2026-08-09

Talos Control System is an **alpha** management UI for Talos Linux clusters.
Prefer **import + observe + limited Talos actions** over greenfield lifecycle claims.

## Feature matrix

| Feature | Status | Notes |
|---------|--------|--------|
| Local auth (Argon2) + JWT | **Supported** | Set `TCS_AUTH_JWT_SECRET` in production |
| OIDC | **Partial** | Implemented; needs real IdP validation |
| LDAP | **Partial** | Implemented; needs real AD validation |
| SAML | **Not available** | UI disabled |
| RBAC (admin / operator / reader) | **Supported** | Coarse route-level; no cluster scopes yet |
| White-label branding | **Supported** | Single tenant (`default`) |
| Per-tenant branding | **Planned** | Schema exists; not multi-tenant |
| Cluster import (kubeconfig) | **Supported** | Stores encrypted kubeconfig when provided |
| Talosconfig attach | **Supported** | Required for machine API actions; encrypted at rest |
| Cluster / machine inventory CRUD | **Supported** | |
| Config patches (store) | **Supported** | |
| Config patches (apply via Talos) | **Supported** | Strategic merge; needs network to :50000 |
| Etcd snapshot backup / download | **Supported** | Control-plane node; retention configurable |
| Scheduled etcd backups | **Supported** | Per-cluster interval hours; ~15m scheduler tick |
| Etcd restore | **Supported** | EtcdRecover + optional Bootstrap; destructive; requires confirm |
| Machine version probe | **Supported** | |
| Machine hostname / service list | **Supported** | Day-2 inspection via Talos API |
| Machine reboot | **Supported** | |
| Machine upgrade | **Supported** | Image string via API |
| Cluster create (provision) | **Not implemented** | UI creates **inventory record only** |
| Scale / destroy machines | **Not implemented** | Status fields only |
| Siderolink discovery | **Not implemented** | Config stub only |
| In-process gRPC server | **Not implemented** | Outbound Talos gRPC client only |
| Postgres | **Not implemented** | SQLite only; startup fails if postgres selected |
| Audit log | **Supported** | Durable in SQLite |
| Docker / scratch image | **Not provided** | Single binary + installer |
| Helm chart | **Experimental** | Chart sources under `deploy/helm` |

## Network requirements

For Talos actions (backup, apply, reboot, version, upgrade), the TCS host must reach node addresses on **TCP 50000** with the cluster talosconfig mTLS credentials.

## Deferred (explicitly not in alpha)

- Greenfield bootstrap / metal provisioning  
- Siderolink WireGuard tunnels  
- SAML SSO  
- HA multi-replica TCS + Postgres  
- Fleet multi-cluster upgrade waves  
- Full multi-tenancy  

See also [INSTALL](INSTALL.md), [CONFIGURATION](CONFIGURATION.md), [TALOS](TALOS.md).
