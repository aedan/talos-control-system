# Changelog

## [Unreleased]

## [0.5.0] — 2026-08-21

### Added
- **Real `kubectl` / `helm` / `talosctl` passthrough** — new CLI verbs `tcs kubectl …`, `tcs helm …`, and `tcs talosctl …` run the actual binaries **server-side** on the TCS host, using the cluster's stored credentials. The kubeconfig/talosconfig is decrypted in memory, written to a `0600` file in a `0700` temp dir (removed on drop, even on panic), and never reaches the CLI — only command output does.
  - One-shot commands (`tcs kubectl get pods -A`, `tcs helm list -A`, `tcs talosctl get machines`) go over `POST /api/clusters/:id/tool` and return stdout/stderr/exit code.
  - Interactive commands (`tcs kubectl exec -it <pod> -- sh`) are auto-detected and bridged over a PTY WebSocket at `GET /api/clusters/:id/tool/tty` (stdin/stdout/resize/exit).
  - The tool name is restricted to an allowlist and argv is passed verbatim (no shell), so there is no shell-injection surface.
- **Installer now provisions the toolchain** — `install.sh` additionally installs version-pinned `kubectl` (v1.31.4) and `helm` (v3.16.2) alongside `talosctl` (v1.13.8), so passthrough works out of the box.

### Changed
- The existing Rust kubectl-like verbs (`tcs get/describe/logs/exec/...`) are kept as fast paths; the new passthrough verbs are the general escape hatch for anything they don't cover (including all of Helm).

## [0.4.8] — 2026-08-21

### Fixed
- **`tcs get <kind> <name>`, `describe`, and `delete`** — these failed with `missing field 'name'` because the server's `get_resource`/`delete_resource` query structs declared a `name` field that is actually in the URL path. Removed the bogus field so single-object fetches, describes, and deletes reach the API.
- **`tcs scale`** — failed with 404/400. The request targeted the wrong path and used server-side apply, which the `/scale` subresource rejects. It now merge-patches the `spec.replicas` on the deployment's `/scale` subresource.
- **`tcs delete`** — failed to deserialize the API response (the API returns the deleted object, not a `Status`). Now deserializes loosely so both shapes work.
- **Short kind names** — `get svc`, `get ns`, `get po`, `get deploy`, etc. now resolve via a kubectl short-name map (the discovery `ApiResource` type carries no short names).

### Added
- **`-n`/`--namespace`** short flag on `get`, `describe`, `logs`, `exec`, `attach`, `delete`, and `scale` for kubectl parity (`--ns` still works).

## [0.4.7] — 2026-08-21

### Added
- **Interactive `tcs login`** — run bare `tcs login` to be prompted for email (visible) and password (hidden). Positional `tcs login <email> <password>` and `--email`/`--password` flags still work.
- **`--cluster` accepts names** — `tcs --cluster kronos get pods` now resolves the cluster by name or unique UUID prefix (the server routes are keyed by UUID). Full UUIDs still work.
- **`-A`/`--all-namespaces`** on `tcs get` for kubectl parity (lists across all namespaces, which is already the default).

### Changed
- `tcs login` no longer requires both positional args; missing values fall back to prompts.

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
