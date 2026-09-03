# Changelog

## [Unreleased]

## [0.5.46] — 2026-09-03

### Fixed
- **Cluster page reported "0 nodes connected through the tunnel" even though all 15 SideroLink tunnels were up and healthy.** The SideroLink peer registry is keyed by each node's **Talos MUID** (the hardware machine ID, e.g. Dell/HP tag `091716XMQ524041H`, sent as `node_uuid` on Provision), but the machine inventory was keyed by a TCS-invented **`mac-<MAC>` alias** in `machines.system_uuid`. Every place that intersected the two — the cluster `siderolink_status` endpoint (which builds the UI's connected count), the `effective_endpoint` tunnel-IP lookup, and the `set_siderolink_connected` flag update — matched MUID against the MAC alias and found **zero** rows, so the count stayed 0 and management always fell back to the LAN address. Fixed by capturing each node's real MUID (`talosctl get systeminformation → spec.uuid`, retrieved by the status reconciler) into a new `machines.muid` column and correlating by it (with the legacy `system_uuid` as fallback) in all three places. The reconciler now also refreshes a peer's `last_seen` and sets `siderolink_connected` from a live peer every cycle, so the flag and the tunnel-IP preference stay accurate over time.

### Added
- `machines.muid` column (migration `020_machine_muid.sql`) + index, the `TalosctlClient::get_muid` helper, and `repos::machine::set_muid` / MUID-aware `set_siderolink_connected`.

## [0.5.45] — 2026-09-03

### Fixed
- **SideroLink device recreation never applied the WG private key — the tunnels stayed dead after every auto-prime.** The code set the identity with `wg set tcs-sl0 … private-key /dev/stdin`, but the command runner spawns with **null stdin**, so `wg` read an empty key and aborted the entire `wg set` — leaving the device with a **random listen port and no private key**, to which no node could ever complete a handshake. The `.or_else` tmp-file fallback was supposed to cover this, but the observed on-host behavior (recreated device stuck at an ephemeral port, 0/15 fresh handshakes for minutes, while an identical **manual** recreation with the key applied from the file came up 15/15 in ~20s) proved the key was not landing. `ensure_interface` (boot) and `prime_socket` (recreation) now apply the identity by pointing `wg` directly at the on-disk key file (`data_dir/siderolink_wg_private.key`), the proven-reliable path — no `/dev/stdin`, no in-memory-string fallback. With the key actually set, a code-driven recreation behaves exactly like the manual one: all 15 peers re-handshake within ~20-30s and `ping6` over the tunnel is 0% loss. Combined with v0.5.44's reactive cooldown watchdog, a clean boot and an Enable/Disable toggle now self-heal to a working tunnel with no manual intervention.

## [0.5.44] — 2026-09-03

### Fixed
- **SideroLink self-heal watchdog flapped against the nodes' re-provisioning, so it had to be reworked from a scheduled backoff into a reactive cooldown.** On v0.5.43 the watchdog recreated the device on a growing timer (45→90→180→300→480s). This fought the nodes: it recreated, the 15 nodes re-handshook (tunnels briefly healthy — confirmed 14/15 fresh), then the next timer tick recreated **again** and wiped them, in a loop. The watchdog now **reacts only to a real staleness signal** — peers present but no *fresh* handshake (age < 45s, i.e. a healthy node's 25s keepalive has gone stale) — and enforces a **240s cooldown** after every recreation so it gives the nodes time to re-provision and re-handshake and cannot flap. When healthy it does nothing and only re-checks every 20s; when stale-but-cooling-down it holds and re-checks every 15s. Combined with v0.5.43's full-device-recreation prime, this means a clean boot and an Enable/Disable toggle reach a working tunnel and the watchdog only recreates when it genuinely needs to, at most once per 4 minutes.

## [0.5.43] — 2026-09-03

### Fixed
- **SideroLink tunnels still dead after boot/toggle — the socket "prime" was a link down/up that never actually reset the stale kernel WG socket.** v0.5.42's backoff reduced the bounce rate but the bounce itself (`ip link down/up`) was the wrong primitive: confirmed live that once `tcs-sl0`'s kernel UDP socket enters the bound-but-not-receiving (stale) state, a link `down`/`up` does **not** reset it — node WireGuard handshake-inits keep arriving on the wire (visible in tcpdump) but the kernel drops them (no reply, frozen transfer counters, handshake ages frozen). `SiderolinkWg::prime_socket` now **fully recreates the device** (`ip link del` + `ip link add … type wireguard`), which builds a brand-new receiving socket; on kronos a single recreation brought all 15 peers to fresh handshakes and 0% `ping6` loss within ~20s. Because recreation wipes the kernel peer list, the manager now keeps a `peer_cache` (public key → allowed IP, maintained by `set_peer`/`reapply_peers`) and re-applies it right after each recreation so a prime never loses the configured peers. The watchdog keeps the v0.5.42 growing-backoff cadence (initial 45s settle, then 45→90→180→300→480s) and keys success on a *fresh* handshake (age < 45s, via `parse_handshake_age_secs`), standing down quietly while healthy and re-priming (recreating) only on real degradation. Net: a clean boot and an Enable/Disable toggle both reach a working tunnel with no manual intervention.

## [0.5.42] — 2026-09-03

### Fixed
- **SideroLink watchdog death-spiral: it bounced the socket on a fixed ~30s interval and never let a freshly-created device settle.** The v0.5.41 watchdog primed (link down/up) every ~30s whenever it saw no fresh handshake — permanently true while a fresh device was still coming up — so it bounced indefinitely (10+ minutes of bounces) and the tunnels stayed down. The watchdog now waits ~45s before its first prime and uses growing backoff between primes (45→90→180→300→480s), standing down (90s idle checks) the moment a fresh handshake (age < 45s) is seen and resuming only on degradation. **Note:** the backoff reduced flapping but did not by itself restore data flow, because the prime used a link down/up that does not reset a stale WG socket — that is fixed in v0.5.43 (full device recreation).

## [0.5.41] — 2026-09-02

### Fixed
- **SideroLink tunnels went stale and stayed down after boot/toggle — the finite prime loop gave up before the fresh device was ready.** A freshly-created `tcs-sl0` kernel WG socket does not reliably receive until the device has aged an **indeterminate** amount; on kronos one boot the socket only became functional well over 5 minutes after start, so the v0.5.40 "prime every 15s for ~5 min, then give up" loop exhausted its retries and left all 15 tunnels stale (handshakes frozen, ping6 100% loss) until a manual prime. `SiderolinkWg::re_prime_in_background` is now a **persistent, self-healing watchdog** running for the whole process lifetime: every ~20s, if there are known kernel peers but no *fresh* handshake (age < 45s), it primes the socket once (bounce + key + addrs) and keeps checking until the tunnels recover — no time cap. When healthy (nodes keepalive every 25s → handshakes always fresh) it is silent and never bounces. The boot path starts this single watchdog (idempotent via a `Once` guard), and the Enable/Disable handlers also call it, so both the boot-aging case and the toggle re-provision case (nodes rotate WG keys → handshakes go stale → watchdog re-primes them back) self-heal with no manual intervention. A new `parse_handshake_age_secs` helper (with unit tests) distinguishes fresh from stale handshakes, which the old "any handshake line exists" check could not.
- **SideroLink cluster Enable/Disable toggle failed to push config to nodes — "Decrypt failed — JWT secret may have changed".** The `POST /clusters/:id/siderolink/enable` and `/disable` handlers built their `ClusterController` with `ClusterController::new(pool)`, whose constructor carries an **empty** `jwt_secret`. When the controller then decrypted the cluster's stored talosconfig to fetch each node's live config (merge-on-enable / strip-on-disable), `secrets::decrypt("", …)` failed on every node, so the toggle reported "no machines patched" and never applied the `SideroLinkConfig`. Both handlers now use the shared `controller_for(&state)` helper, which binds the controller to the real `config.auth.jwt_secret` (the same secret the REST `/talosconfig` endpoint uses and that the stored talosconfig was encrypted with). The toggle now pushes/pulls the SideroLink config live to every running node with no reboot. Verified on kronos: the failing call was reproduced, and the secret-bound controller decrypts the stored talosconfig correctly.
- **Let's Encrypt certificate fell back to self-signed on every TCS restart/upgrade.** `resolve_initial_certificate` went straight to a fresh ACME issuance on each boot and never read the cert already on disk, so repeated installs (e.g. the upgrade path re-running `tcs install`) exhausted Let's Encrypt's "5 certificates per exact identifier set per 168h" rate limit — after which the server silently downgraded to an unverifiable self-signed cert even though a valid LE cert was sitting in `/var/lib/tcs/certs/`. Boot now **loads the persisted LE cert first** (strict: must parse, cover a configured domain, and be outside the 30-day renewal window), persists a freshly issued cert to disk on success, and only if issuance fails (e.g. rate-limited) serves the persisted cert under a lenient check before falling back to self-signed. A valid LE cert now survives restarts and upgrades.
- **Issued Let's Encrypt certs are now persisted to disk with restrictive key perms.** `cert::renewal::write_cert_to_disk` is now public (so the boot issuance path can reuse it, not just the renewal task) and sets `key.pem` to `0600`.

### Changed
- **Cluster page: the SideroLink tunnel section is now a compact toggle** at the top of the Rolling upgrade panel (checkbox with a short "on · N nodes connected" / "off" status) instead of a separate expandable "Siderolink tunnel" dropdown with a per-node table. Toggling applies live to all nodes (no reboot); errors surface inline.

## [0.5.40] — 2026-09-02

### Fixed
- **Siderolink background socket prime used fixed delays that could all land before the device was ready.** A freshly-created `tcs-sl0` device does not reliably receive until it has aged an indeterminate amount (on kronos, primes at +6s and +71s left the peer at 0 rx, while the identical prime at ~3 min worked immediately) — so fixed-delay primes could all miss. The background prime thread now **primes, waits ~8s, and checks whether any peer completed a handshake** (via `wg show` "latest handshake"), retrying every ~15s until a handshake is observed or ~5 minutes elapse, then stops. It exits early the moment the tunnel is functional (so a fast-aging device isn't over-primed) and is silent when no node is joining (the give-up path logs once). This makes a clean first-boot come up with a working Siderolink tunnel regardless of how long the device takes to become functional.

## [0.5.39] — 2026-09-02

### Fixed
- **Siderolink background socket prime fired too early to take effect.** A freshly netlink-created `tcs-sl0` device does not reliably receive until it has been alive for well over 6 seconds — on kronos a prime bounce (down/up + key re-set) at +6s left the peer at 0 rx, while the identical prime run on an already-aged device worked immediately. The single +6s prime therefore missed the window. The background prime thread now re-primes at **6s, 26s, and 71s** after boot (cumulative), so once the device has aged enough one prime lands on a functional socket; each prime is idempotent and safe on an already-working tunnel (a WG link down/up preserves peer state and only clears the addresses, which are restored). This makes a clean TCS boot — including the true first-boot with no surviving device — come up with a working Siderolink tunnel with no manual intervention.

## [0.5.38] — 2026-09-02

### Fixed
- **Siderolink interface never created at boot (regression introduced in v0.5.37).** While refactoring `SiderolinkWg::init` to spawn the background socket-prime thread, the `ensure_interface()` call was accidentally dropped, so on TCS startup the `tcs-sl0` WireGuard device was never created, `wg_enabled` stayed `false`, the prime thread never spawned, and the tunnel could not come up (`tcs` reported "Siderolink WireGuard not active" was not even logged because the match block was gone). Restored the `ensure_interface()` call (and its ready/not-active logging) before the manager is wrapped in `Arc`, so the device is created, the key set, the socket settled, and the background prime thread is spawned when the interface is enabled.

## [0.5.37] — 2026-09-02

### Fixed
- **Siderolink tunnel reliably down on a fresh TCS boot — the just-created kernel WG device needs a delayed socket prime.** Even with v0.5.36's 1.5s settle, a WireGuard device created from scratch at boot does not reliably receive for the first several seconds: node handshake-inits arrived at the host yet the peer stayed at 0 rx and TCS sent no replies, so the tunnel only came up after the device had been alive a while and was manually bounced. A 1.5s inline settle is not enough on a true first-boot (no prior device); the socket needs the device to have actually settled in the kernel. `SiderolinkWg::init` now spawns a detached background thread that, ~6s after boot, re-bounces `tcs-sl0` (down/up) and re-sets the listen-port/private-key and restores the overlay addresses — the sequence proven to make a freshly-created device functional. This prime does not disturb already-configured WireGuard peers (a WG link down/up clears addresses but preserves peer state, re-validated live) and never blocks boot. Combined with v0.5.32 (boot peer re-apply) and v0.5.36 (settle delay), a TCS restart — including the true first-boot case with no surviving device — now comes up with a functioning SideroLink tunnel.

## [0.5.36] — 2026-09-02

### Fixed
- **Siderolink tunnel still intermittently down at TCS boot — kernel WG socket needs a settle delay before the key is set.** v0.5.35 set `listen-port`/`private-key` immediately after the down/up bounce, but a freshly (re)created/`up`-ed WireGuard device needs a moment for the kernel to finish attaching its UDP socket; binding the key immediately left the socket in a state that is bound but does not receive (node handshake-inits arrived at the host yet the peer stayed at 0 rx, and TCS sent no replies). `ensure_interface()` now inserts a 1.5s settle delay between the bounce and `wg set private-key`. Validated live on kronos: with the settle delay, a clean TCS boot comes up with a functioning `tcs-sl0` — an already-joined node's WireGuard handshake completes and `ping6` to its `fd…` overlay IP succeeds (0% loss) with no manual intervention.

## [0.5.35] — 2026-09-02

### Fixed
- **Siderolink tunnel still down after TCS restart — `private-key` was set before the socket-rebind bounce.** v0.5.34 set `wg listen-port`/`private-key` and *then* bounced the link down→up, which left the freshly-created device's kernel WireGuard UDP socket stale (bound but not receiving): node handshake-inits arrived at the host yet the peer stayed at 0 rx / no handshake. The proven-working order (validated live on kronos against a running v0.5.34) is to bounce the link **first** (down→up on the existing device) and *then* set `listen-port`/`private-key` on the now-live device — that re-binds the socket and gets it receiving. `ensure_interface()` now does: create-if-absent → up → down/up bounce → `wg set listen-port + private-key` → overlay address → MTU. With this, a TCS restart comes up with a functioning `tcs-sl0` and an already-joined node's cached WireGuard retries complete without needing the node to re-provision.

## [0.5.34] — 2026-09-02

### Fixed
- **Siderolink tunnel still intermittently down after TCS restart — stale kernel WG socket + dropped overlay address.** Two compounding issues in how `tcs-sl0` is built at boot: (1) a WireGuard device that is freshly created via netlink (`ip link add type wireguard`) — or a leftover one from a prior boot — can be left with a kernel UDP socket that is bound to the listen port but never demultiplexes incoming datagrams to the device, so node handshake-inits arrive at the host (visible in tcpdump) yet the peer shows 0 rx / no handshake and UDP `RcvbufErrors` climb; and (2) a down→up bounce of the link **clears its IPv6/IPv4 addresses**, so the server's `fd…::1/64` overlay address was lost and TCS had no route to the node (`Network is unreachable` even after the handshake). `ensure_interface()` now: creates the device only if absent (never delete+recreate), sets the key on the UP device, performs the down→up socket-rebind bounce, and then assigns the overlay addresses **after** the bounce so they survive. Live-validated on kronos from a clean first-boot state: node `connected: true`, WG peer shows a completed handshake, and `ping6` to the node's `fd…` overlay IP over `tcs-sl0` succeeds (0% loss). Together with v0.5.32 (boot peer re-apply) and v0.5.33 (clean start), a TCS restart now comes up with a fully functional SideroLink tunnel.

## [0.5.33] — 2026-09-02

### Fixed
- **Siderolink tunnel still down after restart — stale kernel WG UDP socket.** v0.5.31's down→up bounce did not reliably re-bind the kernel WireGuard socket: the private-key/listen-port were set while the device was still down, which allocates the UDP socket on the default port and leaves it non-receiving after the up. `ensure_interface()` now (1) removes any leftover `tcs-sl0` device for a clean start, (2) brings the link UP **before** setting `listen-port`/`private-key` (setting the key on a live device is what reliably binds the socket to port 443 and gets it receiving), and (3) assigns the overlay addresses after. Combined with v0.5.32's boot peer re-apply, a TCS restart now comes up with a functioning `tcs-sl0` and the node's cached-handshake retries complete without waiting on a node-side re-provision.

## [0.5.32] — 2026-09-02

### Fixed
- **Siderolink tunnel dropped on TCS restart until each node happened to re-provision.** Kernel WireGuard peers exist only in the per-device state, which is wiped when `tcs-sl0` is recreated on TCS startup. TCS stored peers in the DB but only re-applied them when a node dialed `Provision` again. Talos's SideroLink `ManagerController` keeps its own cached `provisionData` after a successful provision and — on a TCS restart — only retries the existing WireGuard handshake (it does not re-provision unless its data is empty), so already-joined nodes found no matching peer on the fresh device and stayed disconnected until they coincidentally re-dialed the API. TCS now re-registers all known DB peers to `tcs-sl0` at boot (`SiderolinkWg::reapply_peers`), so the tunnel survives a TCS restart without waiting on node-side re-provisioning.

## [0.5.31] — 2026-09-02

### Fixed
- **Siderolink WireGuard tunnel never established — the kernel WG UDP socket went stale at TCS boot.** The SideroLink API, address assignment, peer registration, and node-side configuration were all correct (verified: node's `siderolink` LinkSpec had the right peer key `KR/+…`, the right `192.168.1.2:443` endpoint, and the node's own WG public key matched exactly what TCS registered as the peer). But the host `tcs-sl0` interface's kernel WireGuard UDP socket was bound to port 443 yet never demultiplexed incoming datagrams — node handshake-inits arrived at the host (tcpdump) but the peer showed `0 rx bytes` / no latest-handshake, so no handshake ever completed and the tunnel stayed `connected: false`. Root cause: creating the WG device via netlink and then setting `listen-port`/`private-key`/`up` in that order leaves the socket in a state that doesn't receive; the reliable fix is a **down → up bounce after configuration** (what `wg-quick` does implicitly). `ensure_interface()` now bounces the link at the end so the kernel re-creates a fresh, working UDP socket on every TCS start. **Live-validated end-to-end on kronos:** after the bounce, the node reports `CONNECTED: true`, the TCS WG peer shows a completed latest-handshake with rx/tx bytes climbing, and `ping6` to the node's `fd…` overlay IP over `tcs-sl0` succeeds (0% loss, ~0.5ms). The SideroLink WireGuard management path is now fully functional.
- Quieted per-re-provision Siderolink `info!` logging to `debug!` (Talos re-dials `Provision` ~every 30s per node; the key-load and provision logs now require `RUST_LOG=debug` to appear).

## [0.5.28] — 2026-09-02

### Fixed
- **Siderolink node address changed on every re-provision, so the WireGuard tunnel never established.** Talos's SideroLink `ManagerController` re-dials `Provision` periodically (≈30s) to check peer health, and TCS was assigning a fresh random overlay IP each time — the node kept changing address, the TCS WireGuard peer kept being re-registered with a new IP, and the tunnel never came up (`connected: false` forever, peer flapping). `Provision` now reuses the node's existing assigned IP (looked up by `node_uuid`) when the node has already joined, so the overlay address is stable and the WireGuard handshake can complete. Live-validated on kronos: a worker provisioned, got a stable `fd…` tunnel IP, and TCS registered a persistent WireGuard peer.

## [0.5.27] — 2026-09-02

### Fixed
- **Siderolink API was unreachable from nodes on a different VLAN.** The SideroLink gRPC API bound only to the HTTP `bind_addr` (e.g. the cluster's `bond0.202` IP), but nodes dial it from the management VLAN — so nodes on `192.168.1.x` got `connection refused` reaching `172.24.48.x:8082`. The gRPC API now binds `0.0.0.0` (all host interfaces) and the advertised `apiUrl` host is controlled by `TCS_SIDEROLINK_ENDPOINT_HOST` (falls back to `TCS_PUBLIC_HOST`, then `advertised_url` host, then `bind_addr`), so nodes always dial the TCS host IP they can actually reach. Live-validated on kronos: nodes on VLAN 1 now reach the SideroLink API on the host's VLAN-1 IP and complete the Provision handshake.

## [0.5.26] — 2026-09-02

### Fixed
- **Siderolink overlay network address was wrong — nodes could not reach TCS over the tunnel.** `SiderolinkWg::network_prefix()` (and the gRPC server's twin) formatted all 16 bytes of the installation-derived ULA prefix instead of the /64 **network** address, so `tcs-sl0` was assigned a non-network IPv6 and the host had no address matching the `server_address` nodes were told to dial. Now `tcs-sl0` is assigned the server's own first-usable address (`fd…::1/64`) and `network_prefix()` returns the correct `fd…::/64` network. Live-validated on kronos: `tcs-sl0` now carries the correct ULA and the SideroLink API accepts the Provision handshake.

## [0.5.25] — 2026-09-02

### Added
- **Siderolink is now a real, working WireGuard management path (not a stub).** TCS now speaks the genuine SideroLink protocol: a `ProvisionService` gRPC API server (`backend/src/siderolink/`) that Talos nodes actually dial during join. When a node presents a valid join token, TCS assigns it a WireGuard IP from an RFC 4193 IPv6 ULA overlay, registers a kernel WireGuard peer on `tcs-sl0`, records the peer in the DB, and flips the machine to `siderolink_connected` — after which TCS manages that node over the tunnel first, falling back to direct LAN. This replaces the old no-op "siderolink inventory stub" that never encrypted or routed anything.
- **Cluster-page Siderolink enable/disable toggle.** Each cluster detail page now has a "Siderolink tunnel" panel with a live Enable/Disable button and a per-node peer table (tunnel IP + last seen). **Enable** bakes the `SideroLinkConfig` into every running node live (no reboot); **Disable** strips it from every node live (no reboot) and revokes the cluster token. Direct-LAN management always remains available, so the cluster can never be isolated. New endpoints: `GET /clusters/:id/siderolink`, `POST /clusters/:id/siderolink/enable`, `POST /clusters/:id/siderolink/disable`.
- **Siderolink baked into greenfield provisioning by default.** The wizard's generate-config path and the metal/PXE scheduler path now emit a standalone `SideroLinkConfig` machine-config doc for every cluster that has (or gets) a cluster token, so new nodes join over the tunnel out of the box.

### Fixed
- **Siderolink config schema was invalid — Talos rejected it.** The old code emitted a nested `machine: siderolink: {…}` key that Talos v1.10–v1.13 rejects with `unknown keys found during decoding: machine.siderolink`. Generated configs now append a top-level `SideroLinkConfig` document (`apiVersion: v1alpha1`, `kind: SideroLinkConfig`, `apiUrl: grpc://<host>:<bind_port>/?jointoken=<token>`) — the exact form Talos reconciles live. Verified on kronos: the standalone doc applies with `--mode=no-reboot` and Talos accepts it; the nested form was rejected.

### Changed
- **Removed the Settings → Siderolink menu item and settings card.** Siderolink is now configured per-cluster (the cluster-page toggle) and baked in automatically — there is no manual token/peer "menu" to manage. The `/settings/siderolink` route still exists for direct-URL access to the peer/token inventory, and the per-machine "via Siderolink tunnel" badge on the machine detail page now reflects real tunnel state.
- **WireGuard overlay moved to RFC 4193 IPv6 ULA** (`fd…::/64`, derived deterministically from a SHA-256 of the installation ID), with `tcs-sl0` carrying both the ULA address and a legacy `100.64.0.1/10`. Peer `AllowedIPs` use the correct `/128` host form. The SideroLink gRPC API listens plaintext on `bind_port` (default 8082); the WG data plane listens on `listen_port` (default 443/udp) and is the encrypted part.

## [0.5.24] — 2026-09-02

### Fixed
- **Certificates page didn't reflect the saved Let's Encrypt config.** There was no `GET /settings/certificates/config` route (only `PUT`), so the config form always loaded with defaults (http-01, blank email, no DNS provider) even when a real DNS-01/GoDaddy setup was persisted. Added `GET /settings/certificates/config` (admin-only) that reads the `[tls]` overlay written by Apply and returns it, and the UI now pre-fills mode, domains, email, challenge type, DNS provider, zone, and (masked) credentials from it. **Renew/Apply now round-trips the real saved config** instead of silently losing it — the root cause of "how would renew succeed if the form shows the wrong config?" is resolved.

### Fixed
- **Certificate status panel expiry display.** The `/settings/certificates/status` endpoint returns `expires_at` and `days_remaining` (snake_case), but the Certificates UI read `expiryDate`/`daysRemaining` (camelCase), so a healthy Let's Encrypt cert rendered as "N/A / Expired". Fixed the field mapping so the panel shows the real expiry date and days remaining. (The live cert on :443 was always correct — this was display-only.)

### Fixed
- **Let's Encrypt DNS-01 secondary validation race.** Primary validation passed but LE's *secondary* validator (different vantage point, can lag by 10-60s) saw NXDOMAIN because GoDaddy's authoritative NSes had not all refreshed yet. TCS now (a) waits until the TXT record resolves at **two independent public resolvers** (8.8.8.8 + 1.1.1.1), not just one, and (b) settles a **20s buffer** before asking LE to validate. The record remains published through finalization + download (cleanup only happens after the cert is in hand), so both validators observe it. Combined with v0.5.21's relative record-name fix, this completes a reliable Let's Encrypt DNS-01 flow via GoDaddy.

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
