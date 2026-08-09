# Lab smoke checklist

Use after deploy or before a release. Requires a reachable Talos cluster and mTLS talosconfig.

## Prerequisites

```bash
export TCS_ALLOW_INSECURE=1   # lab only
export TCS_DEFAULT_ADMIN_PASSWORD=admin
cd frontend && npm ci && npm run build
cd ../backend && cargo run
```

Open `http://localhost:8081`, log in as `admin@tcs.local`.

## Checklist

| # | Step | Pass? |
|---|------|-------|
| 1 | Login works; password change if forced | |
| 2 | **Import** cluster with kubeconfig + talosconfig | |
| 3 | Cluster shows `hasTalosconfig` / `hasKubeconfig` | |
| 4 | **Test Talos** on cluster detail returns ok results | |
| 5 | Machine **Version** returns a tag; **Services** lists apid/etcd/kubelet | |
| 6 | **Create Backup** → status `ready`, size &gt; 0 | |
| 7 | **Download** snapshot file | |
| 8 | Config patch **Dry-run** then **Apply** | |
| 9 | Set backup schedule (e.g. 24h), confirm saved on cluster GET | |
| 10 | System info shows feature flags (`etcdBackup`, `etcdRestore`) | |

## Optional / careful

| Step | Notes |
|------|--------|
| Machine reboot | Non-prod node only |
| Etcd restore | Disaster recovery only; type `RESTORE` in UI |

## Network

TCS host must reach machine addresses on **TCP 50000**. Edit machine address if `internal_ip` is not reachable from TCS.
