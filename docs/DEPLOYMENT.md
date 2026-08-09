# Kubernetes / Host Deployment

> **Alpha notes:** SQLite only; Siderolink not implemented; prefer binary install. See [STATUS.md](STATUS.md) and [TALOS.md](TALOS.md).

Guide

This guide covers deploying TCS as a standalone systemd service with production-grade configuration.

## Architecture

TCS runs as a single binary on a Linux host. It does not run inside Kubernetes — it manages Kubernetes clusters from the outside via Talos API and kubeconfig.

```
┌─────────────────────────────────────────────┐
│  Your Server (systemd)                      │
│  ┌─────────────────────────────────────┐    │
│  │  tcs (single binary)                │    │
│  │  ├─ REST API + Web UI  :8081        │    │
│  │  ├─ gRPC (Talos API) :8080          │    │
│  │  ├─ Siderolink       :8082          │    │
│  │  └─ Metrics          :9090          │    │
│  └─────────────────────────────────────┘    │
│                           ↕ Talos API        │
└─────────────────────────────────────────────┘
                ↕ kubeconfig
┌─────────────────────────────────────────────┐
│  Your Kubernetes Cluster                    │
│  (Talos Linux nodes)                        │
└─────────────────────────────────────────────┘
```

## Prerequisites

- Linux server (Ubuntu 22.04+, Debian 12+, or similar)
- Network access to your Talos cluster's control plane
- Talos cluster v1.7+ with accessible kubeconfig

## Quick Start

### 1. Install via self-extracting script

```bash
# Download the installer for your architecture
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.1.0/tcs-installer-linux-amd64.sh -o tcs-install.sh
chmod +x tcs-install.sh

# Install (creates systemd unit, config, and data directories)
sudo ./tcs-install.sh
```

### 2. Configure

Edit `/etc/tcs/config.toml`:

```toml
[server]
bind_addr = "0.0.0.0"
advertised_url = "https://tcs.example.com"
http_port = 8081

[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"
```

### 3. Start

```bash
sudo systemctl enable --now tcs
sudo journalctl -u tcs -f
```

### 4. Get admin password

On first boot, TCS creates a default admin user and logs the password:

```bash
sudo journalctl -u tcs | grep "password:"
# Output: Created default admin user: admin@tcs.local with password: abc123
```

Login at `http://your-server:8081` with `admin@tcs.local` and the displayed password.

## Production Configuration

### Database

For production, consider using PostgreSQL instead of SQLite:

```toml
[database]
backend = "postgres"
postgres_url = "postgresql://tcs:strong_password@localhost:5432/tcs"
max_connections = 20
```

### TLS with Let's Encrypt

See [TLS.md](./TLS.md) for detailed configuration. Quick example:

```toml
[tls]
enabled = true
mode = "letsencrypt"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"
```

Ensure port 80 is accessible from the internet for ACME validation.

### Importing a Cluster

After logging in, import your cluster via the API:

```bash
TOKEN=$(curl -s -X POST http://localhost:8081/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@tcs.local","password":"YOUR_PASSWORD"}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")

curl -s -X POST http://localhost:8081/api/clusters/import \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "{
    \"name\": \"production\",
    \"kubeconfig\": \"$(cat /path/to/kubeconfig | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))')\"
  }"
```

## Systemd Unit

The installer creates `/etc/systemd/system/tcs.service`:

```ini
[Unit]
Description=Talos Control System
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/tcs
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

## Backup and Restore

### Database Backup

```bash
# SQLite
sudo cp /var/lib/tcs/data.db /backup/data.db.$(date +%F).bak

# Verify backup
sqlite3 /backup/data.db.$(date +%F).bak ".tables"
```

### Restore

```bash
# Stop TCS
sudo systemctl stop tcs

# Restore database
sudo cp /backup/data.db.2024-01-01.bak /var/lib/tcs/data.db
sudo chown root:root /var/lib/tcs/data.db

# Start TCS
sudo systemctl start tcs
```

## Monitoring

TCS exposes Prometheus metrics on port 9090:

```bash
curl http://localhost:9090/metrics
```

Key metrics:
- `tcs_clusters_total` — Total managed clusters
- `tcs_machines_total` — Total registered machines
- `tcs_machines_by_status` — Machine count by status
- `tcs_http_requests_total` — API request counter
- `tcs_http_request_duration_seconds` — Request latency histogram

## Upgrading

### Via installer

```bash
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.2.0/tcs-installer-linux-amd64.sh -o tcs-install.sh
chmod +x tcs-install.sh
sudo ./tcs-install.sh
```

### Manual

```bash
# Download new binary
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.2.0/tcs-linux-amd64 -o /usr/local/bin/tcs
chmod +x /usr/local/bin/tcs

# Restart
sudo systemctl restart tcs
```

TCS runs database migrations automatically on startup. Review the changelog for breaking changes across major versions.

## Resource Recommendations

| Deployment Size | CPU | Memory |
|----------------|-----|--------|
| Small (< 5 clusters) | 1 vCPU | 512Mi |
| Medium (5-20 clusters) | 2 vCPU | 2Gi |
| Large (20+ clusters) | 4 vCPU | 4Gi |

## Troubleshooting

### Check service status

```bash
sudo systemctl status tcs
sudo journalctl -u tcs --no-pager -f
```

### Verify API

```bash
curl -s http://localhost:8081/api/health
# Expected: {"status":"ok","version":"0.1.0"}
```

### Database migration errors

TCS tracks applied migrations in a `_tcs_migrations` table. Migrations are only applied once. If you encounter migration errors, check the journal:

```bash
sudo journalctl -u tcs | grep -i migrat
```

### Siderolink connectivity

Ensure port 8082 is accessible from your Talos machines:

```bash
# Test from a Talos node
nc -zv tcs.example.com 8082
```
