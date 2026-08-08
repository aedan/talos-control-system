use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    #[serde(default = "default_mode")]
    pub mode: TlsMode,

    #[serde(default)]
    pub letsencrypt: Option<LetsEncryptConfig>,

    #[serde(default)]
    pub self_signed: Option<SelfSignedConfig>,

    #[serde(default)]
    pub provided: Option<ProvidedCertConfig>,
}

fn default_enabled() -> bool {
    false
}

fn default_mode() -> TlsMode {
    TlsMode::Disabled
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    LetsEncrypt,
    SelfSigned,
    Provided,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetsEncryptConfig {
    pub domains: Vec<String>,
    pub email: String,

    #[serde(default = "default_challenge")]
    pub challenge_type: ChallengeType,

    #[serde(default)]
    pub dns_provider: Option<DnsProviderConfig>,
}

fn default_challenge() -> ChallengeType {
    ChallengeType::Http01
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChallengeType {
    Http01,
    Dns01,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsProviderConfig {
    #[serde(default = "default_provider")]
    pub provider: String,

    #[serde(default)]
    pub api_key: String,

    #[serde(default)]
    pub api_secret: String,

    #[serde(default)]
    pub api_token: String,

    #[serde(default)]
    pub zone_id: String,
}

fn default_provider() -> String {
    "godaddy".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfSignedConfig {
    #[serde(default = "default_self_signed_domains")]
    pub domains: Vec<String>,
}

fn default_self_signed_domains() -> Vec<String> {
    vec!["localhost".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidedCertConfig {
    pub cert_path: String,
    pub key_path: String,

    #[serde(default)]
    pub ca_path: Option<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: TlsMode::Disabled,
            letsencrypt: None,
            self_signed: None,
            provided: None,
        }
    }
}
