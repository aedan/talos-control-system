# Authentication

TCS supports **local passwords**, **LDAP/Active Directory**, **OIDC**, and **SAML 2.0 SP** (alpha).
The login page discovers enabled SSO via `GET /api/auth/providers`.

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
- CSRF `state` is **DB-backed** (`oidc_states` table), supporting multi-replica deployments
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

**ID token verification:** When the IdP publishes `jwks_uri`, TCS verifies the
`id_token` signature (RS256) and checks issuer + audience before accepting claims.
If JWKS verification fails, TCS falls back to userinfo / unverified claims (logged).

**Known alpha limits:**

- OIDC `state` is DB-backed (`oidc_states` table), supporting multi-replica deployments
- Only RSA JWKs (`n`/`e`) are supported for ID token verify
- Browser callbacks inject the JWT into `localStorage` via a small HTML page

### SAML 2.0 (Service Provider)

Alpha SP: builds HTTP-Redirect AuthnRequest, serves SP metadata, accepts HTTP-POST ACS,
parses NameID / attributes, maps groups → roles, auto-provisions local users.

```toml
[auth.saml]
enabled = true
idp_metadata_url = "https://idp.example.com/realms/tcs/protocol/saml/descriptor"
# or set idp_sso_url + optional idp_cert_pem
sp_entity_id = "https://tcs.example.com/saml/sp"
acs_url = "https://tcs.example.com/api/auth/saml/acs"
attribute_email = "email"
attribute_name = "displayName"
attribute_groups = "groups"
default_role = "reader"

[[auth.saml.group_role_mappings]]
group_pattern = "tcs-admins"
role = "admin"
```

| Endpoint | Purpose |
|----------|---------|
| `GET /api/auth/saml/metadata` | SP metadata XML |
| `GET /api/auth/saml/login` | Redirect to IdP |
| `POST /api/auth/saml/acs` | Assertion consumer (HTML → JWT in localStorage) |

**Known alpha limits:**

- Full XML digital signature verification is best-effort (prefer TLS + trusted network path; validate with your IdP before production)
- Attribute extraction is string/XML naive (works with common IdP shapes)

## Roles (RBAC)

| Role | Permissions |
|------|-------------|
| `admin` | Full access: users, clusters, settings, memberships |
| `operator` | Manage clusters and nodes (no user admin) |
| `reader` | Read-only |

### Per-cluster memberships

Optional table `cluster_access` scopes non-admin users to specific clusters.

| Condition | Behaviour |
|-----------|-----------|
| Global `admin` | All clusters |
| No membership rows | Global role applies to **all** clusters (legacy) |
| ≥1 membership row | Only listed clusters; role = membership role |

API (admin only): `GET/PUT /api/clusters/:id/access`, `DELETE /api/clusters/:id/access/:userId`.

See [OPS.md](OPS.md).

## API endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/auth/login` | Local or LDAP password login |
| POST | `/api/auth/logout` | Client-side token discard |
| GET | `/api/auth/oidc` | Start OIDC (redirect) |
| GET | `/api/auth/oidc/callback` | OIDC code exchange |
| POST | `/api/auth/password` | Change local password |
| GET | `/api/auth/me` | Current user |
