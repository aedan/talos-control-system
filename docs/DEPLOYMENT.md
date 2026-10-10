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

1. First start moves the installer-generated JWT from `/etc/tcs/env` into the database vault. Record the break-glass passphrase from the journal.
2. `server.advertised_url` matches the URL operators use
3. TLS mode (`self_signed` for lab, `letsencrypt` / `provided` for prod) — see [TLS.md](TLS.md)
4. Firewall: UI ports only as needed; TCS must egress to node :50000
5. Backups of `/var/lib/tcs/data.db`, `/var/lib/tcs/vault.hostkey`, and `/var/lib/tcs/backups`. Keep the break-glass passphrase offline, not in the backup.
6. Optional LDAP/OIDC — client secret and LDAP bind password go in with `tcs vault put`. See [AUTH.md](AUTH.md).

## Upgrades

Re-run a newer self-extracting installer (keeps config + DB) or replace
`/usr/local/bin/tcs` and `systemctl restart tcs`.

## Smoke test

After deploy: [SMOKE.md](SMOKE.md).
