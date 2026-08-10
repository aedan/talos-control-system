# Changelog

## [0.2.0] — 2026-08-10

### Added
- Rolling cluster/fleet upgrade jobs with scheduler and UI
- SAML SP (alpha), multi-tenant branding, greenfield config factory
- Siderolink inventory + optional host WireGuard path
- Postgres dual-backend runtime (`DbPool`) and `tcs migrate-sqlite-to-postgres`
- Multi-replica HA foundation (`ha_locks`, DB OIDC state)
- Machine reset/bootstrap, cluster scale (inventory), provision apply-config
- Admin password reset API + Users UI
- Login shell fix (sidebar after client-side navigation)

### Changed
- Version **0.2.0**; CI/CD Node.js **22**
- STATUS / AUTH / POSTGRES / TALOS / SMOKE documentation refresh

### Notes
- SQLite remains the default database
- WireGuard requires `wg`/`ip` on the TCS host
- Full PXE/IPMI metal provision remains out of scope

## [0.1.0] — earlier

Alpha import-centric control plane: local/LDAP/OIDC auth, etcd backup/restore,
config apply (pure-Rust COSI), host installer (no Helm).
