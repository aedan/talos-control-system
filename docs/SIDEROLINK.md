# Siderolink

Siderolink is Talos Linux's built-in WireGuard tunnel integration. TCS acts as
the Siderolink server: it manages a WireGuard interface on the host, assigns
tunnel IPs to registering machines, and provides join tokens for authentication.

This lets Talos nodes behind NAT or firewalls reach the Kubernetes API and be
managed by TCS without exposing them publicly.

## Prerequisites

TCS must run as **root** (the default systemd service does) and have
`wireguard-tools` installed on the host:

```bash
# Debian/Ubuntu
sudo apt-get install -y wireguard-tools
```

Without `wireguard-tools`, TCS still works in inventory-only mode — peers
register and get IPs, but the WireGuard tunnel is not created.

## Configuration

```toml
[siderolink]
bind_port = 8082               # Internal port for WG listener
listen_port = 443               # Port advertised to machines
mtu = 1420                      # Tunnel MTU
subnet = "100.64.0.0/10"        # CGNAT subnet for machine IPs
rate_limit_bytes = 0            # Rate limit (0 = unlimited)
```

### Environment variables

| Variable | Purpose |
|---|---|
| `TCS_SIDEROLINK_ENDPOINT` | Override the advertised endpoint (e.g. `tcs.example.com:8082`). Without this, TCS guesses from `TCS_PUBLIC_HOST`. |
| `TCS_PUBLIC_HOST` | Fallback hostname used in the endpoint hint. |
| `TCS_SIDEROLINK_IFACE` | WireGuard interface name (default: `tcs-sl0`). |

### Firewall

The `bind_port` (default `8082`) must be reachable from Talos nodes. UDP traffic
must be allowed:

```bash
# ufw example
sudo ufw allow 8082/udp
```

## Creating join tokens

Join tokens are one-time-use credentials that machines use to register with TCS.

**From the UI:** Settings → Siderolink → "Create join token" section.
Enter a label (e.g. `prod-batch-1`) and expiry in hours (default 168 = 7 days).
Click **Create token**. Copy the token immediately — it's only shown once.

**Via API:**

```bash
curl -X POST https://devstation.jakelab.info/api/siderolink/tokens \
  -H "Authorization: Bearer $TCS_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"label": "prod-batch-1", "expiresHours": 168}'
```

Response:
```json
{ "token": "abc123def456..." }
```

## Configuring a Talos machine

Add a `siderolink` section to the machine's Talos configuration
(either `machineFiles` or inline in the config patch):

```yaml
siderolink:
  enabled: true
  endpoint: devstation.jakelab.info:8082
  token: abc123def456...
```

The endpoint must be the TCS server's hostname and the `bind_port` from
configuration. The token is the join token created above.

Apply the config:

```bash
talosctl apply-config --nodes <node-ip> --file talosconfig.yaml
```

The machine will register with TCS and receive:
- An assigned tunnel IP (e.g. `100.64.0.2`)
- WireGuard server public key
- Connection parameters (listen port, allowed IPs, keepalive)

## Verifying connections

**From the UI:** Settings → Siderolink → "Registered peers" table shows all
registered machines with their assigned IPs and last-seen timestamps.

**From the WireGuard interface:**

```bash
# On the TCS host
sudo wg show tcs-sl0
```

This shows the WireGuard interface state, peer public keys, endpoint IPs,
transfer bytes, and last handshake time.

**Via API:**

```bash
# List peers
curl https://devstation.jakelab.info/api/siderolink/peers \
  -H "Authorization: Bearer $TCS_TOKEN"

# List tokens
curl https://devstation.jakelab.info/api/siderolink/tokens \
  -H "Authorization: Bearer $TCS_TOKEN"
```

## Troubleshooting

### WireGuard not active

Check TCS logs for the WireGuard initialization message:

```bash
journalctl -u tcs | grep -i siderolink
```

Expected output when active:
```
Siderolink WireGuard interface ready iface=tcs-sl0 port=8082
```

If you see "Siderolink WireGuard not active" instead:
- Verify `wireguard-tools` is installed: `wg --version`
- Verify TCS runs as root: `systemctl status tcs`
- Check the TCS data dir is writable (default `/var/lib/tcs`)

### Machine can't connect

1. **DNS**: Ensure the Talos node can resolve the TCS endpoint hostname.
2. **Firewall**: Verify UDP `bind_port` is open from the node to TCS:
   ```bash
   nc -uvu devstation.jakelab.info 8082
   ```
3. **Token expired**: Create a new join token and re-apply the machine config.
4. **Check WG state**: On the TCS host, `sudo wg show tcs-sl0` should show the
   peer's latest handshake. If it's been hours, the tunnel is stale.

### Peers show in inventory but not in `wg show`

This happens when `wireguard-tools` isn't installed. The registration still
works (IP allocation, DB record) but the tunnel isn't created. Install
`wireguard-tools` and restart TCS.

## API reference

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/api/siderolink/register` | None | Machine registration (public) |
| `GET` | `/api/siderolink/peers` | Admin | List registered peers |
| `GET` | `/api/siderolink/tokens` | Admin | List join tokens |
| `POST` | `/api/siderolink/tokens` | Admin | Create join token |

### Register request body

```json
{
  "token": "abc123...",
  "systemUuid": "550e8400-e29b-41d4-a716-446655440000",
  "publicKey": "base64wgpublickey..."
}
```

### Register response

```json
{
  "peerId": "...",
  "assignedIp": "100.64.0.2",
  "systemUuid": "550e8400-...",
  "wireguard": {
    "enabled": true,
    "serverPublicKey": "base64serverpubkey...",
    "endpoint": "devstation.jakelab.info:8082",
    "listenPort": 8082,
    "allowedIps": "100.64.0.0/10",
    "persistentKeepalive": 25
  }
}
```
