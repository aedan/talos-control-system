# TLS Configuration

TCS **always** listens on **:443 (HTTPS)** and **:80 (HTTP → redirect / ACME challenges)** — there is no `http_port` listener. A TLS "mode" only decides *which certificate* serves :443: a fresh install with no `[tls]` (or `mode = "disabled"`) auto-generates a **self-signed** cert so HTTPS works immediately, and you can switch to a real cert later **live** (no restart) via Settings → Certificates.

## TLS Modes

| Mode | Description |
|------|-------------|
| `letsencrypt` | Automatic certificate provisioning via Let's Encrypt ACME protocol |
| `self-signed` | Self-signed certificates generated at boot with `rcgen` |
| `provided` | Operator-supplied PEM certificates |
| `disabled` | No TLS, HTTP-only mode (default) |

## Configuration

```toml
[tls]
enabled = true
mode = "letsencrypt"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"

[tls.letsencrypt.dns_provider]
provider = "godaddy"
api_key = "YOUR_API_KEY"
api_secret = "YOUR_API_SECRET"

[tls.self-signed]
domains = ["localhost"]

[tls.provided]
cert_path = "/etc/tls/cert.pem"
key_path = "/etc/tls/key.pem"
# ca_path = "/etc/tls/ca.pem"   # optional CA chain
```

## ACME Challenges

### HTTP-01

TCS serves `/.well-known/acme-challenge/*` on port 80. Ensure port 80 is accessible from the internet. This is the simplest option and requires no DNS provider configuration.

```toml
[tls]
enabled = true
mode = "letsencrypt"

[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "http-01"
```

### DNS-01

For DNS-01 challenges, TCS configures TXT records through a supported DNS provider. This is useful when port 80 is not accessible from the internet.

Supported providers:
- **GoDaddy** — Configure `api_key` (account number) and `api_secret` (API key)
- **Cloudflare** — Configure with Cloudflare API token

```toml
[tls.letsencrypt]
domains = ["tcs.example.com"]
email = "admin@example.com"
challenge_type = "dns-01"

[tls.letsencrypt.dns_provider]
provider = "godaddy"
api_key = "YOUR_ACCOUNT_NUMBER"
api_secret = "YOUR_API_KEY"
```

## Settings UI (live reload)

Saving TLS settings from **Settings → Certificates**:

1. Writes a TLS overlay to `/var/lib/tcs/tls.toml` (writable under the default systemd unit).
2. Best-effort merges into `/etc/tcs/config.toml` when that path is writable.
3. **Hot-reloads** the HTTPS listener when TCS already started with TLS (self-signed, LE, or provided). New certs apply on the next TLS handshake — **no restart**.

Response fields:

| Field | Meaning |
|-------|---------|
| `appliedLive: true` | Cert reloaded in-process |
| `restartRequired: true` | Saved only (e.g. process was HTTP-only, or live ACME failed) |

```bash
# Only needed when going from TLS disabled → enabled (bind :443), or if live apply failed:
sudo systemctl restart tcs
journalctl -u tcs -f
```

### Let's Encrypt HTTP-01 checklist

- DNS A/AAAA for the domain points at this host
- Port **80** reachable from the internet (challenge); **443** for HTTPS
- `email` and `domains` filled in the UI (or config)
- After **Apply**, logs should show live ACME issuance (or a clear ACME error) without requiring a restart

## Certificate Renewal

A background tokio task runs daily checking certificate expiry. Renewal is triggered when less than 30 days remain. Renewal logs appear in the systemd journal:

```bash
journalctl -u tcs | grep -i renew
```

## Using Provided Certificates

When using your own certificates, place them on disk and reference them in config:

```bash
sudo mkdir -p /etc/tcs/tls
sudo cp cert.pem /etc/tcs/tls/cert.pem
sudo cp key.pem /etc/tcs/tls/key.pem
sudo chown -R root:root /etc/tcs/tls
sudo chmod 600 /etc/tcs/tls/key.pem
```

```toml
[tls]
enabled = true
mode = "provided"

[tls.provided]
cert_path = "/etc/tcs/tls/cert.pem"
key_path = "/etc/tcs/tls/key.pem"
# ca_path = "/etc/tls/ca.pem"   # optional CA chain
```

Restart TCS after placing certificates:

```bash
sudo systemctl restart tcs
```

## Firewall Requirements

TCS always needs **443 (HTTPS)**. It also opens **80 (HTTP)** for the redirect and, when using Let's Encrypt HTTP-01, for ACME validation.

| Mode | Ports Required |
|------|---------------|
| `disabled` / self-signed | 443 + 80 (self-signed :443) |
| `letsencrypt` (http-01) | 443 + 80 (ACME) |
| `letsencrypt` (dns-01) | 443 + 80 (redirect; 80 optional) |
| `provided` | 443 + 80 |

## Troubleshooting

### ACME validation fails

Ensure port 80 is reachable from the internet and not blocked by a firewall:

```bash
# Test from an external host
curl -v http://tcs.example.com/.well-known/acme-challenge/test
```

### Certificate not renewing

Check the journal for renewal task logs:

```bash
journalctl -u tcs --since "1 day ago" | grep -i "renew\|cert\|acme"
```

Force a manual renewal by toggling the certificate settings via the API:

```bash
curl -k -X POST https://localhost:443/api/settings/certificates/renew \
  -H "Authorization: Bearer $TOKEN"
```

### Self-signed cert warnings

Self-signed certificates are not trusted by browsers. For development, you can add the certificate to your system's trust store, or use Let's Encrypt with a valid domain.
