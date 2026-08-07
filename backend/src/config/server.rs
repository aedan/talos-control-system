use std::fmt;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,

    #[serde(default)]
    pub advertised_url: String,

    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,

    #[serde(default = "default_http_port")]
    pub http_port: u16,

    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_bind_addr(),
            advertised_url: String::new(),
            grpc_port: default_grpc_port(),
            http_port: default_http_port(),
            metrics_port: default_metrics_port(),
        }
    }
}

fn default_bind_addr() -> String {
    "0.0.0.0".to_string()
}

fn default_grpc_port() -> u16 {
    8080
}

fn default_http_port() -> u16 {
    8081
}

fn default_metrics_port() -> u16 {
    9090
}

impl fmt::Display for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Server(bind={}, http:{}, grpc:{}, metrics:{})",
            self.bind_addr, self.http_port, self.grpc_port, self.metrics_port
        )
    }
}
