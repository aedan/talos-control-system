# White-Label Branding Guide

TCS supports full white-label customization. Change colors, logos, fonts, and identity to match your organization's brand — either through configuration at deployment time, or dynamically through the UI.

## Default Branding

Out of the box, TCS ships with:

| Element | Value |
|---------|-------|
| Name | Talos Control System |
| Short Name | TCS |
| Tagline | Kubernetes Management Simplified |
| Primary Color | `#150D6A` (deep indigo) |
| Secondary Color | `#4F8BFF` (bright blue) |
| Background | `#0A0A0A` (near black) |
| Surface | `#1A1A1A` (dark gray) |
| Font | System font stack |

## Customizing via Config

The simplest way to rebrand is via `config.toml` or environment variables:

```toml
[branding]
name = "Acme Cloud Platform"
short_name = "ACP"
tagline = "Your Infrastructure, Managed"
primary_color = "#2563EB"
secondary_color = "#60A5FA"
background_color = "#0F172A"
surface_color = "#1E293B"
text_color = "#F8FAFC"
text_muted_color = "#94A3B8"
font_family = "'Inter', -apple-system, sans-serif"
```

Or via environment variables:

```bash
export TCS_BRANDING_NAME="Acme Cloud Platform"
export TCS_BRANDING_SHORT_NAME="ACP"
export TCS_BRANDING_PRIMARY_COLOR="#2563EB"
```

## Customizing via UI

Authenticated admins can customize branding directly from the **Settings > White-Label Branding** page:

1. Navigate to `/settings/branding`
2. Edit identity fields (name, short name, tagline)
3. Use color pickers for each color variable
4. Preview changes in real-time
5. Click **Save Changes** to persist

Changes applied through the UI are stored in the database and take effect immediately for all users. They override config file values.

## Custom Logos

TCS supports custom logo and favicon files.

### Via Config

```toml
[branding]
logo_path = "/app/branding/logo.svg"
favicon_path = "/app/branding/favicon.svg"
```

The paths are relative to the container or application root. For Docker/Kubernetes deployments, mount the files into the image:

```dockerfile
COPY branding/logo.svg /app/branding/logo.svg
COPY branding/favicon.svg /app/branding/favicon.svg
```

### Via Kubernetes ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
binaryData:
  logo.svg: <base64-encoded-logo>
  favicon.ico: <base64-encoded-favicon>
metadata:
  name: tcs-branding
```

```yaml
# In your deployment:
volumeMounts:
  - name: branding
    mountPath: /app/branding
    readOnly: true
volumes:
  - name: branding
    configMap:
      name: tcs-branding
```

### Via UI Upload

The branding settings page supports uploading logo and favicon files directly. Supported formats:

- **Logo**: SVG (recommended), PNG
- **Favicon**: SVG, ICO, PNG

Uploaded files are stored in the database and served dynamically.

## Custom Links

Add external documentation and support links:

```toml
[branding]
docs_url = "https://docs.yourcompany.com/kubernetes"
support_url = "https://support.yourcompany.com"
```

These appear as links in the application sidebar and footer.

## Per-Tenant Branding

TCS supports serving different branding to different tenants. Each tenant can have its own set of branding overrides that merge with the global defaults.

### API

```bash
curl -X PUT http://localhost:8081/api/branding/tenant/tenant-123 \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Tenant A Platform",
    "primary_color": "#DC2626",
    "secondary_color": "#F87171"
  }'
```

Only the fields you specify are overridden. All other values fall through to the global branding config.

### Tenant Resolution

TCS determines the active tenant via:

1. **Subdomain**: `tenant-a.tcs.example.com` → tenant `tenant-a`
2. **Path prefix**: `/t/tenant-a/...` → tenant `tenant-a`
3. **Header**: `X-Tenant-ID: tenant-123`
4. **JWT claim**: `tenant_id` claim in the authentication token

## CSS Variables

All branding colors are exposed as CSS custom properties for maximum flexibility:

| CSS Variable | Config Key |
|--------------|------------|
| `--tcs-primary` | `primary_color` |
| `--tcs-secondary` | `secondary_color` |
| `--tcs-background` | `background_color` |
| `--tcs-surface` | `surface_color` |
| `--tcs-text` | `text_color` |
| `--tcs-text-muted` | `text_muted_color` |
| `--tcs-font` | `font_family` |

These are applied to `:root` on page load and can be inspected in browser DevTools.

## Branding Precedence

When multiple sources provide branding values, TCS resolves them in this order (highest to lowest priority):

1. **Per-tenant overrides** (from database)
2. **UI-saved branding** (from database)
3. **Config file / environment variables**
4. **Built-in defaults**

## Color Palette Recommendations

### Light Theme (Optional)

For a light theme, consider these values:

```toml
[branding]
background_color = "#F8FAFC"
surface_color = "#FFFFFF"
text_color = "#0F172A"
text_muted_color = "#64748B"
primary_color = "#2563EB"
secondary_color = "#3B82F6"
```

### High Contrast

```toml
[branding]
background_color = "#000000"
surface_color = "#111111"
text_color = "#FFFFFF"
text_muted_color = "#AAAAAA"
primary_color = "#0066FF"
secondary_color = "#00AAFF"
```
