pub mod server;
pub mod database;
pub mod branding;
pub mod siderolink;
pub mod auth;
pub mod tls;
pub mod metal;
pub mod factory;

pub use server::ServerConfig;
pub use database::{DatabaseBackend, DatabaseConfig};
pub use branding::BrandingConfig;
pub use siderolink::SideroLinkConfig;
pub use auth::LdapConfig;
pub use auth::OidcConfig;
pub use tls::{SelfSignedConfig, TlsConfig, TlsMode};
pub use metal::{MetalBmcConfig, MetalConfig, MetalDhcpConfig, MetalPxeConfig};
pub use factory::FactoryConfig;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub branding: BrandingConfig,
    pub siderolink: SideroLinkConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub metal: MetalConfig,
    #[serde(default)]
    pub factory: FactoryConfig,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    pub ldap: Option<LdapConfig>,
    pub oidc: Option<OidcConfig>,
    pub saml: Option<crate::config::auth::SamlConfig>,
    #[serde(default = "default_jwt_secret")]
    pub jwt_secret: String,
}

fn default_jwt_secret() -> String {
    if let Ok(secret) = std::env::var("TCS_AUTH_JWT_SECRET") {
        return secret;
    }
    tracing::warn!("Using default JWT secret — set TCS_AUTH_JWT_SECRET in production!");
    "talos-control-system-default-secret-change-in-production".to_string()
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            ldap: None,
            oidc: None,
            saml: None,
            jwt_secret: default_jwt_secret(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let file_path = std::env::var("TCS_CONFIG")
            .unwrap_or_else(|_| "/etc/tcs/config.toml".to_string());

        // TLS overlay written by the Settings UI (writable under ProtectSystem=strict).
        let data_dir = std::env::var("TCS_DATA_DIR").unwrap_or_else(|_| "/var/lib/tcs".into());
        let tls_overlay = std::path::PathBuf::from(data_dir.trim_end_matches('/')).join("tls.toml");

        let mut builder = config::Config::builder()
            .add_source(config::File::from(std::path::PathBuf::from(&file_path)).required(false));

        if tls_overlay.is_file() {
            builder = builder.add_source(config::File::from(tls_overlay.clone()).required(false));
            tracing::info!(path = %tls_overlay.display(), "Loading TLS config overlay");
        }

        let config = builder
            .add_source(config::Environment::with_prefix("TCS").separator("_"))
            .build()?;

        let mut result = config.try_deserialize::<Config>()?;

        if result.server.advertised_url.is_empty() {
            result.server.advertised_url = "https://localhost:443".to_string();
        }

        Ok(result)
    }

    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let file_source = config::File::from(path.as_ref().to_path_buf()).required(false);

        let config = config::Config::builder()
            .add_source(file_source)
            .add_source(config::Environment::with_prefix("TCS").separator("_"))
            .build()?;

        config.try_deserialize()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = config::Config::builder()
            .add_source(config::Environment::with_prefix("TCS").separator("_"))
            .build()?;

        config.try_deserialize()
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }

    pub fn from_default() -> Self {
        Self {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            branding: BrandingConfig::default(),
            siderolink: SideroLinkConfig::default(),
            auth: AuthConfig::default(),
            tls: TlsConfig::default(),
            metal: MetalConfig::default(),
            factory: FactoryConfig::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_default()
    }
}
