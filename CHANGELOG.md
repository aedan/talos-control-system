# Changelog

## [Unreleased]

## [0.4.6] — 2026-08-20

### Added
- **CLI hover tooltips** — every button, input, select, and textarea across the Dashboard, cluster detail, machine detail, import wizards, Workloads explorer (YAML/Logs/Terminal/Actions), and all Settings pages now shows a native `title` tooltip on hover.
- `Button` component accepts an optional `title` prop forwarded to the underlying `<button>`.

### Changed
- **CLI is now the default** — running bare `tcs` prints help instead of starting the server. Use `tcs serve` to run the control-plane server.
- **Installer systemd unit** now starts the service with `tcs serve` (the installer always rewrites the unit on install/upgrade).
- **Local CLI server discovery** — when no `--server` flag, `TCS_SERVER` env, or `~/.tcs/config` is set, the CLI reads `/etc/tcs/config.toml` (or `$TCS_CONFIG`) and targets the local server's `advertised_url` (or concrete `bind_addr:http_port`). This fixes `tcs login` on the host without passing `--server`.

## [0.4.5] — 2026-08-20

### Changed
- **Dashboard-centric IA** — removed the left sidebar; the Dashboard (`/`) is now the primary page and absorbs the Clusters list (status, K8s/Talos versions, node counts, delete, live polling). Settings is a top-bar dropdown linking straight to sub-pages.
- **Per-cluster upgrades** — upgrade jobs now live on the cluster detail page (inline list with details/cancel) instead of a global `/upgrades` page. New API `GET /clusters/:id/upgrade-jobs`.
- Cluster detail auto-refreshes (15s) and drops the redundant Nodes tab plus the Test Talos / Probe versions actions (Refresh from K8s remains).

### Removed
- Global `/upgrades` page, fleet upgrade form, and `POST /fleets/upgrades` + `GET /upgrade-jobs` APIs.
- Global `/machines` and `/machines/pending` list pages (machine detail + import remain).
- Machine Classes feature end-to-end (UI, APIs, repo/model, `machine_classes` table via migration `015`).

## [0.4.2] — 2026-08-11

### Added
- **Per-machine Talos config editor** — load live config, save desired YAML, helpers for install image / network / extra mounts, dry-run and apply (optional reboot)
- APIs: `GET/PUT /machines/:id/config`, `GET .../config/live`, `POST .../config/apply`, `POST .../config/helpers`
- Cluster config patches accept optional `machineId` for node-scoped patches

## [0.4.1] — 2026-08-10

### Added
- Machine inventory **YAML/CSV import** (`/machines/import`, `POST /api/machines/import`)
- Full machine inventory editor (hostname, role, cluster, MAC, address, install disk)
- **Live metal config** apply via Settings UI (`/var/lib/tcs/metal.toml`, rebind DHCP/PXE without process restart)
- Docs: `docs/INVENTORY.md`

## [0.4.0] — 2026-08-10

### Added
- **Full metal provisioning (alpha)** — Redfish (primary) / IPMI power, full DHCP on a dedicated interface, HTTP iPXE + Talos asset cache, and metal provision jobs (PXE → install → bootstrap)
- `POST /api/machines`, BMC power/boot APIs, PXE profiles, DHCP leases, `/clusters/:id/provision`
- Settings → Metal / PXE UI; machine detail BMC panel; create-wizard MAC/BMC + start provision
- Docs: `docs/METAL.md`

### Fixed
- Install path injects selected `install_disk` into machine config YAML
- Generated greenfield talosconfig auto-attaches to cluster when `clusterId` is set
- Create-wizard TypeScript type that broke CI `svelte-check`

### Ops
- FTC and other envs should deploy **tagged releases only** (not main `0.1.0-dev` artifacts)

## [0.3.0] — 2026-08-10

### Added
- **Live TLS certificate reload** — switch self-signed / Let's Encrypt / provided without process restart when HTTPS is already bound
- **Bare-metal install assist** — disk discovery, install-disk selection, install (apply+reboot), bootstrap via UI wizard
- Pure-Rust Talos config/PKI generation (no `talosctl` for secrets)
- TLS overlay at `/var/lib/tcs/tls.toml` for Settings UI under systemd hardening

### Fixed
- Certificate status/renew use live TLS mode (not boot-time config only)
- Settings UI messaging for live apply vs restart-only cases
- Login shell (sidebar) after client-side navigation

### Notes
- Bare-metal path assumes nodes are already in the Talos installer environment
- PXE / Redfish / IPMI remain out of band
- LE HTTP-01 still needs public port 80 + correct DNS

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
