# TCS Status (Alpha)

**Last updated:** 2026-10-10 · **Release: v0.5.91** (this page matches `main`, including fixes landed after the v0.5.91 changelog entry)

Talos Control System is an **alpha** management UI for Talos Linux clusters.
It runs as a **host systemd service** (or bare binary), **outside** the managed
cluster.

## Feature matrix

| Feature | Status | Notes |
|---------|--------|--------|
| Local auth + JWT | **Supported** | Signing secret lives in the database vault. A host key unlocks it; a one-time break-glass passphrase recovers a lost host key. |
| OIDC | **Supported (alpha)** | Code flow + JWKS; **DB-backed CSRF state** for multi-replica |
| LDAP / AD | **Supported (alpha)** | Service bind + search + user bind |
| SAML 2.0 SP | **Supported (alpha)** | AuthnRequest + ACS; XML-DSig best-effort |
| RBAC + per-cluster memberships | **Supported** | |
| White-label + multi-tenant branding | **Supported (alpha)** | `X-Tenant-ID` / subdomain |
| Cluster import | **Supported** | kubeconfig + talosconfig |
| Dashboard-centric IA | **Supported** | No sidebar; clusters + counts on home, Settings top-bar dropdown |
| Config apply (pure-Rust COSI) | **Supported** | |
| Etcd backup / restore / schedule | **Supported** | Leader-only when multi-replica |
| Rolling upgrade jobs (per-cluster) | **Supported (alpha)** | Talos image roll (reboots) + optional in-place Kubernetes upgrade; max-unavailable, control-plane-last. A second job is rejected while one is pending or running. `clusters.talos_version` updates when every node agrees on the new tag. |
| In-place Kubernetes upgrade | **Supported (alpha)** | `talosctl upgrade-k8s` (no reboots, CP first); multi-minor ladders via sequential steps; drain/uncordon of workers via stored kubeconfig |
| Image-Factory modules | **Supported (alpha)** | Cluster-default set + per-node +/− deltas + reset-to-defaults; effective set drives the installer schematic. Import does not yet copy the extensions loaded on a running node; set them before the next upgrade or that upgrade builds an image without them. |
| Greenfield config factory | **Supported (alpha)** | Pure-Rust PKI + machine configs (no talosctl) |
| In-place kubeadm/Kubespray convert | **Supported (alpha)** | Replaces the OS with Talos on the existing machines. One machine CA per cluster. See [convert-takeover-lessons.md](convert-takeover-lessons.md). |
| **Bare-metal install assist** | **Supported (alpha)** | Disk list, set install disk, apply+reboot install, bootstrap CP |
| **PXE + full DHCP** | **Supported (alpha)** | Dedicated provision iface; MAC allowlist; HTTP iPXE + assets |
| **BMC Redfish / IPMI** | **Supported (alpha)** | Power + PXE-once; IPMI via ipmitool fallback. Host NIC MACs are read from Redfish EthernetInterfaces and Dell `ipmitool delloem mac` when inventory has an out-of-band login but no MAC. |
| **iDRAC 7 virtual console** | **Supported (alpha)** | Machine page opens the HTML5 KVM viewer |
| **Metal provision jobs** | **Supported (alpha)** | BMC → PXE → wait installer → install → bootstrap |
| **Inventory YAML/CSV import** | **Supported (alpha)** | Bulk MAC/BMC inventory; optional create cluster |
| **Live metal config** | **Supported (alpha)** | Settings UI stores metal config in the database and rebinds DHCP/PXE without a process restart |
| Scale workers (desired size) | **Supported (alpha)** | Inventory target |
| Machine reset / wipe | **Supported (alpha)** | Talos Reset RPC; requires confirm |
| Live TLS cert reload | **Supported** | Self-signed ↔ LE ↔ provided without restart when HTTPS already up |
| Siderolink inventory | **Supported** | Join tokens + register |
| Siderolink WireGuard | **Supported (alpha)** | Host `wg`/`ip` when available; graceful degrade. Node addresses are IPv6 ULAs. `[siderolink] subnet` optionally adds an IPv4 address on `tcs-sl0` (default `100.64.0.1/10`); `""` adds none. Nodes dial UDP `listen_port` (default 443). |
| **Postgres runtime** | **Supported (alpha)** | Dual `DbPool`; SQLite default |
| Multi-replica HA foundation | **Supported (alpha)** | DB locks for schedulers; OIDC state in DB |
| Host installer | **Supported** | Self-extracting |
| Helm / in-cluster | **Not provided** | By design |
| Full rack BMC discovery | **Not provided** | Host NIC MACs can be filled from a BMC that is already in inventory. The rack itself is not scanned. |

## Bare-metal: what “complete” means

**In scope (done):**

1. **Assisted:** nodes already in Talos installer → register → disks → install → bootstrap.  
2. **Full path (alpha):** machine inventory with MAC + BMC → Redfish/IPMI power + PXE once → TCS DHCP + HTTP boot into Talos metal → install → bootstrap (job worker).

**Still out of scope:** automatic rack BMC discovery, multi-region metal HA, Secure Boot enrollment UX.

## Explicitly still later

- Production-grade SAML XML-DSig  
- Full multi-region HA story  
- proxyDHCP mode (optional alternative to full DHCP)  

See [METAL](METAL.md), [INSTALL](INSTALL.md), [CONFIGURATION](CONFIGURATION.md), [AUTH](AUTH.md), [POSTGRES](POSTGRES.md), [TALOS](TALOS.md), [SIDEROLINK](SIDEROLINK.md), [TLS](TLS.md).
