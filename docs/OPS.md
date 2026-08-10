# Operations notes (host deploy)

TCS runs on a management host under systemd. It is not an in-cluster workload.

## JWT / secrets hygiene

| Source | Purpose |
|--------|---------|
| `/etc/tcs/env` → `TCS_AUTH_JWT_SECRET` | Preferred production secret (mode 0600) |
| `/etc/tcs/config.toml` → `auth.jwt_secret` | Optional; avoid duplicating secrets here |

Recommended:

1. Put the only secret in `/etc/tcs/env` (installer generates one).
2. Remove or comment out `jwt_secret` from `config.toml` so the env var is authoritative.
3. Never set `TCS_ALLOW_INSECURE=1` outside a lab.

If you change the JWT secret, encrypted kubeconfig/talosconfig blobs become unreadable
and must be re-imported.

## Admin recovery (lab)

- Email: `admin@tcs.local` (default first user)
- Password: printed once on first boot:  
  `journalctl -u tcs \| grep -i 'Default admin password'`
- Or set `TCS_DEFAULT_ADMIN_PASSWORD` in `/etc/tcs/env` **before** first start (new DB only)

Do **not** commit lab passwords into git.

## TLS

| Mode | When |
|------|------|
| `self_signed` | Lab / air-gapped |
| `letsencrypt` | Public hostname + ports 80/443 (watch LE rate limits) |
| `provided` | You manage cert/key files |

Lab FTC currently uses **self-signed** for `devstation.jakelab.info`. Prefer that until LE rate limits clear.

## Backups

- Etcd snapshots: UI **Cluster → Backups** or API
- TCS state: back up `/var/lib/tcs/data.db` and `/var/lib/tcs/backups/`

## Upgrades

```bash
sudo ./tcs-VERSION-linux-x86_64.sh --upgrade
# keeps config.toml, env, and SQLite data
```

## Health

```bash
curl -sk https://localhost/api/health
# {"status":"ok","version":"...","commit":"...","buildTime":"..."}
```

## Per-cluster access (RBAC)

Global roles: `admin` | `operator` | `reader`.

Optional **memberships** limit non-admin users to specific clusters:

```http
GET    /api/clusters/{id}/access          # admin
PUT    /api/clusters/{id}/access          # body: { "userId": "...", "role": "operator" }
DELETE /api/clusters/{id}/access/{userId}
```

Rules:

- Global **admin** always sees every cluster.
- User with **zero** membership rows → legacy behaviour (global role on all clusters).
- User with **one or more** memberships → only those clusters, using membership role.
