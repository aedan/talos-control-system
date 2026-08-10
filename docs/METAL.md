# Bare-metal provisioning (PXE + DHCP + BMC)

TCS can own the full path from **BMC power** → **PXE boot Talos installer** → **install** → **bootstrap**.

> **Safety:** DHCP is **off by default**. When enabled, bind it to a **dedicated provisioning interface/VLAN** only.

## Architecture

| Component | Role |
|-----------|------|
| BMC (Redfish primary, IPMI fallback) | Power on/off/cycle; set boot to PXE once |
| DHCP (full server) | Leases for known machine MACs; next-server + bootfile |
| PXE HTTP | iPXE script + Talos kernel/initramfs |
| Provision job worker | Orchestrates steps per machine (HA-locked) |
| Existing install assist | Disks, apply config + reboot, bootstrap |

## Configuration

```toml
[metal]
enabled = true

[metal.dhcp]
enabled = true
interface = "eth1"              # REQUIRED — dedicated provision NIC
# bind_ip = "10.88.0.1"         # optional; else interface primary
subnet = "10.88.0.0/24"
range_start = "10.88.0.100"
range_end = "10.88.0.200"
gateway = "10.88.0.1"
dns = ["10.88.0.1"]
lease_ttl_secs = 3600
allow_unknown = false           # only inventory MACs get leases

[metal.pxe]
enabled = true
http_port = 6969
asset_dir = "/var/lib/tcs/pxe"
default_talos_version = "v1.13.7"
# mirror_base = "https://github.com/siderolabs/talos/releases/download"

[metal.bmc]
connect_timeout_secs = 15
prefer_redfish = true
ipmi_interface = "lanplus"
```

Env overrides use the `TCS_METAL_*` prefix (same pattern as other config).

### Host requirements

- Root (or CAP_NET_BIND_SERVICE / CAP_NET_RAW) for UDP/67 DHCP
- BMC network reachability from the TCS host
- Optional: `ipmitool` for classic IPMI (`apt install ipmitool`)
- Provision VLAN must **not** share DHCP with another server

## Operator workflow

1. Enable metal DHCP/PXE in config and restart TCS.
2. **Settings → Metal / PXE** → create profile → **Sync assets** (downloads kernel + initramfs).
3. **Clusters → Provision bare metal**:
   - Create cluster + **Generate PKI + configs** (talosconfig is auto-attached).
   - Register machines with **MAC + BMC** credentials (and role).
   - **Start metal provision**.
4. Job steps: set PXE → power on/cycle → wait installer → install → bootstrap (CP) → restore disk boot.
5. Monitor jobs and DHCP leases on the Metal settings page.

### Manual / hybrid path

Still supported: boot ISO/USB yourself, register by **address**, discover disks, install, bootstrap.

## API surface

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/machines` | Create inventory (MAC, BMC, address) |
| PUT/GET | `/api/machines/:id/bmc` | Set credentials / probe power |
| POST | `/api/machines/:id/power` | `{ "action": "on\|off\|cycle\|reset" }` |
| POST | `/api/machines/:id/boot-device` | `{ "target": "pxe\|disk", "once": true }` |
| GET | `/api/metal/status` | Config snapshot |
| GET | `/api/metal/dhcp/leases` | Active leases |
| GET/POST | `/api/pxe/profiles` | Profiles |
| POST | `/api/pxe/profiles/:id/sync` | Download assets |
| POST | `/api/clusters/:id/provision` | Start metal job |
| GET | `/api/provision-jobs` | List jobs |
| POST | `/api/provision-jobs/:id/cancel` | Cancel |

PXE HTTP (when enabled): `http://<next-server>:6969/pxe/ipxe/<mac>` and `/pxe/assets/...`.

## BMC notes

- **Redfish** first (`/redfish/v1/Systems`), basic auth, optional insecure TLS (default on for lab BMCs).
- **IPMI** via `ipmitool -I lanplus` when Redfish fails or `bmc_type=ipmi`.
- Passwords encrypted at rest with the same key material as talosconfig (`jwt_secret`).

## Limitations (alpha)

- No multi-VLAN DHCP without separate TCS instances / config
- No automatic BMC discovery / rack crawl
- No Secure Boot enrollment UX
- UEFI HTTP/iPXE client firmware varies; some NICs need iPXE binary as bootfile (place under asset_dir as follow-up)
- Full DHCP must not collide with production DHCP
