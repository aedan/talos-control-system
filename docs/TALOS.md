# Talos API control plane

TCS talks **outbound** to Talos machines using the official machine gRPC API
(port **50000**) via mTLS. It does **not** run a Talos-compatible gRPC server
for machines to dial into (Siderolink is not implemented).

## Credentials

### Talosconfig

Same format as `~/.talos/config`:

```yaml
context: my-cluster
contexts:
  my-cluster:
    endpoints:
      - https://10.0.0.2:50000
    nodes:
      - 10.0.0.2
    ca: <base64>
    crt: <base64>
    key: <base64>
```

Provide at **import** or later:

```http
PUT /api/clusters/{id}/talosconfig
{ "talosconfig": "..." }
```

Stored **encrypted** at rest (key derived from `jwt_secret`).

### Kubeconfig

Used for Kubernetes discovery on import and for **refresh**. Stored encrypted.

## Machine addresses

On import, TCS stores each node’s Kubernetes `internal_ip` as `machines.address`.
If TCS cannot reach that address, set a reachable IP/DNS:

```http
PUT /api/machines/{id}
{ "address": "192.168.1.50" }
```

## Operations

| Action | Method | Requires |
|--------|--------|----------|
| Probe version | `GET /api/machines/{id}/version` | talosconfig + address |
| Hostname | `GET /api/machines/{id}/hostname` | talosconfig + address |
| Service list | `GET /api/machines/{id}/services` | talosconfig + address |
| Reboot | `POST /api/machines/{id}/reboot` | talosconfig + address |
| Upgrade (single machine) | `POST /api/machines/{id}/upgrade` | `{ "image": "ghcr.io/siderolabs/installer:v1.x" }` |
| Reset / wipe machine | `POST /api/machines/{id}/reset` | `{ "confirm": true, "graceful": true, "reboot": true }` |
| Bootstrap control plane | `POST /api/machines/{id}/bootstrap` | initial etcd formation |
| Rolling upgrade (cluster) | `POST /api/clusters/{id}/upgrade` | `{ "image", "maxUnavailable", "controlPlaneLast" }` |
| Rolling upgrade (fleet) | `POST /api/fleets/upgrades` | `{ "clusterIds", "image", ... }` |
| Scale workers (desired) | `POST /api/clusters/{id}/scale` | `{ "desiredWorkers": N }` inventory target |
| Upgrade job status | `GET /api/upgrade-jobs`, `GET /api/upgrade-jobs/{id}` | cancel via `POST .../cancel` |
| Generate machine configs | `POST /api/clusters/generate-config` | greenfield assist (`talosctl` or stub) |
| Apply provision config | `POST /api/provision/apply-config` | `{ "machineId", "configYaml" }` |
| Apply config patches | `POST /api/clusters/{id}/config/apply` | optional `{ "dry_run": true }` |
| Etcd snapshot | `POST /api/clusters/{id}/backups` | control-plane node |
| Download snapshot | `GET /api/clusters/{id}/backups/{id}/download` | |
| Restore snapshot | `POST /api/clusters/{id}/backups/{id}/restore` | see below |
| Test Talos connectivity | `POST /api/clusters/{id}/talos/test` | |
| Refresh nodes from K8s | `POST /api/clusters/{id}/refresh` | stored kubeconfig |

### Etcd restore (disaster recovery)

**Destructive.** Only use when recovering a broken control plane.

```http
POST /api/clusters/{id}/backups/{backupId}/restore
{
  "confirm": true,
  "runBootstrap": true,
  "skipHashCheck": false,
  "machineId": null
}
```

1. Uploads the snapshot to a control-plane node via Talos **EtcdRecover**  
2. If `runBootstrap` is true, calls **Bootstrap** with `recover_etcd=true`  

Prefer a maintenance window. Do not restore onto a healthy multi-node etcd without following Talos recovery docs.

## Backups

Snapshots are written under `{parent of sqlite_path}/backups/{cluster_id}/`.
Retention: newest **N** ready backups kept (default 10); older files deleted.

### Schedule

```http
PUT /api/clusters/{id}/backups/schedule
{ "scheduleHours": 24, "retention": 10 }
```

- `scheduleHours`: interval between automatic snapshots; `null` or `0` disables  
- Scheduler loop ticks about every **15 minutes**  
- Requires talosconfig on the cluster

## Common failures

| Error | Cause |
|-------|--------|
| No talosconfig | Attach credentials |
| Connection failed | Firewall / wrong address / wrong certs |
| EtcdSnapshot failed | Target is not control-plane or role insufficient |
| Apply failed | Invalid patch YAML / mode conflict |

## Not implemented

- Etcd **restore** automation  
- Siderolink registration  
- Generating new clusters from scratch  
