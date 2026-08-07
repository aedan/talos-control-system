use crate::config::BrandingConfig;

pub fn generate_css_variables(branding: &BrandingConfig) -> String {
    format!(r#":root {{
  --tcs-primary: {primary};
  --tcs-primary-rgb: {primary_rgb};
  --tcs-secondary: {secondary};
  --tcs-secondary-rgb: {secondary_rgb};
  --tcs-background: {background};
  --tcs-background-rgb: {background_rgb};
  --tcs-surface: {surface};
  --tcs-surface-rgb: {surface_rgb};
  --tcs-text: {text};
  --tcs-text-rgb: {text_rgb};
  --tcs-text-muted: {muted};
  --tcs-text-muted-rgb: {muted_rgb};
  --tcs-font-family: {font};
  --tcs-brand-name: "{name}";
  --tcs-brand-short: "{short}";
  --tcs-brand-tagline: "{tagline}";
}}

body {{
  font-family: var(--tcs-font-family);
  background-color: var(--tcs-background);
  color: var(--tcs-text);
}}

.tcs-logo {{
  color: var(--tcs-primary);
  font-weight: 700;
}}

.tcs-accent {{
  color: var(--tcs-secondary);
}}

.tcs-muted {{
  color: var(--tcs-text-muted);
}}

.tcs-card {{
  background-color: var(--tcs-surface);
  border: 1px solid rgba(var(--tcs-text-rgb), 0.1);
}}

.tcs-button {{
  background-color: var(--tcs-primary);
  color: var(--tcs-text);
}}

.tcs-button:hover {{
  background-color: var(--tcs-secondary);
}}"#,
        primary = branding.primary_color,
        primary_rgb = hex_to_rgb(&branding.primary_color),
        secondary = branding.secondary_color,
        secondary_rgb = hex_to_rgb(&branding.secondary_color),
        background = branding.background_color,
        background_rgb = hex_to_rgb(&branding.background_color),
        surface = branding.surface_color,
        surface_rgb = hex_to_rgb(&branding.surface_color),
        text = branding.text_color,
        text_rgb = hex_to_rgb(&branding.text_color),
        muted = branding.text_muted_color,
        muted_rgb = hex_to_rgb(&branding.text_muted_color),
        font = branding.font_family,
        name = branding.name,
        short = branding.short_name,
        tagline = branding.tagline,
    )
}

fn hex_to_rgb(hex: &str) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return "255, 255, 255".to_string();
    }

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);

    format!("{}, {}, {}", r, g, b)
}

pub fn generate_tailwind_config(branding: &BrandingConfig) -> serde_json::Value {
    serde_json::json!({
        "theme": {
            "extend": {
                "colors": {
                    "primary": branding.primary_color,
                    "secondary": branding.secondary_color,
                    "background": branding.background_color,
                    "surface": branding.surface_color,
                    "text": branding.text_color,
                    "muted": branding.text_muted_color,
                },
                "fontFamily": {
                    "sans": branding.font_family.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>(),
                }
            }
        }
    })
}
