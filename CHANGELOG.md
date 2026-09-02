# Changelog

## [Unreleased]

## [0.5.21] — 2026-08-31

### Fixed
- **GoDaddy DNS-01 record name (the actual blocker).** GoDaddy's API *stores* a TXT record given the full FQDN as the name but **never publishes it** to its nameservers, so Let's Encrypt sees NXDOMAIN. The fix: GoDaddy's record name must be the **relative** name under the zone (e.g. `_acme-challenge.tcs.kronos` under `cloudmunchers.net`, not the full `_acme-challenge.tcs.kronos.cloudmunchers.net`). Verified empirically against a live GoDaddy zone: the relative-name form resolves publicly in <45s; the full-FQDN form did not. `split_zone` now derives the base domain + relative name; an explicit `dns_zone` override still wins. This completes a working Let's Encrypt DNS-01 flow via GoDaddy.

### Fixed
- **DNS-01 propagation wait.** GoDaddy's API returns `200` for a new TXT record before its authoritative nameservers actually serve it, so Let's Encrypt's single check saw `NXDOMAIN` and failed the challenge. TCS now **polls a public resolver (`dig @8.8.8.8`, falling back to `getent`) until the `_acme-challenge.<domain>` TXT record is publicly resolvable** (up to 150s) before asking Let's Encrypt to validate, and extended the ACME challenge wait window to ~150s. If the record never propagates it fails with a clear "provider propagation delay" message instead of a confusing NXDOMAIN.

### Fixed
- **GoDaddy DNS-01 TXT record schema.** GoDaddy's v1 API rejected the record body with `422 INVALID_BODY`: TXT `data` must be a plain **string** (not an array) and `ttl` must be **≥ 600**. Fixed the GoDaddy provider to send `{"data": "<value>", "ttl": 600}`. (Auth + zone derivation were already correct — this was the last blocker to a real Let's Encrypt DNS-01 cert via GoDaddy.)

### Fixed
- **DNS-01 Let's Encrypt actually works now.** The live "Apply" path (`TlsRuntime::apply_mode`) and the boot path **hardcoded HTTP-01**, ignoring the `challenge_type` you selected — so a `dns-01` config failed with an HTTP-01 "no valid A records" error. Both now route through `AcmeClient`, which dispatches by challenge type. The `AcmeClient` DNS-01 arm was also a **stub that silently produced a self-signed cert** instead of doing a real DNS-01 challenge — replaced with a real implementation.

### Added
- **Real DNS-01 ACME issuance** (`obtain_dns01_certificate`): for each domain it computes the `base64url(sha256(keyAuth))` digest, publishes the `_acme-challenge.<domain>` TXT record via the configured provider, validates the challenge, finalizes the order, downloads the Let's Encrypt cert, then removes the TXT record.
- **GoDaddy provider** now does real TXT create/delete (it previously existed but was never invoked by the ACME flow). It derives the GoDaddy-registered zone from the challenge domain (last 2–3 labels) and accepts an optional explicit **DNS Zone / registered domain** override for delegated subzones (e.g. `kronos.cloudmunchers.net`).
- `DnsProviderConfig` gains `dns_zone`; the Certificates UI adds a GoDaddy **DNS Zone** field; Cloudflare (token+zone) and Route53 (needs AWS creds — returns a clear error) are wired to the same flow. `build_dns_provider` validates credentials so misconfiguration fails loudly instead of silently self-signing.
- 5 new backend tests (GoDaddy zone-split heuristics, provider credential validation).

### Fixed
- **Certificates UI now reflects the real cert.** The Certificates page showed "mode: disabled / issuer: None" even though TCS was actually serving a self-signed cert on :443 (the v0.5.15 auto-fallback). The status endpoint now reports the **effective** mode: when the config says `disabled` (or Let's Encrypt falls back), the live runtime is set to `self-signed` with the actual domain(s), so the UI shows `self-signed` / `Self-Signed` / the real host + the cert's true expiry.
- **Removed the "Disabled" option from the Certificates mode dropdown** — TCS always serves :443, so there's no longer an off switch. The dropdown defaults to **Self-Signed** (the effective default), and a legacy `disabled` config is normalized to `self-signed` on load.
- Corrected the Certificates page description: it no longer claims enabling TLS "needs a restart to open :443" (always-on since v0.5.15/0.5.16).

### Changed
- **Removed the legacy `http_port` (8081) listener.** TCS is alpha with a single live deployment, so the backward-compat 8081 listener added in v0.5.15 is gone. Shipped binaries now bind **only :80 (HTTP → redirect/ACME) and :443 (HTTPS)** — no more `http_port`. `server.http_port` in config is ignored; `advertised_url` defaults to `https://localhost:443` when unset; the CLI's local server discovery now targets `https://<bind>:443`.
- **Non-root dev support.** New env overrides `TCS_HTTPS_PORT` (0 disables the :443 listener) and `TCS_HTTP_PORT` let `cargo run -- serve` bind plain high ports without root (dev: `TCS_HTTPS_PORT=0 TCS_HTTP_PORT=8081 cargo run -- serve`). When HTTPS is disabled, the HTTP listener serves the real app (not the redirect router), so the Vite dev proxy keeps working. A skipped listener no longer trips the shutdown `select!`.
- System Settings page now shows **Ports: 80 (HTTP→redirect) · 443 (HTTPS)** instead of a stale `bind:8081`.

### Documentation
- `docs/{CONFIGURATION,TLS,DEVELOPMENT}.md` + `README.md` + `config.example.toml`: removed all `http_port`/8081 production references; documented the always-on 80/443 model and the dev port overrides.

### Changed
- **TCS now always listens on 80 + 443, with or without a certificate.** Previously a TLS-disabled install only bound `http_port` (8081) and could not be switched to HTTPS without a restart; a TLS-enabled install hard-bound :443 but only served it. Now every install:
  - binds **:443 (HTTPS)** — always. If no usable certificate exists it generates a self-signed one at startup, so HTTPS works immediately out of the box;
  - binds **:80 (HTTP)** — always, serving ACME challenges and redirecting everything else to :443;
  - keeps the legacy **`http_port` (8081)** listener as a backward-compat escape hatch, and still starts even if :80 is taken by something else.
- **Self-signed is the default for fresh installs.** A new `[tls]` section with `mode = "disabled"` (or no cert) now boots HTTPS on a generated self-signed certificate rather than dropping to HTTP-only — so the UI/API are reachable over `https://<host>:` from day one. Existing installs keep working (their 8081 listener is preserved).
- Certificate reload is now fully live: enabling/updating TLS from a non-TLS process no longer requires a restart — the `:443` rustls config hot-swaps the new certificate via `ReloadableCertResolver`.

### Added
- **Siderolink auto-bake for greenfield configs.** New persistent **per-cluster join tokens** (`cluster_siderolink_tokens`, migration 019). When you generate machine configs for a cluster (Provision wizard / generate-config API), the `machine.siderolink { enabled, endpoint, token }` block is now spliced into both the controlplane and worker configs automatically, so provisioned nodes dial in and form the WireGuard tunnel on first boot — no manual per-node config. Resolves the known limitation called out in v0.5.14.
- **Siderolink per-cluster token management** — `GET /api/siderolink/cluster-token?cluster_id=…`, `POST …/rotate`, `POST …/revoke` (admin), plus a new "Per-cluster tokens" card on the Siderolink settings page (pick a cluster, create/rotate/revoke, copy a ready-to-paste `machine.siderolink` snippet).
- Join-token validation now also accepts the persistent per-cluster tokens, so auto-baked configs register cleanly.
- Unit tests: `siderolink_block_splices_into_machine_valid_yaml` + `empty_siderolink_block_yields_no_siderolink_key` (90 backend tests pass).

### Notes
- The metal-scheduler (PXEl+BMC) greenfield path omits the Siderolink block for now; the wizard/generate-config path is the primary one. Threading it through the metal scheduler is a small follow-up.


### Added
- **Siderolink-based remote management.** When a node is Siderolink-connected, TCS now reaches it through its **WireGuard tunnel IP** (`100.64.x.x`) for *every* management operation instead of (or in preference to) its LAN address — the only path that works for nodes behind NAT/firewalls. Wired through: version probe, reboot, upgrade (Talos + in-place k8s, including the rolling-upgrade scheduler's CP pick), config read/apply (single + batch + merge-with-live), reset, bootstrap, disks, extensions, and the background status reconciler. The tunnel IP is used only while the peer is fresh (last seen < 5 min); a dropped tunnel automatically falls back to the LAN address.
- **Machine page "Management path" indicator** — a green `via Siderolink tunnel` badge (with the tunnel IP) or a `direct` badge (with the LAN address) shows exactly how TCS is reaching that node. The machine API now returns `effectiveEndpoint`, `viaSiderolink`, and `siderolinkIp`.
- New unit test `effective_endpoint_prefers_fresh_siderolink_tunnel` covering tunnel-preferred / stale-fallback / not-connected cases.

### Documentation
- `docs/SIDEROLINK.md`: added "How TCS uses the tunnel for management", "Enabling the tunnel on a TCS host (opt-in)" (install `wireguard-tools`, config, firewall, restart, verify), and a "Known limitation" note that join tokens are one-time-use so Siderolink is configured per node manually (auto-baking into generated configs is a follow-up needing a per-cluster token model).

## [0.5.13] — 2026-08-31

### Changed
- **Clarified the dashboard's cluster/machine entry points.** The bright-blue primary button was mislabeled "Add inventory" but actually opened the bare-metal **provision** wizard, while the real "import a machine list" page had no dashboard button and the "adopt existing cluster" button was a quiet ghost. There are now three equally-weighted primary buttons, each labeled to match its destination: **Import cluster** (adopt a running cluster via kubeconfig/talosconfig), **Add machines** (import a machine list as inventory, YAML/CSV), and **Provision cluster** (build a new cluster from bare metal via PXE+BMC). The empty-state and the header/subtitle of each of the three destination pages now cross-reference the other two so the distinction is clear on arrival.

## [0.5.12] — 2026-08-31

### Fixed
- **Expired credentials no longer leave you stranded on the current page.** When a JWT expired, the UI just sat wherever you were — the token was cleared but nothing navigated. Now: (1) any API call that returns 401 (including background polls on the cluster/machine pages) immediately hard-redirects to the login screen, discarding stale in-memory state; and (2) the root layout runs a 60-second `/api/auth/me` heartbeat so even an idle page with no active polling detects expiry and bounces you to login.

## [0.5.11] — 2026-08-31

### Changed
- **Machine page: removed the "Talos ops" filler header.** A section that was only a heading + a paragraph pointing at the "Image & modules" and "Machine config" sections below it — which already label themselves — took ~1/4 of the screen for zero information. Gone. The page now flows straight into the actionable content.

## [0.5.10] — 2026-08-31

### Changed
- **Rolling-upgrade jobs table now shows what a job actually does, not a raw installer image.** The "Image" column read like `ghcr.io/siderolabs/installer:v1.13.7` for *every* job — for a Kubernetes-only upgrade that looked like it was going to re-apply an image and wipe the module set (it isn't; a k8s upgrade is in-place and touches no modules/Talos image). The column is now **Action** and renders a per-job summary derived from the job's `phase`/targets: `Talos → v1.13.7`, `Kubernetes → v1.36.4`, or `Talos → vX + Kubernetes → vY`. The installer image is still available in the row tooltip and the Details view for reference.

### Fixed
- **Module-picker rows: the "· author" suffix was a different (smaller) font and sat slightly off the module name's baseline.** The name used the monospace `.mono` size (0.8rem) but the author used `.hint` (0.85rem, different family/weight). Both now use the monospace font and size — the author is only muted in color — so name and author line up.

## [0.5.9] — 2026-08-31

### Fixed
- **A k8s upgrade job that had already converged was stamped "cancelled" if you clicked Cancel mid-flight.** `run_job` force-cancelled on the first tick after a cancel request, *before* the phase's completion check could record the finished step — so the job showed `cancelled` even though the live API server had already rolled to the target version. The top-level cancel now only force-cancels when no target is actively `running` and the work isn't already done; an in-flight `talosctl upgrade-k8s` is allowed to finish (it can't be safely aborted mid-roll), and the phase loops stop before the next unit. Verified on kronos: a 1.36.2→1.36.4 k8s upgrade cancelled at 16:37 still completed the roll at 16:42, but the job was mislabeled `cancelled` — with this fix the status reflects the converged cluster.

## [0.5.8] — 2026-08-31

### Fixed
- **In-place Kubernetes upgrade never actually ran (stuck in a dispatch retry loop).** Two independent bugs in the k8s upgrade path:
  1. `talosctl upgrade-k8s` was invoked with the global `-e/--endpoints` flag, which talosctl v1.13+ ignores for node selection — the command failed every tick with "nodes are not set for the command" and the job retried forever without touching the cluster. Now invoked with `--nodes <bare-host>` (the endpoint is normalized to strip scheme/port, since the talosconfig supplies the port). Verified against a live Talos 1.13.7 / K8s 1.36.2 cluster: `--nodes` discovers all 15 nodes and the 1.36.2 → 1.36.4 dry-run plan succeeds.
  2. The k8s step *poll* (`poll_k8s_step`) built its `ClusterController` with an empty `jwt_secret` (`String::new()`), so even a successfully-dispatched upgrade could never be detected as complete — it looped on "Decrypt failed — JWT secret may have changed". The poll now receives the real `sqlite_path` and `jwt_secret` from `run_k8s_phase`, matching every other scheduler.

## [0.5.7] — 2026-08-31

### Fixed
- **Rolling-upgrade panel showed "Kubernetes probe skipped: Decrypt failed — JWT secret may have changed" on every cluster page.** `GET /clusters/:id/upgrade-targets` built its `ClusterController` with the pool-only `new()` constructor, which leaves `jwt_secret` empty — so decrypting the cluster's stored talosconfig/kubeconfig used a wrong AES key and always failed. It now uses the shared `controller_for(state)` helper, which passes the real `auth.jwt_secret` (and sqlite path), exactly like every other handler. The Talos-only upgrade path was unaffected (it reads the factory image, not the encrypted secrets); this restores the live k8s version + supported-target probe.

## [0.5.6] — 2026-08-31

### Fixed
- **Rolling upgrade marked every node "failed" on a healthy upgrade.** `talosctl upgrade` was internally trying to drain each node by fetching a kubeconfig *from the worker itself*, which only works on control-plane nodes — so every worker target failed after the Talos image swap actually succeeded. Now the scheduler cordons + drains a worker via the cluster's stored kubeconfig *before* the upgrade, runs `talosctl upgrade --drain=false`, and uncordons it when the node returns. Control-plane nodes are cordoned only (never drained) so etcd/apiserver aren't evicted.
- **"Start rolling upgrade" appeared to do nothing in Firefox/Safari/Chrome.** The handler used the native `window.confirm()`, which browser extensions can silently auto-dismiss (returning false with no dialog), so the click returned before any toast or network call. Replaced it with an in-page confirm modal that extensions cannot suppress.

## [0.5.5] — 2026-08-31

### Fixed
- **"Start rolling upgrade" not applying a modules-only change.** The button's change detection relied on a `modulesDirty` flag that could be stale (out of sync with the cluster's stored module set), so toggling modules and clicking Start silently no-opped. It now recomputes the difference between the selected module set and the cluster's *stored* default at click time, always sends the full selected set so the backend is authoritative, and shows a clear "Nothing to change" message (with a per-item breakdown in the confirm dialog) when there is genuinely no change.

## [0.5.4] — 2026-08-31

### Changed
- **Dropped the "Workers first" toggle** from the rolling-upgrade panel. Workers-first / control-plane-last is the safe ordering for Talos, so it's now always applied rather than a user option that could be flipped into a risky ordering.
- **Moved the Upgrade jobs list into the Rolling upgrade panel** and removed it from the "Cluster actions" section (which now only holds the Kubernetes Workloads explorer). Jobs sit right under the Start button where they belong.

## [0.5.3] — 2026-08-31

### Changed
- **Rolling upgrade panel tightened up.** The verbose intro copy is now a single line, and the Talos-phase controls (max-unavailable + "workers first") moved up into the panel next to the version dropdowns — so everything for an upgrade lives in one place instead of the max-unavailable control being buried in the "Cluster actions" block at the bottom. The bottom block is now just the **Upgrade jobs** list.

## [0.5.2] — 2026-08-31

### Fixed
- **`GET /clusters/:id/upgrade-targets` no longer 502s the whole rolling-upgrade panel** when an upstream probe fails. The factory Talos-version fetch and the live `talosctl upgrade-k8s` probe are now independent: if either is unavailable (egress blocked, missing/bad talosconfig, node flapping), the panel still renders with what it has and shows a warning note instead of a hard error. `k8s_upgrade_targets` also degrades a missing/malformed talosconfig to "no k8s targets" rather than an error, so the Talos-only upgrade path stays usable.

## [0.5.1] — 2026-08-30

### Added
- **Consolidated cluster rolling upgrade** — the cluster page now has a single "Rolling upgrade" panel that derives everything: pick a **Talos version** (dropdown from the Image Factory), adjust the **module** set, and optionally pick a **Kubernetes version**. Pressing **Start rolling upgrade** queues one job that runs a per-node Talos image roll (reboots, workers-first by default) and then an in-place, no-reboot `talosctl upgrade-k8s` for the Kubernetes bump. No more free-text installer image.
- **Kubernetes upgrade phase** — in-place, control-plane-first, no reboots. The UI dropdown lists only targets this Talos build supports (probed via `upgrade-k8s --dry-run`): a same-minor patch bump and the next minor(s). Selecting a version more than one minor ahead runs a **sequential ladder** (one minor at a time, as Kubernetes requires).
- **Node-level module deltas** — on the machine page, add (+) or remove (−) individual modules relative to the cluster default. Effective set = cluster defaults − removes + adds, and it recomputes automatically when the cluster defaults change. A **Reset to cluster defaults** button clears a node's override. This coexists with the existing absolute "Apply modules" picker (an explicit absolute selection wins).
- **`GET /clusters/:id/upgrade-targets`** — returns the cluster's current Talos version, available factory Talos versions, current Kubernetes version, and the supported in-place Kubernetes upgrade targets.
- **`PUT /machines/:id/module-overrides`** — set/clear a node's add/remove deltas against its cluster default module set (`{adds, removes, reset}`).

### Changed
- `POST /clusters/:id/upgrade` now accepts `{talosVersion, k8sVersion, modules, maxUnavailable, controlPlaneLast}`. A legacy `image` (installer tag) is still accepted and translated, but the derived fields are the primary path.
- Upgrade jobs are versioned (`upgrade_jobs`/`upgrade_job_targets` migrations) to carry the per-node derived image, the k8s target, the phase (`talos` → `k8s`), and the k8s step ladder.
- Image Factory schematic requests are cached per (version, modules) so deriving a per-node image for a whole cluster doesn't hammer the factory API.

### Fixed
- Kubernetes "current version" detection now uses the live API-server `/version` (via the stored kubeconfig) instead of falling back to the machine's Talos version.
- Removed invalid `--with-docs` / `--with-examples` flags from the `talosctl upgrade-k8s` invocation (the build's `upgrade-k8s` subcommand does not accept them).

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
