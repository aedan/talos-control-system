# Operations notes (host deploy)

TCS runs on a management host under systemd. It is not an in-cluster workload.

## Secrets vault

Secrets are ciphertext in the database. The host key `/var/lib/tcs/vault.hostkey` (mode 0600) unlocks them at startup. A break-glass passphrase is printed once on first start (`journalctl -u tcs | grep 'Break-glass passphrase'`). TCS stores only an Argon2id hash of that passphrase.

| Item | Where it lives |
|------|----------------|
| JWT, OIDC client secret, LDAP bind password, DNS API credentials | Database vault |
| Host key that opens the vault | `/var/lib/tcs/vault.hostkey` |
| Break-glass passphrase | Nowhere after first start. Copy it offline. |
| Postgres URL | Config. Required to open the database. |

The first start copies a bootstrapped `TCS_AUTH_JWT_SECRET` or `auth.jwt_secret` into the vault and comments the cleartext line out. Store anything added later with `tcs vault put <name>` (stdin). There is no command that prints a secret back.

If the host key is lost:

```bash
tcs vault recover
```

That prompts for the break-glass passphrase, writes a new host key, and does not print stored secrets.

Back up `data.db` and `vault.hostkey` together. Keep the passphrase separate from that backup. Never set `TCS_ALLOW_INSECURE=1` outside a lab.

The vault data key stays the key derived from the JWT secret that existed when the vault was created. Cluster ciphertext keeps decrypting if the signing secret is replaced later.

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

For an internal/air-gapped lab, **self-signed** is the simplest path. Reach out to your security team if you need CA-signed certs instead; `provided` mode works with any PEM pair.

## Backups

- Etcd snapshots: UI **Cluster → Backups** or API
- TCS state: back up `/var/lib/tcs/data.db` and `/var/lib/tcs/backups/`

## Upgrades

```bash
sudo ./tcs-VERSION-linux-x86_64.sh --upgrade
# keeps config.toml, env, and SQLite data
```

### Release policy (do not blind-rev)

- **Production / FTC / shared labs:** deploy only **tagged GitHub releases**  
  (`vX.Y.Z` → installer `tcs-X.Y.Z-linux-x86_64.sh`).
- **Do not** install main-branch `tcs-0.1.0-dev-*.sh` artifacts onto shared hosts.  
  Those are CI build outputs; the UI may still report an old `CARGO_PKG_VERSION` and the
  commit will not match a release tag — that confuses operators and support.
- Before deploy: bump `backend/Cargo.toml` version, update `CHANGELOG.md` / `docs/STATUS.md`,
  commit, tag `vX.Y.Z`, wait for **Build & Release**, then upgrade from the release asset.
- After deploy, verify:

```bash
curl -sk https://localhost/api/health
# version and commit must match the intended tag (e.g. 0.4.0 + tag commit)
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
