# Installation Guide

TCS is a **host-local** control plane for Talos Linux. It ships as a single binary
with the UI embedded. It is **not** installed with Helm and is **not** meant to
run inside the managed Kubernetes cluster.

## Prerequisites

- Linux host with **systemd** (Ubuntu 22.04+, Debian 12+, RHEL 9+, etc.)
- Network reachability to Talos node APIs (**TCP 50000**) and the Kubernetes API (for import)
- Root (or equivalent) for install

## Option 1: Self-extracting installer (recommended)

Release assets include a self-extracting shell script that installs the binary,
writes a systemd unit, creates `/etc/tcs` + `/var/lib/tcs`, and generates a JWT secret.

### Download

```bash
# Replace OWNER/REPO and VERSION with your release
# Example: aedan/talos-control-system / v0.1.0
curl -fsSL -o tcs-install.sh \
  "https://github.com/OWNER/REPO/releases/download/v0.1.0/tcs-0.1.0-linux-x86_64.sh"
chmod +x tcs-install.sh
```

### Install

```bash
sudo ./tcs-install.sh
```

Options:

```bash
sudo ./tcs-install.sh --no-start   # files only
sudo ./tcs-install.sh --upgrade    # stop, replace binary, keep config, restart
```

Creates / updates:

| Path | Purpose |
|------|---------|
| `/usr/local/bin/tcs` | Binary |
| `/etc/tcs/config.toml` | Config (created once; upgrades leave it alone) |
| `/etc/tcs/env` | Secrets (`TCS_AUTH_JWT_SECRET`, optional lab flags) |
| `/etc/systemd/system/tcs.service` | systemd unit |
| `/var/lib/tcs/` | SQLite DB, certs, etcd backups |

### Configure

```bash
sudoedit /etc/tcs/config.toml
# Set at least:
#   [server] advertised_url = "https://tcs.example.com"
#   [auth] jwt_secret  (or rely on /etc/tcs/env TCS_AUTH_JWT_SECRET)
```

### First login

```bash
sudo journalctl -u tcs | grep -i 'Default admin password'
# Login: admin@tcs.local + that password
```

UI defaults to HTTP port **8081** (or 80/443 if you enable TLS in config).

## Option 2: Binary + tar package

```bash
curl -fsSL -o tcs.tgz \
  "https://github.com/OWNER/REPO/releases/download/v0.1.0/tcs-0.1.0-linux-x86_64.tar.gz"
sudo tar xzf tcs.tgz -C /tmp
sudo install -m 755 /tmp/tcs /usr/local/bin/tcs
# Then copy config.example.toml and create a systemd unit
# (or re-run the self-extracting installer for unit + dirs).
```

## Option 3: Build from source

```bash
git clone https://github.com/OWNER/REPO.git
cd talos-control-system
cd frontend && npm ci && npm run build && cd ..
export GIT_HASH="$(git rev-parse --short=12 HEAD)"
export BUILD_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cd backend && cargo build --release
./scripts/package-installer.sh \
  --binary target/release/talos-control-system \
  --version 0.1.0-dev \
  --arch "$(uname -m)"
sudo ./dist/tcs-0.1.0-dev-linux-*.sh
```

## Upgrades

```bash
# Prefer installer --upgrade (keeps /etc/tcs/config.toml and data.db)
sudo ./tcs-NEW-linux-x86_64.sh --upgrade
```

Or replace only the binary:

```bash
sudo systemctl stop tcs
sudo install -m 755 ./tcs /usr/local/bin/tcs
sudo systemctl start tcs
```

## Uninstall

```bash
sudo systemctl disable --now tcs
sudo rm -f /usr/local/bin/tcs /etc/systemd/system/tcs.service
sudo systemctl daemon-reload
# Optional: remove data (destructive)
# sudo rm -rf /etc/tcs /var/lib/tcs
```

## Not supported

- **Helm / in-cluster deployment** — TCS is the out-of-band manager for Talos; it should run on a deployer / bastion / management host.
- **Docker-only production** — use the binary + systemd path above.

See also [CONFIGURATION.md](CONFIGURATION.md), [AUTH.md](AUTH.md), [TLS.md](TLS.md), [SMOKE.md](SMOKE.md).
