# Host deployment

TCS is deployed as a **standalone systemd service on a Linux host**. It does **not**
run inside Kubernetes and there is **no Helm chart**.

Architecture:

```
┌─────────────────────────────────────────────┐
│  Management host (systemd)                  │
│  tcs binary + /etc/tcs + /var/lib/tcs       │
│  REST/UI (and optional TLS on 80/443)       │
└─────────────────────────────────────────────┘
         │ Talos gRPC mTLS :50000
         │ Kubernetes API (import)
         ▼
┌─────────────────────────────────────────────┐
│  Talos Linux cluster(s)                     │
└─────────────────────────────────────────────┘
```

## Quick install

See [INSTALL.md](INSTALL.md) for the self-extracting installer.

```bash
sudo ./tcs-VERSION-linux-x86_64.sh
sudoedit /etc/tcs/config.toml   # advertised_url, TLS, auth
sudo journalctl -u tcs -f
```

## Production checklist

1. Strong `TCS_AUTH_JWT_SECRET` (installer generates one in `/etc/tcs/env`)
2. `server.advertised_url` matches the URL operators use
3. TLS mode (`self_signed` for lab, `letsencrypt` / `provided` for prod) — see [TLS.md](TLS.md)
4. Firewall: UI ports only as needed; TCS must egress to node :50000
5. Backups of `/var/lib/tcs/data.db` and `/var/lib/tcs/backups`
6. Optional LDAP/OIDC — see [AUTH.md](AUTH.md)

## Upgrades

Re-run a newer self-extracting installer (keeps config + DB) or replace
`/usr/local/bin/tcs` and `systemctl restart tcs`.

## Smoke test

After deploy: [SMOKE.md](SMOKE.md).
