# Machine inventory (greenfield & metal)

## Ways to add machines

| Method | Use when |
|--------|----------|
| **YAML/CSV import** | Define a rack / cluster from scratch (MAC + BMC for PXE) |
| **Provision wizard** | Interactive one-by-one + metal job |
| **Cluster import** | Brownfield: kubeconfig of a running Talos cluster |
| **API** | `POST /api/machines` single create |

## YAML schema

```yaml
cluster:
  name: rack-a
  endpoint: https://10.88.0.10:6443   # optional at import
  talosVersion: v1.13.7
  kubernetesVersion: v1.36.3
machines:
  - hostname: cp-1
    role: controlplane                 # controlplane | worker
    mac: aa:bb:cc:dd:ee:01
    address: ""                        # optional until DHCP/PXE
    installDisk: ""                    # optional
    bmc:
      address: 10.90.0.11
      username: root
      password: secret
      type: auto                       # auto | redfish | ipmi
```

Bare list is also accepted: `machines: [ ... ]` only.

## CSV schema

Header row (case-insensitive):

```csv
hostname,role,mac,address,install_disk,bmc_address,bmc_username,bmc_password,bmc_type
cp-1,controlplane,aa:bb:cc:dd:ee:01,,/dev/sda,10.90.0.11,root,secret,auto
```

## API

```http
POST /api/machines/import/preview
POST /api/machines/import
{
  "format": "yaml" | "csv",
  "content": "...",
  "clusterId": null,
  "createCluster": true,
  "createClusterName": "optional",
  "upsertByMac": true
}
```

UI: **Machines → Import CSV/YAML**.

## Edit after import

Open **Machines → (row)** or **Cluster → Machines → (row)**.

Editable: hostname, role, cluster, MAC, address, install disk, BMC credentials/power.

## Network / mounts / install image on a single node

1. Open **Machines → (node)** → **Machine config**.
2. **Load live from node** (needs talosconfig + address) or paste full YAML.
3. Use **helpers** for install image (factory schematic with kernel modules), network
   YAML, and kubelet extra mounts — or edit the full YAML directly.
4. **Save desired** (DB only), **Dry-run**, then **Apply to node** (optional reboot).

Cluster-wide path patches still work under **Cluster → Config** (`machineId` optional).

## MAAS (later)

Planned: read-only sync of machine MAC + power params into this same inventory schema.
Until then, export from MAAS to CSV/YAML and import here. Do not run TCS DHCP on the same
VLAN as MAAS DHCP.

## Metal DHCP/PXE without process restart

**Settings → Metal / PXE → Apply** writes `/var/lib/tcs/metal.toml` and rebinds listeners
in-process. Dedicated provision VLAN still required when enabling DHCP.
