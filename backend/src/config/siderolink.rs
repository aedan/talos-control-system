#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SideroLinkConfig {
    #[serde(default = "default_bind_port")]
    pub bind_port: u16,

    #[serde(default = "default_listen_port")]
    pub listen_port: u16,

    #[serde(default = "default_mtu")]
    pub mtu: u16,

    #[serde(default = "default_subnet")]
    pub subnet: String,

    #[serde(default)]
    pub rate_limit_bytes: u64,
}

impl Default for SideroLinkConfig {
    fn default() -> Self {
        Self {
            bind_port: default_bind_port(),
            listen_port: default_listen_port(),
            mtu: default_mtu(),
            subnet: default_subnet(),
            rate_limit_bytes: 0,
        }
    }
}

fn default_bind_port() -> u16 {
    8082
}

fn default_listen_port() -> u16 {
    443
}

fn default_mtu() -> u16 {
    1420
}

fn default_subnet() -> String {
    "100.64.0.0/10".to_string()
}
