# TLS Configuration

TCS uses a dual-server architecture: an HTTP server on port 80 for ACME challenges and HTTPS redirects, and an HTTPS server on port 443 with rustls-backed TLS.

## TLS Modes

| Mode | Description |
|------|-------------|
| `letsencrypt` | Automatic certificate provisioning via Let's Encrypt ACME protocol |
| `self-signed` | Self-signed certificates generated at boot with `rcgen` |
| `provided` | Operator-supplied PEM certificates |
| `disabled` | No TLS, HTTP-only mode (default port 8081) |

## Config.toml

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
api_key = "..."
api_secret = "..."

[tls.self-signed]
domains = ["localhost"]

[tls.provided]
cert_path = "/etc/tls/cert.pem"
key_path = "/etc/tls/key.pem"
```

## ACME Challenges

### HTTP-01
TCS serves `/.well-known/acme-challenge/*` on port 80. Ensure port 80 is accessible from the internet.

### DNS-01
Supported providers: GoDaddy, Cloudflare, Route53 (stub). Configure via `dns_provider` section.

## Certificate Renewal

A background tokio task runs daily (`RENEWAL_CHECK_INTERVAL`) checking certificate expiry. Renewal triggers when <30 days remain.

## Helm Deployment

```yaml
tls:
  enabled: true
  mode: letsencrypt
  letsencrypt:
    email: "admin@example.com"
    domains: ["tcs.example.com"]
    challengeType: "http-01"
```

For provided certs:
```yaml
tls:
  enabled: true
  mode: provided
  provided:
    secretName: "my-tls-secret"
```
