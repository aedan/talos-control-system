# Lab smoke checklist

Use after deploy or before a release. Requires a reachable Talos cluster and mTLS talosconfig.

## Prerequisites

```bash
# Lab credentials after first boot or admin password reset
# Email: admin@tcs.local
# Password: set via TCS_DEFAULT_ADMIN_PASSWORD (empty DB only) or Users → Reset PW
```

Open the TCS UI (e.g. `https://devstation.jakelab.info`), log in.

## API quick smoke (from TCS host)

```bash
TOKEN=$(curl -sk -X POST https://127.0.0.1/api/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"admin@tcs.local","password":"YOUR_PASSWORD"}' | jq -r .token)

curl -sk -H "Authorization: Bearer $TOKEN" https://127.0.0.1/api/clusters | jq length
curl -sk -H "Authorization: Bearer $TOKEN" https://127.0.0.1/api/machines | jq length
curl -sk -H "Authorization: Bearer $TOKEN" https://127.0.0.1/api/settings/system/info | jq .features
```

## Checklist

| # | Step | Pass? |
|---|------|-------|
| 1 | Login works; **sidebar appears immediately** (no reload required) | |
| 2 | Overview shows cluster/machine counts | |
| 3 | **Import** cluster with kubeconfig + talosconfig (or existing inventory OK) | |
| 4 | Cluster **Test Talos** / **Probe versions** | |
| 5 | Machine **Version** / **Services** / **Hostname** | |
| 6 | **Create Backup** → ready; **Download** | |
| 7 | Config patch dry-run / apply | |
| 8 | **Generate machine configs** (create wizard) | |
| 9 | Rolling upgrade UI (cluster or `/upgrades`) — optional cancel | |
| 10 | Scale workers (inventory) | |
| 11 | Siderolink: create join token | |
| 12 | Users: **Reset PW** for a local user | |
| 13 | System info feature flags include `postgres`, `multiReplicaHa`, `machineReset` | |
| 14 | Branding page loads; optional `X-Tenant-ID` | |

## Optional / careful

| Step | Notes |
|------|--------|
| Machine reboot | Non-prod only |
| Machine **Reset** | Destructive wipe |
| Bootstrap | Control-plane only; initial etcd |
| Etcd restore | Disaster recovery only |
| WireGuard | Needs `wireguard-tools` + privileges on TCS host |

## Network

TCS host must reach machine addresses on **TCP 50000**.

## Postgres cutover (optional)

```bash
TCS_SQLITE_PATH=/var/lib/tcs/data.db \
TCS_POSTGRES_URL='postgresql://tcs:secret@db:5432/tcs' \
  tcs migrate-sqlite-to-postgres
# then set database.backend = "postgres" and restart
```
