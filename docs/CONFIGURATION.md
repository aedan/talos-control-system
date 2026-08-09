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
# [server]
# http_port = 9090

export TCS_SERVER_HTTP_PORT=9090
```

## Server Configuration

```toml
[server]
bind_addr = "0.0.0.0"          # Host to bind on
advertised_url = ""            # External URL (auto-set from bind + port if empty)
grpc_port = 8080               # gRPC server port (Talos API, machine connections)
http_port = 8081               # REST API + Web UI port
metrics_port = 9090            # Prometheus metrics endpoint port
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `bind_addr` | string | `"0.0.0.0"` | Network interface to listen on |
| `advertised_url` | string | `""` | Public-facing URL for redirects and links |
| `grpc_port` | u16 | `8080` | gRPC port for Talos API and siderolink |
| `http_port` | u16 | `8081` | HTTP port for REST API and web UI |
| `metrics_port` | u16 | `9090` | Prometheus metrics endpoint |

## Database Configuration

```toml
[database]
backend = "sqlite"              # alpha: sqlite only (postgres refuses to start)
sqlite_path = "/var/lib/tcs/data.db"
postgres_url = ""               # e.g., "postgresql://user:pass@host:5432/tcs"
max_connections = 10
connection_timeout = 30         # seconds
```

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `backend` | string | `"sqlite"` | Database backend: `sqlite` or `postgres` |
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

Siderolink provides encrypted tunnel connectivity between Talos machines and TCS.

```toml
[siderolink]
bind_port = 8082               # Port TCS accepts siderolink connections on
listen_port = 443               # Advertised port machines should connect to
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

## Full Example

```toml
[server]
bind_addr = "0.0.0.0"
advertised_url = "https://tcs.example.com"
grpc_port = 8080
http_port = 8081
metrics_port = 9090

[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"

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
