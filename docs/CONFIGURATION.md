<!-- Alpha: see docs/STATUS.md and docs/TALOS.md for real capabilities -->

# Configuration Reference

TCS is configured via a TOML file and environment variables. Environment variables take precedence over the config file.

## Config File Location

TCS looks for configuration in the following order:

1. Path specified by the `TCS_CONFIG` environment variable
2. `/etc/tcs/config.toml` (default)

```bash
export TCS_CONFIG=/path/to/config.toml
```

## Environment Variables

All config keys are available as environment variables with the `TCS_` prefix and `_` separators:

```bash
# Equivalent to:
# [tls]
# mode = "self_signed"

export TCS_TLS_MODE=self_signed
```

## Server Configuration

```toml
[server]
bind_addr = "0.0.0.0"          # Host to bind on
advertised_url = ""            # External URL (used for Siderolink + cert SAN; defaults to https://localhost:443)
grpc_port = 8080               # Reserved (not bound) — outbound Talos uses node :50000
metrics_port = 9090            # Reserved (not bound)
```

TCS always listens on **:80 (HTTP → redirect to HTTPS / ACME challenges)** and
**:443 (HTTPS)**. There is no separate `http_port` listener. For non-root
development, override the bind ports with `TCS_HTTPS_PORT` (0 disables) and
`TCS_HTTP_PORT`.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `bind_addr` | string | `"0.0.0.0"` | Network interface to listen on |
| `advertised_url` | string | `""` | Public-facing URL (Siderolink endpoint + cert SAN); defaults to `https://localhost:443` |
| `grpc_port` | u16 | `8080` | **Reserved** — TCS has no inbound gRPC server; it dials Talos nodes' `:50000` |
| `metrics_port` | u16 | `9090` | **Reserved** — no metrics endpoint is currently served |

> TCS always binds **:443 (HTTPS)** and **:80 (HTTP → redirect/ACME)**. There is
> no `http_port` listener. Dev-only env overrides: `TCS_HTTPS_PORT` (0 disables)
> and `TCS_HTTP_PORT`.

## Database Configuration

```toml
[database]
backend = "sqlite"              # or "postgres" with postgres_url
sqlite_path = "/var/lib/tcs/data.db"
postgres_url = ""               # e.g., "postgresql://user:pass@host:5432/tcs"
max_connections = 10
connection_timeout = 30         # seconds
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"sqlite"` | `sqlite` or `postgres` (see POSTGRES.md) |
| `sqlite_path` | string | `"/var/lib/tcs/data.db"` | Path to SQLite database file |
| `postgres_url` | string | `""` | PostgreSQL connection URL |
| `max_connections` | u32 | `10` | Maximum connection pool size |
| `connection_timeout` | u64 | `30` | Connection timeout in seconds |

### PostgreSQL URL Format

```
postgresql://username:password@host:port/database?sslmode=require
```

## TLS Configuration

```toml
[tls]
enabled = false
mode = "disabled"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"

[tls.letsencrypt.dns_provider]
provider = "godaddy"
api_key = "..."
api_secret = "..."

[tls.provided]
cert_path = "/etc/tls/cert.pem"
key_path = "/etc/tls/key.pem"
```

See [TLS.md](./TLS.md) for detailed TLS and ACME configuration.

## Siderolink Configuration

Siderolink here is TCS's **WireGuard control-plane** side: it manages a host WG
interface (default `tcs-sl0`) and hands registering Talos nodes their tunnel
peer config (assigned CGNAT IP + server public key + `listen_port`). It is not a
gRPC tunnel server — nodes reach TCS over UDP `bind_port`. See
[SIDEROLINK.md](SIDEROLINK.md).

```toml
[siderolink]
bind_port = 8082               # UDP port nodes reach TCS on (advertised as listen_port)
listen_port = 443               # Advertised port for machines to connect to
mtu = 1420                      # Maximum transmission unit
subnet = "100.64.0.0/10"        # Carrier-grade NAT subnet for tunnel IPs
rate_limit_bytes = 0            # Rate limit in bytes/sec (0 = unlimited)
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `bind_port` | u16 | `8082` | Internal port for siderolink listener |
| `listen_port` | u16 | `443` | Port advertised to machines |
| `mtu` | u16 | `1420` | MTU for tunnel packets |
| `subnet` | string | `"100.64.0.0/10"` | CGNAT subnet for assigning machine IPs |
| `rate_limit_bytes` | u64 | `0` | Rate limit in bytes/second (0 = unlimited) |

## Branding Configuration

```toml
[branding]
name = "Talos Control System"
short_name = "TCS"
tagline = "Kubernetes Management Simplified"
primary_color = "#150D6A"
secondary_color = "#4F8BFF"
background_color = "#0A0A0A"
surface_color = "#1A1A1A"
text_color = "#FFFFFF"
text_muted_color = "#A0A0A0"
font_family = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif"
logo_path = ""                  # Custom logo path (overrides built-in)
favicon_path = ""               # Custom favicon path
docs_url = ""                   # External documentation link
support_url = ""                # External support link
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | `"Talos Control System"` | Full platform name |
| `short_name` | string | `"TCS"` | Short name for top bar |
| `tagline` | string | `"Kubernetes Management Simplified"` | Tagline displayed on login |
| `primary_color` | string | `"#150D6A"` | Primary brand color |
| `secondary_color` | string | `"#4F8BFF"` | Secondary/accent color |
| `background_color` | string | `"#0A0A0A"` | Page background |
| `surface_color` | string | `"#1A1A1A"` | Card/panel background |
| `text_color` | string | `"#FFFFFF"` | Primary text color |
| `text_muted_color` | string | `"#A0A0A0"` | Secondary text color |
| `font_family` | string | system font stack | CSS font-family value |
| `logo_path` | string | `""` | Path to custom logo file |
| `favicon_path` | string | `""` | Path to custom favicon |
| `docs_url` | string | `""` | External docs link (empty = hidden) |
| `support_url` | string | `""` | External support link (empty = hidden) |

## Bare-metal (metal) Configuration

Full DHCP + PXE + BMC path for provisioning bare metal. **Off by default** —
enable it on a dedicated provisioning VLAN only. See [METAL.md](METAL.md) for the
operator workflow and host requirements. A live overlay can also be written to
`/var/lib/tcs/metal.toml` from the Settings UI without a process restart.

```toml
[metal]
enabled = false                # master switch for DHCP/PXE/BMC runtimes

[metal.dhcp]
enabled = false
interface = "eth1"             # REQUIRED when enabled — dedicated provision NIC
bind_ip = ""                   # optional; else the interface's primary address
subnet = "10.88.0.0/24"
range_start = "10.88.0.100"
range_end = "10.88.0.200"
gateway = "10.88.0.1"
dns = ["10.88.0.1"]
lease_ttl_secs = 3600
allow_unknown = false          # only inventory MACs get leases

[metal.pxe]
enabled = false
http_port = 6969               # iPXE + asset HTTP server
tftp_enabled = false           # serve chainloaders (undionly.kpxe/snponly.efi) over TFTP
asset_dir = "/var/lib/tcs/pxe"
default_talos_version = "v1.13.7"
mirror_base = "https://github.com/siderolabs/talos/releases/download"
extra_cmdline = ""             # extra kernel cmdline (console, earlycon, …)
ipxe_bios_file = "undionly.kpxe"
ipxe_uefi_file = "snponly.efi"

[metal.bmc]
connect_timeout_secs = 15
prefer_redfish = true          # Redfish first, IPMI (ipmitool) fallback
ipmi_interface = "lanplus"
```

## Image Factory Configuration

TCS uses the [Talos Image Factory](https://factory.talos.dev) to build installer
images that bundle the cluster's module set (system extensions). Point these at a
self-hosted factory if the public one is unavailable.

```toml
[factory]
base_url = "https://factory.talos.dev"  # factory API base
registry = "factory.talos.dev"          # OCI registry host for installer images
```

## Full Example

```toml
[server]
bind_addr = "0.0.0.0"
advertised_url = "https://tcs.example.com"
grpc_port = 8080
metrics_port = 9090

[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"

[auth]
# REQUIRED in production; prefer /etc/tcs/env TCS_AUTH_JWT_SECRET (see OPS.md)
jwt_secret = "a-long-random-string"

[tls]
enabled = true
mode = "letsencrypt"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"

[siderolink]
bind_port = 8082
listen_port = 443
mtu = 1420
subnet = "100.64.0.0/10"

[factory]
base_url = "https://factory.talos.dev"
registry = "factory.talos.dev"

# Bare-metal is off by default; see METAL.md
[metal]
enabled = false

[branding]
name = "Acme Kubernetes"
short_name = "Acme K8s"
tagline = "Managed Kubernetes by Acme Corp"
primary_color = "#2563EB"
secondary_color = "#60A5FA"
background_color = "#0F172A"
surface_color = "#1E293B"
text_color = "#F8FAFC"
text_muted_color = "#94A3B8"
docs_url = "https://docs.acme.example.com/kubernetes"
support_url = "https://support.acme.example.com"
```
