use std::fmt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BrandingConfig {
    #[serde(default = "default_name")]
    pub name: String,

    #[serde(default = "default_short_name")]
    pub short_name: String,

    #[serde(default = "default_tagline")]
    pub tagline: String,

    #[serde(default = "default_primary_color")]
    pub primary_color: String,

    #[serde(default = "default_secondary_color")]
    pub secondary_color: String,

    #[serde(default = "default_background_color")]
    pub background_color: String,

    #[serde(default = "default_surface_color")]
    pub surface_color: String,

    #[serde(default = "default_text_color")]
    pub text_color: String,

    #[serde(default = "default_text_muted_color")]
    pub text_muted_color: String,

    #[serde(default = "default_font_family")]
    pub font_family: String,

    #[serde(default)]
    pub logo_path: String,

    #[serde(default)]
    pub favicon_path: String,

    #[serde(default)]
    pub docs_url: String,

    #[serde(default)]
    pub support_url: String,
}

impl Default for BrandingConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            short_name: default_short_name(),
            tagline: default_tagline(),
            primary_color: default_primary_color(),
            secondary_color: default_secondary_color(),
            background_color: default_background_color(),
            surface_color: default_surface_color(),
            text_color: default_text_color(),
            text_muted_color: default_text_muted_color(),
            font_family: default_font_family(),
            logo_path: String::new(),
            favicon_path: String::new(),
            docs_url: String::new(),
            support_url: String::new(),
        }
    }
}

fn default_name() -> String {
    "Talos Control System".to_string()
}

fn default_short_name() -> String {
    "TCS".to_string()
}

fn default_tagline() -> String {
    "Kubernetes Management Simplified".to_string()
}

fn default_primary_color() -> String {
    "#150D6A".to_string()
}

fn default_secondary_color() -> String {
    "#4F8BFF".to_string()
}

fn default_background_color() -> String {
    "#0A0A0A".to_string()
}

fn default_surface_color() -> String {
    "#1A1A1A".to_string()
}

fn default_text_color() -> String {
    "#FFFFFF".to_string()
}

fn default_text_muted_color() -> String {
    "#A0A0A0".to_string()
}

fn default_font_family() -> String {
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif".to_string()
}

impl fmt::Display for BrandingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.short_name)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TenantBranding {
    #[serde(default)]
    pub tenant_id: String,

    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub short_name: Option<String>,

    #[serde(default)]
    pub tagline: Option<String>,

    #[serde(default)]
    pub primary_color: Option<String>,

    #[serde(default)]
    pub secondary_color: Option<String>,

    #[serde(default)]
    pub background_color: Option<String>,

    #[serde(default)]
    pub surface_color: Option<String>,

    #[serde(default)]
    pub text_color: Option<String>,

    #[serde(default)]
    pub text_muted_color: Option<String>,

    #[serde(default)]
    pub font_family: Option<String>,

    #[serde(default)]
    pub logo_path: Option<String>,

    #[serde(default)]
    pub favicon_path: Option<String>,

    #[serde(default)]
    pub docs_url: Option<String>,

    #[serde(default)]
    pub support_url: Option<String>,
}

impl TenantBranding {
    pub fn merge_with_defaults(&self, defaults: &BrandingConfig) -> BrandingConfig {
        BrandingConfig {
            name: self.name.clone().unwrap_or_else(|| defaults.name.clone()),
            short_name: self.short_name.clone().unwrap_or_else(|| defaults.short_name.clone()),
            tagline: self.tagline.clone().unwrap_or_else(|| defaults.tagline.clone()),
            primary_color: self.primary_color.clone().unwrap_or_else(|| defaults.primary_color.clone()),
            secondary_color: self.secondary_color.clone().unwrap_or_else(|| defaults.secondary_color.clone()),
            background_color: self.background_color.clone().unwrap_or_else(|| defaults.background_color.clone()),
            surface_color: self.surface_color.clone().unwrap_or_else(|| defaults.surface_color.clone()),
            text_color: self.text_color.clone().unwrap_or_else(|| defaults.text_color.clone()),
            text_muted_color: self.text_muted_color.clone().unwrap_or_else(|| defaults.text_muted_color.clone()),
            font_family: self.font_family.clone().unwrap_or_else(|| defaults.font_family.clone()),
            logo_path: self.logo_path.clone().unwrap_or_else(|| defaults.logo_path.clone()),
            favicon_path: self.favicon_path.clone().unwrap_or_else(|| defaults.favicon_path.clone()),
            docs_url: self.docs_url.clone().unwrap_or_else(|| defaults.docs_url.clone()),
            support_url: self.support_url.clone().unwrap_or_else(|| defaults.support_url.clone()),
        }
    }
}
