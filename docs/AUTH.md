# Authentication

TCS supports three authentication backends: local passwords, LDAP/Active Directory, and OIDC.

## Auth Providers

### Local (Default)
- Passwords hashed with argon2id
- JWT tokens issued on login
- Default admin created on first boot (random 16-char password printed to stdout)

### LDAP / Active Directory
- Simple bind authentication via `ldap3`
- Configurable user search base, filter, and group-to-role mappings
- Supports LDAPS

Config example:
```toml
[auth.ldap]
url = "ldaps://ad.example.com:636"
user_search_base = "ou=users,dc=example,dc=com"
user_search_filter = "(sAMAccountName={})"
default_role = "viewer"

[[auth.ldap.group_role_mappings]]
group_dn_pattern = "CN=Admins,CN=Users,DC=example,DC=com"
role = "admin"
```

### OIDC
- Raw `reqwest`-based flow (no `openidconnect` dependency)
- Standard authorization code flow
- Auto-provisions users on first OIDC login

Config example:
```toml
[auth.oidc]
issuer_url = "https://auth0.example.com/"
client_id = "your-client-id"
client_secret = "your-client-secret"
redirect_url = "https://tcs.example.com/api/auth/oidc/callback"
scopes = ["openid", "email", "profile"]
```

## JWT Configuration

```toml
[auth]
jwt_secret = "your-secret-key-change-me"
jwt_ttl_hours = 24
```

Change via `/settings/auth` in the TCS UI.

## Roles

| Role | Permissions |
|------|-------------|
| `admin` | Full access: manage users, clusters, settings |
| `operator` | Manage clusters and nodes |
| `viewer` | Read-only access to clusters and status |

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/auth/login` | Authenticate and get JWT |
| POST | `/api/auth/logout` | Invalidate session |
| POST | `/api/auth/oidc` | Initiate OIDC flow |
| GET | `/api/auth/oidc/callback` | OIDC callback handler |
| PUT | `/api/auth/password` | Change password |
| GET | `/api/auth/me` | Get current user info |
| GET | `/api/auth/users` | List all users (admin) |
| GET | `/api/settings/auth/config` | Get auth configuration |
| PUT | `/api/settings/auth/config` | Update auth configuration |
