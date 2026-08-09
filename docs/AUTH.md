# Authentication

TCS supports **local passwords**, **LDAP/Active Directory**, and **OIDC**.
**SAML is not available** in alpha (login button is disabled).

## JWT secret (required in production)

```bash
export TCS_AUTH_JWT_SECRET="$(openssl rand -hex 32)"
# installer writes this into /etc/tcs/env automatically
```

Or set `auth.jwt_secret` in config. Starting with the built-in default secret is
**refused** unless `TCS_ALLOW_INSECURE=1` (local lab only).

The JWT secret also derives the key used to **encrypt talosconfig/kubeconfig** at rest.
Changing it makes previously stored secrets unreadable.

## Auth providers

### Local (default)

- Passwords hashed with argon2id
- JWT issued on `POST /api/auth/login`
- Default admin `admin@tcs.local` on first boot
- Random password printed to logs unless `TCS_DEFAULT_ADMIN_PASSWORD` is set
- `passwordNeedsChange` forced until password is changed

### LDAP / Active Directory

Flow:

1. Connect to `url` (`ldap://` or `ldaps://`)
2. Optional **service bind** (`bind_dn` / `bind_password`) for directory search
3. Search `user_search_base` with `user_search_filter` (`{0}` = username)
4. Simple-bind as the user DN with the provided password
5. Map `memberOf` groups → role; upsert local user row

Login accepts **email or username**. For emails, `{0}` is the local-part (`alice` from `alice@corp`).

First-time LDAP users are **auto-provisioned** when no local row exists (if LDAP is configured).

```toml
[auth.ldap]
url = "ldaps://ad.example.com:636"
bind_dn = "CN=tcs-svc,OU=Service Accounts,DC=example,DC=com"
bind_password = "redacted"
user_search_base = "OU=Users,DC=example,DC=com"
user_search_filter = "(sAMAccountName={0})"
default_role = "reader"

[[auth.ldap.group_role_mappings]]
group_dn_pattern = "CN=TCS-Admins*"
role = "admin"

[[auth.ldap.group_role_mappings]]
group_dn_pattern = "CN=TCS-Operators*"
role = "operator"
```

Group patterns are case-insensitive. A single `*` wildcard is supported.

**Verification checklist (real AD/LDAP):**

1. `ldapsearch` with the same URL, bind DN, base, and filter succeeds
2. User bind with the same credentials succeeds
3. Configure TCS, restart, log in with a directory user
4. Confirm role from group mapping in **Settings → Users**

### OIDC

- Authorization code flow via discovery (`/.well-known/openid-configuration`)
- CSRF `state` is stored in-process (single-instance; multi-node needs sticky sessions or shared store)
- Query parameters are URL-encoded
- Users auto-provisioned on first login with `default_role` (default `reader`)
- Password login is rejected for existing `oidc` users (use the OIDC button)

```toml
[auth.oidc]
enabled = true
issuer_url = "https://login.example.com/"
client_id = "tcs"
client_secret = "redacted"
redirect_url = "https://tcs.example.com/api/auth/oidc/callback"
scopes = ["openid", "email", "profile"]
default_role = "reader"
```

**IdP app settings:**

- Redirect URI must match `redirect_url` exactly
- Grant type: authorization code
- Scopes: openid + email (required for account email)

**Verification checklist (real IdP):**

1. Discovery URL returns JSON with `authorization_endpoint` and `token_endpoint`
2. Browser hit `GET /api/auth/oidc` redirects to IdP
3. Callback returns JWT + user; subsequent `/api/auth/me` works
4. Replay of `state` fails (CSRF)

**Known alpha limits:**

- ID tokens are not fully JWKS-signature-validated (userinfo / claims used after code exchange)
- OIDC `state` is in-memory (lost on restart; not multi-replica)

## Roles (RBAC)

| Role | Permissions |
|------|-------------|
| `admin` | Full access: users, clusters, settings |
| `operator` | Manage clusters and nodes |
| `reader` | Read-only |

Route-level only; **no per-cluster scopes** yet.

## API endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/auth/login` | Local or LDAP password login |
| POST | `/api/auth/logout` | Client-side token discard |
| GET | `/api/auth/oidc` | Start OIDC (redirect) |
| GET | `/api/auth/oidc/callback` | OIDC code exchange |
| POST | `/api/auth/password` | Change local password |
| GET | `/api/auth/me` | Current user |
