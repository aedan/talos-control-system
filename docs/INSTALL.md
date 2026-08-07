# Installation Guide

## Prerequisites

- **Docker** 24+ and **Docker Compose** v2 (for standalone deployment)
- **Kubernetes** 1.28+ and **Helm** 3.12+ (for K8s deployment)
- **Talos Linux** 1.7+ on managed machines

## Option 1: Docker Deployment

### 1. Clone the repository

```bash
git clone https://github.com/siderolabs/talos-control-system.git
cd talos-control-system
```

### 2. Configure

Copy the example configuration and adjust settings:

```bash
cp config.example.toml config.toml
```

At minimum, set:
- `server.http_port` — The port TCS will listen on
- `database.sqlite_path` — Path for the SQLite database

### 3. Start with Docker Compose

```bash
docker compose up -d
```

TCS will be available at `http://localhost:8081`.

### 4. Configure siderolink

TCS exposes port 8082 for siderolink tunnel connections. Configure your Talos machines with:

```
--siderolink.server=tcs.example.com
--siderolink.token=<your-token>
```

## Option 2: Kubernetes Deployment

### 1. Add the Helm repository

```bash
helm repo add tcs https://charts.talos.dev
helm repo update
```

### 2. Install with default values

```bash
helm install tcs tcs/tcs -n tcs --create-namespace
```

### 3. Install with custom values

```bash
helm install tcs tcs/tcs -n tcs --create-namespace \
  --set ingress.enabled=true \
  --set ingress.hosts[0].host=tcs.example.com \
  --set database.backend=postgres \
  --set database.postgresUrl="postgresql://user:pass@postgres-host:5432/tcs"
```

### 4. Expose siderolink

For machine connectivity, forward the siderolink port:

```bash
kubectl port-forward svc/tcs-tcs 8082:8082 -n tcs
```

Or configure a `NodePort` / `LoadBalancer` service:

```yaml
# values-custom.yaml
service:
  type: LoadBalancer
siderolink:
  listenPort: 443
```

## Option 3: Bare Metal / Systemd

### 1. Build the binary

```bash
cargo build --release
cp target/release/talos-control-system /usr/local/bin/tcs
```

### 2. Create config

```bash
cp config.example.toml /etc/tcs/config.toml
```

### 3. Create systemd unit

```ini
[Unit]
Description=Talos Control System
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/tcs --config /etc/tcs/config.toml
Restart=on-failure
User=tcs
Group=tcs

[Install]
WantedBy=multi-user.target
```

### 4. Enable and start

```bash
systemctl enable --now tcs
```

## Post-Installation

### Initial Admin User

The first admin account can be created via the API:

```bash
curl -X POST http://localhost:8081/api/auth/setup \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"strong-password"}'
```

### Configure OIDC (Optional)

```bash
curl -X PUT http://localhost:8081/api/auth/oidc \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <admin-token>" \
  -d '{
    "issuer": "https://accounts.google.com",
    "client_id": "YOUR_CLIENT_ID",
    "client_secret": "YOUR_CLIENT_SECRET"
  }'
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

## Troubleshooting

### TCS fails to start

Check logs:

```bash
# Docker
docker compose logs tcs

# Kubernetes
kubectl logs -n tcs deployment/tcs-tcs

# Systemd
journalctl -u tcs --no-pager -f
```

### Machines can't connect via siderolink

Ensure port 8082 is accessible and not blocked by a firewall:

```bash
# Test connectivity
nc -zv tcs.example.com 8082

# Check TCS logs for siderolink messages
grep -i siderolink /var/log/tcs.log
```

### Database migration errors

If you get migration errors after upgrading, ensure you haven't skipped versions. TCS applies migrations automatically on startup.

For manual migration review:

```bash
ls backend/migrations/
cat backend/migrations/001_initial.sql
```
