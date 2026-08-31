//! Talos Image Factory client.
//!
//! Talks to the public factory API (default https://factory.talos.dev) to:
//!   - list buildable Talos versions
//!   - list official system extensions (modules) available for a version
//!   - create a schematic (a set of modules) and get its id
//!   - resolve the OCI installer image ref that bundles a schematic + version
//!
//! The installer image is what `talosctl upgrade --image …` pulls to apply the
//! modules to a running node (reboot included).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppError;

/// Process-wide cache: sorted module list → schematic id. The factory hashes
/// the customization deterministically (verified: identical list ⇒ identical
/// id), so a 50-node rolling upgrade with 3 distinct module sets only hits the
/// factory 3 times.
fn schematic_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, String>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct ImageFactoryClient {
    http: reqwest::Client,
    base: String,
}

/// One official system extension (module) available for a Talos version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactoryExtension {
    pub name: String,
    pub ref_: Option<String>,
    pub digest: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
}

impl ImageFactoryClient {
    pub fn new(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .user_agent("tcs-image-factory/0.5")
                .build()
                .expect("reqwest client"),
            base,
        }
    }

    /// GET /versions → list of buildable Talos versions.
    pub async fn list_versions(&self) -> Result<Vec<String>, AppError> {
        let url = format!("{}/versions", self.base);
        let resp = self.http.get(&url).send().await.map_err(http_err)?;
        if !resp.status().is_success() {
            return Err(http_status_err("list versions", resp.status()));
        }
        let v: Value = resp.json().await.map_err(json_err)?;
        let arr = v
            .as_array()
            .ok_or_else(|| AppError::Network("factory /versions: expected array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect())
    }

    /// GET /version/:version/extensions/official → list of modules for a version.
    pub async fn list_extensions(&self, version: &str) -> Result<Vec<FactoryExtension>, AppError> {
        let v = norm_version(version);
        let url = format!("{}/version/{}/extensions/official", self.base, v);
        let resp = self.http.get(&url).send().await.map_err(http_err)?;
        if !resp.status().is_success() {
            return Err(http_status_err("list extensions", resp.status()));
        }
        let v: Value = resp.json().await.map_err(json_err)?;
        let arr = v
            .as_array()
            .ok_or_else(|| AppError::Network("factory extensions: expected array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|x| {
                let name = x.get("name")?.as_str()?.to_string();
                Some(FactoryExtension {
                    name,
                    ref_: x.get("ref").and_then(|r| r.as_str()).map(String::from),
                    digest: x.get("digest").and_then(|d| d.as_str()).map(String::from),
                    author: x.get("author").and_then(|a| a.as_str()).map(String::from),
                    description: x.get("description").and_then(|d| d.as_str()).map(String::from),
                })
            })
            .collect())
    }

    /// POST /schematics with the chosen modules → returns the schematic id.
    ///
    /// `modules` are official extension names, e.g. ["siderolabs/bnx2-bnx2x"].
    /// An empty list yields the default schematic (no customizations).
    ///
    /// Results are cached per (module-set) for the process lifetime — the
    /// factory's id is a deterministic hash of the module list.
    pub async fn create_schematic(&self, modules: &[String]) -> Result<String, AppError> {
        let mut sorted: Vec<&str> = modules.iter().map(|s| s.as_str()).collect();
        sorted.sort_unstable();
        sorted.dedup();
        let key = sorted.join("\u{0}");

        if let Some(cache) = schematic_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
            return Ok(cache);
        }

        let url = format!("{}/schematics", self.base);
        // Build the YAML body the factory expects.
        let mut body = String::from("customization:\n");
        if sorted.is_empty() {
            // default schematic (well-known, no customization)
            body.push_str("    extraKernelArgs: []\n");
        } else {
            body.push_str("    systemExtensions:\n        officialExtensions:\n");
            for m in &sorted {
                body.push_str(&format!("          - {m}\n"));
            }
        }

        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "application/yaml")
            .body(body)
            .send()
            .await
            .map_err(http_err)?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Network(format!(
                "factory create schematic HTTP {status}: {text}"
            )));
        }
        let v: Value = resp.json().await.map_err(json_err)?;
        let id = v
            .get("id")
            .and_then(|i| i.as_str())
            .ok_or_else(|| AppError::Network("factory create schematic: missing id".into()))?
            .to_string();
        if let Ok(mut cache) = schematic_cache().lock() {
            cache.insert(key, id.clone());
        }
        Ok(id)
    }
}

fn norm_version(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

fn http_err(e: reqwest::Error) -> AppError {
    AppError::Network(format!("Image Factory request failed: {e}"))
}
fn json_err(e: reqwest::Error) -> AppError {
    AppError::Network(format!("Image Factory JSON decode failed: {e}"))
}
fn http_status_err(what: &str, status: reqwest::StatusCode) -> AppError {
    AppError::Network(format!("Image Factory {what} HTTP {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_image_ref() {
        let cfg = crate::config::FactoryConfig::default();
        assert_eq!(
            cfg.installer_image("abc123", "v1.13.7"),
            "factory.talos.dev/metal-installer/abc123:v1.13.7"
        );
        assert_eq!(
            cfg.installer_image("abc123", "1.13.7"),
            "factory.talos.dev/metal-installer/abc123:v1.13.7"
        );
    }

    #[test]
    fn normalized_base() {
        let mut cfg = crate::config::FactoryConfig::default();
        cfg.base_url = "factory.talos.dev/".into();
        assert_eq!(cfg.normalized_base(), "https://factory.talos.dev");
    }
}
