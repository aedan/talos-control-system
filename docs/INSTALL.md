# Installation Guide

TCS is distributed as a single statically-linked binary with the frontend embedded. No Docker, Kubernetes, or Helm required.

## Prerequisites

- **Linux server** (Ubuntu 22.04+, Debian 12+, or similar)
- **Talos Linux** 1.7+ on managed clusters

## Option 1: Self-Extracting Installer (Recommended)

The installer downloads the binary, creates a systemd unit, sets up directories, and writes a default config.

### 1. Download the installer

```bash
# x86_64
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.1.0/tcs-installer-linux-amd64.sh -o tcs-install.sh

# ARM64
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.1.0/tcs-installer-linux-arm64.sh -o tcs-install.sh
```

### 2. Run the installer

```bash
chmod +x tcs-install.sh
sudo ./tcs-install.sh
```

This creates:
- `/usr/local/bin/tcs` — The TCS binary
- `/etc/tcs/config.toml` — Default configuration
- `/etc/systemd/system/tcs.service` — Systemd unit
- `/var/lib/tcs/` — Data directory (SQLite database)

### 3. Configure

Edit `/etc/tcs/config.toml` at minimum:

```toml
[server]
advertised_url = "https://tcs.example.com"
```

### 4. Start

```bash
sudo systemctl enable --now tcs
```

TCS will be available at `http://localhost:8081`.

### 5. Get admin credentials

On first boot, TCS creates a default admin user and logs the password:

```bash
sudo journalctl -u tcs | grep "password:"
# Created default admin user: admin@tcs.local with password: abc123
```

## Option 2: Manual Binary Install

### 1. Download the binary

```bash
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.1.0/tcs-linux-amd64 -o /usr/local/bin/tcs
chmod +x /usr/local/bin/tcs
```

### 2. Create directories

```bash
sudo mkdir -p /etc/tcs /var/lib/tcs
```

### 3. Create config

```bash
sudo tee /etc/tcs/config.toml > /dev/null << 'EOF'
[server]
bind_addr = "0.0.0.0"
http_port = 8081
grpc_port = 8080

[database]
backend = "sqlite"
sqlite_path = "/var/lib/tcs/data.db"
EOF
```

### 4. Create systemd unit

```bash
sudo tee /etc/systemd/system/tcs.service > /dev/null << 'EOF'
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
EOF
```

### 5. Enable and start

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now tcs
```

## Post-Installation

### Import a Cluster

After logging in, import your Talos cluster:

```bash
TOKEN=$(curl -s -X POST http://localhost:8081/api/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@tcs.local","password":"YOUR_PASSWORD"}' \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])")

curl -s -X POST http://localhost:8081/api/clusters/import \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "$(python3 -c "
import json, sys
print(json.dumps({
    'name': 'my-cluster',
    'kubeconfig': open('/root/.kube/config').read()
}))")"
```

### Configure TLS (Let's Encrypt)

Update `/etc/tcs/config.toml`:

```toml
[tls]
enabled = true
mode = "letsencrypt"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"
```

Then restart:

```bash
sudo systemctl restart tcs
```

### Configure Siderolink

TCS exposes port 8082 for siderolink tunnel connections. Configure your Talos machines with:

```
--siderolink.server=tcs.example.com
--siderolink.token=<your-token>
```

### Configure Branding

Use the `/settings/branding` page in the UI, or set branding in `config.toml`:

```toml
[branding]
name = "My Platform"
short_name = "MP"
tagline = "Our Kubernetes Platform"
primary_color = "#2563EB"
```

## Upgrading

### Via installer

```bash
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.2.0/tcs-installer-linux-amd64.sh -o tcs-install.sh
chmod +x tcs-install.sh
sudo ./tcs-install.sh
```

### Manual

```bash
curl -sL https://github.com/siderolabs/talos-control-system/releases/download/v0.2.0/tcs-linux-amd64 -o /usr/local/bin/tcs
chmod +x /usr/local/bin/tcs
sudo systemctl restart tcs
```

## Troubleshooting

### TCS fails to start

Check logs:

```bash
sudo journalctl -u tcs --no-pager -f
```

### Check service status

```bash
sudo systemctl status tcs
```

### Verify API

```bash
curl -s http://localhost:8081/api/health
# Expected: {"status":"ok","version":"0.1.0"}
```

### Machines can't connect via siderolink

Ensure port 8082 is accessible and not blocked by a firewall:

```bash
# Test connectivity
nc -zv tcs.example.com 8082
```

### Database migration errors

TCS tracks applied migrations and only applies new ones. If you encounter persistent migration errors, the database may need to be reset (warning: this deletes all data):

```bash
sudo systemctl stop tcs
sudo rm /var/lib/tcs/data.db
sudo systemctl start tcs
```
