//! Talos Image Factory integration: build images with chosen system extensions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FactoryConfig {
    /// Base URL of the Image Factory (public default: https://factory.talos.dev).
    pub base_url: String,
    /// OCI registry host for generated installer images (defaults to the base URL host).
    /// Used to form `factory.talos.dev/metal-installer/<schematic>:<version>` refs.
    pub registry: String,
}

impl Default for FactoryConfig {
    fn default() -> Self {
        Self {
            base_url: "https://factory.talos.dev".to_string(),
            registry: "factory.talos.dev".to_string(),
        }
    }
}

impl FactoryConfig {
    /// Normalize base URL (strip trailing slash, ensure scheme).
    pub fn normalized_base(&self) -> String {
        let mut b = self.base_url.trim_end_matches('/').to_string();
        if !b.starts_with("http://") && !b.starts_with("https://") {
            b = format!("https://{b}");
        }
        b
    }

    /// OCI installer image reference for a schematic + version (includes the extensions).
    pub fn installer_image(&self, schematic: &str, version: &str) -> String {
        let v = if version.starts_with('v') {
            version.to_string()
        } else {
            format!("v{version}")
        };
        format!("{}/metal-installer/{}:{}", self.registry.trim_end_matches('/'), schematic, v)
    }
}
