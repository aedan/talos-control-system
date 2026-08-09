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
| Reboot | `POST /api/machines/{id}/reboot` | talosconfig + address |
| Upgrade | `POST /api/machines/{id}/upgrade` | `{ "image": "ghcr.io/siderolabs/installer:v1.x" }` |
| Apply config patches | `POST /api/clusters/{id}/config/apply` | optional `{ "dry_run": true }` |
| Etcd snapshot | `POST /api/clusters/{id}/backups` | control-plane node |
| Download snapshot | `GET /api/clusters/{id}/backups/{id}/download` | |
| Test Talos connectivity | `POST /api/clusters/{id}/talos/test` | |
| Refresh nodes from K8s | `POST /api/clusters/{id}/refresh` | stored kubeconfig |

## Backups

Snapshots are written under `{parent of sqlite_path}/backups/{cluster_id}/`.
Retention: newest **N** ready backups kept (default 10); older files deleted.

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
