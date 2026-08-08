# TLS Configuration

TCS supports four TLS modes for securing the HTTP server. When TLS is enabled, TCS listens on the configured `http_port` with HTTPS. When using Let's Encrypt with HTTP-01 challenges, an additional listener is opened on port 80 for ACME validation.

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
```

Restart TCS after placing certificates:

```bash
sudo systemctl restart tcs
```

## Firewall Requirements

| Mode | Ports Required |
|------|---------------|
| `disabled` | `http_port` (default 8081) |
| `letsencrypt` (http-01) | 80 (ACME) + `http_port` |
| `letsencrypt` (dns-01) | `http_port` only |
| `self-signed` | `http_port` |
| `provided` | `http_port` |

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
curl -X POST http://localhost:8081/api/settings/certificates/renew \
  -H "Authorization: Bearer $TOKEN"
```

### Self-signed cert warnings

Self-signed certificates are not trusted by browsers. For development, you can add the certificate to your system's trust store, or use Let's Encrypt with a valid domain.
