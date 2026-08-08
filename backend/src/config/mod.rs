pub mod server;
pub mod database;
pub mod branding;
pub mod siderolink;
pub mod auth;
pub mod tls;

pub use server::ServerConfig;
pub use database::{DatabaseBackend, DatabaseConfig};
pub use branding::BrandingConfig;
pub use siderolink::SideroLinkConfig;
pub use auth::LdapConfig;
pub use auth::OidcConfig;
pub use tls::{TlsConfig, TlsMode};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub branding: BrandingConfig,
    pub siderolink: SideroLinkConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub tls: TlsConfig,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    pub ldap: Option<LdapConfig>,
    pub oidc: Option<OidcConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            ldap: None,
            oidc: None,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config = config::Config::builder()
            .add_source(config::Environment::with_prefix("TCS").separator("_"))
            .build()?;

        let mut result = config.try_deserialize::<Config>()?;

        if result.server.advertised_url.is_empty() {
            result.server.advertised_url = format!("http://localhost:{}", result.server.http_port);
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
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_default()
    }
}
