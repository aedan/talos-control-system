//! Real Talos Linux machine API client over gRPC + mTLS.
//!
//! Uses [`talos-rust-client`] against the standard machine API (port 50000).

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use talos_rust_client::machine::apply_configuration_request::Mode as ApplyMode;
use talos_rust_client::machine::reboot_request::Mode as RebootMode;
use talos_rust_client::machine::{
    ApplyConfigurationRequest, EtcdSnapshotRequest, RebootRequest, UpgradeRequest,
};
use talos_rust_client::talosconfig::TalosConfig;
use talos_rust_client::{MachineServiceClient, TalosConnector};
use tracing::{info, warn};

use crate::AppError;

/// Credentials + endpoints from a talosconfig document.
#[derive(Debug, Clone)]
pub struct TalosCredentials {
    pub ca: Vec<u8>,
    pub crt: Vec<u8>,
    pub key: Vec<u8>,
    pub endpoints: Vec<String>,
    pub nodes: Vec<String>,
}

impl TalosCredentials {
    /// Parse a talosconfig YAML string (same format as `~/.talos/config`).
    pub fn from_talosconfig_yaml(yaml: &str) -> Result<Self, AppError> {
        let config: TalosConfig = serde_yaml::from_str(yaml).map_err(|e| {
            AppError::InvalidInput(format!("Invalid talosconfig YAML: {}", e))
        })?;

        let context = config
            .contexts
            .get(&config.context)
            .ok_or_else(|| {
                AppError::InvalidInput(format!(
                    "talosconfig context '{}' not found",
                    config.context
                ))
            })?;

        let ca = decode_b64(&context.ca)?;
        let crt = decode_b64(&context.crt)?;
        let key = decode_b64(&context.key)?;

        if context.endpoints.is_empty() {
            return Err(AppError::InvalidInput(
                "talosconfig has no endpoints".to_string(),
            ));
        }

        Ok(Self {
            ca,
            crt,
            key,
            endpoints: context.endpoints.clone(),
            nodes: context.nodes.clone(),
        })
    }
}

fn decode_b64(data: &str) -> Result<Vec<u8>, AppError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| AppError::InvalidInput(format!("Invalid base64 in talosconfig: {}", e)))
}

/// Normalize Talos client key PEM for rustls `Identity::from_pem`.
///
/// talosconfig often uses `BEGIN ED25519 PRIVATE KEY` whose body is already
/// PKCS#8 DER. rustls only accepts the generic PKCS#8 labels
/// (`BEGIN PRIVATE KEY`), so relabel when needed.
fn ensure_pkcs8_pem(key_pem: &[u8]) -> Result<Vec<u8>, AppError> {
    let s = String::from_utf8_lossy(key_pem);
    if s.contains("BEGIN ED25519 PRIVATE KEY") {
        let fixed = s
            .replace("BEGIN ED25519 PRIVATE KEY", "BEGIN PRIVATE KEY")
            .replace("END ED25519 PRIVATE KEY", "END PRIVATE KEY");
        return Ok(fixed.into_bytes());
    }
    Ok(key_pem.to_vec())
}

/// Normalize a host/IP (or host:port / URL) into a Talos gRPC endpoint URL.
pub fn normalize_endpoint(host_or_url: &str) -> String {
    let s = host_or_url.trim();
    if s.starts_with("https://") || s.starts_with("http://") {
        return s.to_string();
    }
    if s.contains(':') {
        format!("https://{}", s)
    } else {
        format!("https://{}:50000", s)
    }
}

/// Client for a single Talos node (or any endpoint reachable with the given certs).
pub struct TalosClient {
    endpoint: String,
    ca: Vec<u8>,
    crt: Vec<u8>,
    key: Vec<u8>,
}

impl TalosClient {
    pub fn new(node_address: String, ca: Vec<u8>, crt: Vec<u8>, key: Vec<u8>) -> Self {
        Self {
            endpoint: normalize_endpoint(&node_address),
            ca,
            crt,
            key,
        }
    }

    pub fn from_credentials(node_address: &str, creds: &TalosCredentials) -> Self {
        Self::new(
            node_address.to_string(),
            creds.ca.clone(),
            creds.crt.clone(),
            creds.key.clone(),
        )
    }

    /// Prefer an explicit machine address; fall back to the first talosconfig endpoint.
    pub fn for_machine(address: Option<&str>, creds: &TalosCredentials) -> Result<Self, AppError> {
        let host = address
            .filter(|a| !a.is_empty())
            .map(|s| s.to_string())
            .or_else(|| creds.endpoints.first().cloned())
            .ok_or_else(|| {
                AppError::InvalidInput(
                    "No Talos endpoint: set machine address or attach a talosconfig".to_string(),
                )
            })?;
        Ok(Self::from_credentials(&host, creds))
    }

    async fn connect(&self) -> Result<MachineServiceClient<talos_rust_client::Channel>, AppError> {
        // rustls Identity::from_pem rejects OpenSSL "BEGIN ED25519 PRIVATE KEY"
        // (common in talosconfig). Convert to PKCS#8 labels first.
        let key = ensure_pkcs8_pem(&self.key)?;

        // Dial by IP is normal for Talos. Node server certs include IP SANs, so
        // leave server_name unset and let TalosConnector use the URL host (IP).
        // Do NOT force "localhost" — that fails SAN verification.
        let channel = TalosConnector::new(&self.endpoint)
            .ca_pem(self.ca.clone())
            .cert_pem(self.crt.clone())
            .key_pem(key)
            .connect()
            .await
            .map_err(|e| {
                AppError::Network(format!(
                    "Failed to connect to Talos API at {}: {}",
                    self.endpoint, e
                ))
            })?;

        Ok(MachineServiceClient::new(channel)
            .max_decoding_message_size(64 * 1024 * 1024))
    }

    pub async fn get_version(&self) -> Result<String, AppError> {
        let mut client = self.connect().await?;
        let request = tonic::Request::new(talos_rust_client::generated::google::protobuf::Empty {});
        let response = client.version(request).await.map_err(|e| {
            AppError::Grpc(format!("Version RPC failed on {}: {}", self.endpoint, e))
        })?;

        let inner = response.into_inner();
        for msg in &inner.messages {
            if let Some(v) = &msg.version {
                if !v.tag.is_empty() {
                    info!(endpoint = %self.endpoint, tag = %v.tag, "Talos version");
                    return Ok(v.tag.clone());
                }
            }
        }

        Err(AppError::Grpc(format!(
            "Version RPC returned no tag from {}",
            self.endpoint
        )))
    }

    /// Apply machine configuration or a strategic-merge config patch.
    pub async fn apply_config(&self, config: &str) -> Result<(), AppError> {
        self.apply_config_with_options(config, false).await
    }

    pub async fn apply_config_with_options(
        &self,
        config: &str,
        dry_run: bool,
    ) -> Result<(), AppError> {
        let mut client = self.connect().await?;
        let request = ApplyConfigurationRequest {
            data: config.as_bytes().to_vec(),
            mode: ApplyMode::NoReboot as i32,
            dry_run,
            try_mode_timeout: None,
        };

        let response = client
            .apply_configuration(request)
            .await
            .map_err(|e| {
                AppError::Grpc(format!(
                    "ApplyConfiguration failed on {}: {}",
                    self.endpoint, e
                ))
            })?;

        let inner = response.into_inner();
        for msg in &inner.messages {
            if let Some(meta) = &msg.metadata {
                if !meta.error.is_empty() {
                    return Err(AppError::Grpc(format!(
                        "ApplyConfiguration error on {}: {}",
                        self.endpoint, meta.error
                    )));
                }
            }
            if !msg.mode_details.is_empty() {
                info!(
                    endpoint = %self.endpoint,
                    details = %msg.mode_details,
                    "Config applied"
                );
            }
            for w in &msg.warnings {
                warn!(endpoint = %self.endpoint, warning = %w, "ApplyConfiguration warning");
            }
        }

        info!(endpoint = %self.endpoint, config_len = config.len(), "Config applied");
        Ok(())
    }

    pub async fn reboot(&self) -> Result<(), AppError> {
        let mut client = self.connect().await?;
        let request = RebootRequest {
            mode: RebootMode::Default as i32,
        };
        client.reboot(request).await.map_err(|e| {
            AppError::Grpc(format!("Reboot failed on {}: {}", self.endpoint, e))
        })?;
        info!(endpoint = %self.endpoint, "Reboot initiated");
        Ok(())
    }

    pub async fn upgrade(&self, image: &str) -> Result<(), AppError> {
        use talos_rust_client::machine::upgrade_request::RebootMode as UpgradeRebootMode;
        let mut client = self.connect().await?;
        let request = UpgradeRequest {
            image: image.to_string(),
            preserve: true,
            stage: false,
            force: false,
            reboot_mode: UpgradeRebootMode::Default as i32,
        };
        client.upgrade(request).await.map_err(|e| {
            AppError::Grpc(format!("Upgrade failed on {}: {}", self.endpoint, e))
        })?;
        info!(endpoint = %self.endpoint, image, "Upgrade initiated");
        Ok(())
    }

    /// List machined services (apid, etcd, kubelet, …) on this node.
    pub async fn service_list(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let mut client = self.connect().await?;
        let request =
            tonic::Request::new(talos_rust_client::generated::google::protobuf::Empty {});
        let response = client.service_list(request).await.map_err(|e| {
            AppError::Grpc(format!("ServiceList failed on {}: {}", self.endpoint, e))
        })?;

        let mut out = Vec::new();
        for msg in response.into_inner().messages {
            for svc in msg.services {
                let healthy = svc
                    .health
                    .as_ref()
                    .map(|h| h.healthy)
                    .unwrap_or(false);
                let unknown = svc
                    .health
                    .as_ref()
                    .map(|h| h.unknown)
                    .unwrap_or(true);
                out.push(serde_json::json!({
                    "id": svc.id,
                    "state": svc.state,
                    "healthy": healthy,
                    "unknown": unknown,
                }));
            }
        }
        out.sort_by(|a, b| {
            a.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.get("id").and_then(|v| v.as_str()).unwrap_or(""))
        });
        Ok(out)
    }

    pub async fn hostname(&self) -> Result<String, AppError> {
        let mut client = self.connect().await?;
        let request =
            tonic::Request::new(talos_rust_client::generated::google::protobuf::Empty {});
        let response = client.hostname(request).await.map_err(|e| {
            AppError::Grpc(format!("Hostname failed on {}: {}", self.endpoint, e))
        })?;
        for msg in response.into_inner().messages {
            if !msg.hostname.is_empty() {
                return Ok(msg.hostname);
            }
        }
        Err(AppError::Grpc(format!(
            "Hostname RPC returned empty on {}",
            self.endpoint
        )))
    }

    /// Stream an etcd snapshot from a control-plane node to `dest_path`.
    pub async fn etcd_snapshot(&self, dest_path: &Path) -> Result<u64, AppError> {
        let mut client = self.connect().await?;
        let request = EtcdSnapshotRequest {};
        let mut stream = client
            .etcd_snapshot(request)
            .await
            .map_err(|e| {
                AppError::Grpc(format!(
                    "EtcdSnapshot failed on {} (control-plane only): {}",
                    self.endpoint, e
                ))
            })?
            .into_inner();

        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Io(std::io::Error::new(
                    e.kind(),
                    format!("create backup dir {}: {}", parent.display(), e),
                ))
            })?;
        }

        let mut file = tokio::fs::File::create(dest_path).await.map_err(AppError::Io)?;
        let mut total: u64 = 0;

        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = stream.next().await {
            let data = chunk.map_err(|e| {
                AppError::Grpc(format!("EtcdSnapshot stream error on {}: {}", self.endpoint, e))
            })?;
            if !data.bytes.is_empty() {
                file.write_all(&data.bytes).await.map_err(AppError::Io)?;
                total += data.bytes.len() as u64;
            }
        }
        file.flush().await.map_err(AppError::Io)?;

        if total == 0 {
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err(AppError::Grpc(format!(
                "EtcdSnapshot from {} returned empty stream",
                self.endpoint
            )));
        }

        info!(
            endpoint = %self.endpoint,
            path = %dest_path.display(),
            bytes = total,
            "Etcd snapshot written"
        );
        Ok(total)
    }

    /// Upload an etcd snapshot to a control-plane node (EtcdRecover client stream).
    /// The node stores the snapshot for a subsequent bootstrap with recover_etcd.
    pub async fn etcd_recover(&self, snapshot_path: &Path) -> Result<u64, AppError> {
        use talos_rust_client::common::Data;

        let mut client = self.connect().await?;
        let bytes = tokio::fs::read(snapshot_path).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                e.kind(),
                format!("read snapshot {}: {}", snapshot_path.display(), e),
            ))
        })?;
        if bytes.is_empty() {
            return Err(AppError::InvalidInput("Snapshot file is empty".to_string()));
        }

        const CHUNK: usize = 256 * 1024;
        let chunks: Vec<Data> = bytes
            .chunks(CHUNK)
            .map(|c| Data {
                metadata: None,
                bytes: c.to_vec(),
            })
            .collect();
        let total = bytes.len() as u64;
        let stream = futures_util::stream::iter(chunks);

        client
            .etcd_recover(stream)
            .await
            .map_err(|e| {
                AppError::Grpc(format!(
                    "EtcdRecover failed on {} (control-plane only): {}",
                    self.endpoint, e
                ))
            })?;

        info!(
            endpoint = %self.endpoint,
            path = %snapshot_path.display(),
            bytes = total,
            "Etcd snapshot uploaded (recover)"
        );
        Ok(total)
    }

    /// Bootstrap the control plane with etcd recovery from a previously uploaded snapshot.
    /// Destructive: only use during disaster recovery on a control-plane node.
    pub async fn bootstrap_recover_etcd(&self, skip_hash_check: bool) -> Result<(), AppError> {
        use talos_rust_client::machine::BootstrapRequest;

        let mut client = self.connect().await?;
        let request = BootstrapRequest {
            recover_etcd: true,
            recover_skip_hash_check: skip_hash_check,
        };
        let response = client.bootstrap(request).await.map_err(|e| {
            AppError::Grpc(format!(
                "Bootstrap(recover_etcd) failed on {}: {}",
                self.endpoint, e
            ))
        })?;

        for msg in response.into_inner().messages {
            if let Some(meta) = &msg.metadata {
                if !meta.error.is_empty() {
                    return Err(AppError::Grpc(format!(
                        "Bootstrap recover error on {}: {}",
                        self.endpoint, meta.error
                    )));
                }
            }
        }

        info!(endpoint = %self.endpoint, "Bootstrap etcd recover initiated");
        Ok(())
    }

    /// Best-effort read of the running machine config.
    ///
    /// On modern Talos the live config is a COSI resource (`MachineConfigs`), not a
    /// file under `/system/state`. Prefer `talosctl get mc` when available; fall back
    /// to the historical file path for older images.
    pub async fn get_machine_config(&self) -> Result<String, AppError> {
        match self.get_machine_config_via_talosctl().await {
            Ok(s) if !s.trim().is_empty() => return Ok(s),
            Ok(_) => {}
            Err(e) => {
                warn!(endpoint = %self.endpoint, error = %e, "talosctl get mc failed; trying file read");
            }
        }

        let mut client = self.connect().await?;
        let request = talos_rust_client::machine::ReadRequest {
            path: "/system/state/config.yaml".to_string(),
        };
        let mut stream = client
            .read(request)
            .await
            .map_err(|e| {
                AppError::Grpc(format!(
                    "Read config failed on {}: {}",
                    self.endpoint, e
                ))
            })?
            .into_inner();

        let mut buf = Vec::new();
        while let Some(chunk) = stream.next().await {
            let data = chunk.map_err(|e| {
                AppError::Grpc(format!("Read stream error on {}: {}", self.endpoint, e))
            })?;
            buf.extend_from_slice(&data.bytes);
        }

        String::from_utf8(buf).map_err(|e| {
            AppError::Internal(format!("Machine config is not valid UTF-8: {}", e))
        })
    }

    /// Apply a strategic-merge style config patch (same as `talosctl patch mc`).
    ///
    /// Partial YAML docs are not accepted by `ApplyConfiguration` alone on Talos 1.13+
    /// multi-document configs; `talosctl patch mc` merges against the live resource.
    pub async fn apply_config_patch(&self, patch_yaml: &str, dry_run: bool) -> Result<(), AppError> {
        let node = self.node_host()?;
        let (talosconfig_path, patch_path, _tmpdir) = self.write_talosctl_workspace(patch_yaml)?;

        let mut cmd = tokio::process::Command::new("talosctl");
        cmd.arg("--talosconfig")
            .arg(&talosconfig_path)
            .arg("-n")
            .arg(&node)
            .arg("patch")
            .arg("mc")
            .arg("--patch")
            .arg(format!("@{}", patch_path.display()))
            .arg("--mode")
            .arg("no-reboot");
        if dry_run {
            cmd.arg("--dry-run");
        }

        let output = cmd.output().await.map_err(|e| {
            AppError::Internal(format!(
                "Failed to run talosctl (required for config patch apply): {}",
                e
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(AppError::Grpc(format!(
                "talosctl patch mc failed on {}: {}{}",
                self.endpoint,
                stderr.trim(),
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!(" ({})", stdout.trim())
                }
            )));
        }

        info!(
            endpoint = %self.endpoint,
            dry_run,
            "Config patch applied via talosctl patch mc"
        );
        Ok(())
    }

    fn node_host(&self) -> Result<String, AppError> {
        let s = self
            .endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let host = s
            .split(['/', '?'])
            .next()
            .unwrap_or(s)
            .split(':')
            .next()
            .unwrap_or(s)
            .to_string();
        if host.is_empty() {
            return Err(AppError::InvalidInput(format!(
                "Cannot derive node host from endpoint {}",
                self.endpoint
            )));
        }
        Ok(host)
    }

    /// Write a temporary talosconfig + optional patch file for talosctl.
    /// Returns (talosconfig_path, patch_path, tempdir_guard).
    fn write_talosctl_workspace(
        &self,
        patch_yaml: &str,
    ) -> Result<(PathBuf, PathBuf, tempfile::TempDir), AppError> {
        use base64::Engine;
        use std::io::Write;

        let dir = tempfile::tempdir().map_err(|e| {
            AppError::Io(std::io::Error::new(
                e.kind(),
                format!("create temp dir for talosctl: {}", e),
            ))
        })?;

        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let node = self.node_host()?;
        // Minimal single-context talosconfig for this node.
        let talosconfig = format!(
            "context: tcs\ncontexts:\n  tcs:\n    endpoints:\n      - {node}\n    nodes:\n      - {node}\n    ca: {ca}\n    crt: {crt}\n    key: {key}\n",
            node = node,
            ca = b64(&self.ca),
            crt = b64(&self.crt),
            key = b64(&self.key),
        );

        let talosconfig_path = dir.path().join("talosconfig");
        let mut f = std::fs::File::create(&talosconfig_path).map_err(AppError::Io)?;
        f.write_all(talosconfig.as_bytes()).map_err(AppError::Io)?;

        let patch_path = dir.path().join("patch.yaml");
        let mut p = std::fs::File::create(&patch_path).map_err(AppError::Io)?;
        p.write_all(patch_yaml.as_bytes()).map_err(AppError::Io)?;

        Ok((talosconfig_path, patch_path, dir))
    }

    async fn get_machine_config_via_talosctl(&self) -> Result<String, AppError> {
        let node = self.node_host()?;
        let (talosconfig_path, _patch_path, _tmpdir) = self.write_talosctl_workspace("")?;

        let output = tokio::process::Command::new("talosctl")
            .arg("--talosconfig")
            .arg(&talosconfig_path)
            .arg("-n")
            .arg(&node)
            .arg("get")
            .arg("mc")
            .arg("-o")
            .arg("jsonpath={.spec}")
            .output()
            .await
            .map_err(|e| {
                AppError::Internal(format!("Failed to run talosctl get mc: {}", e))
            })?;

        if !output.status.success() {
            return Err(AppError::Grpc(format!(
                "talosctl get mc failed on {}: {}",
                self.endpoint,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        String::from_utf8(output.stdout).map_err(|e| {
            AppError::Internal(format!("talosctl get mc returned non-UTF8: {}", e))
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Convert a JSON-path style patch (`/machine/sysctls/foo`) + value into nested YAML.
pub fn path_value_to_yaml_patch(path: &str, value: &str) -> Result<String, AppError> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        // Treat as a full document body
        return Ok(value.to_string());
    }

    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(AppError::InvalidInput("Empty config patch path".to_string()));
    }

    let leaf: serde_yaml::Value = serde_yaml::from_str(value)
        .or_else(|_| serde_json::from_str(value).map_err(|e| e.to_string()))
        .unwrap_or_else(|_| serde_yaml::Value::String(value.to_string()));

    let mut current = leaf;
    for seg in segments.iter().rev() {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String((*seg).to_string()),
            current,
        );
        current = serde_yaml::Value::Mapping(map);
    }

    serde_yaml::to_string(&current).map_err(|e| {
        AppError::Internal(format!("Failed to serialize config patch YAML: {}", e))
    })
}

/// Merge multiple path/value patches into a multi-document YAML string (preview/debug).
pub fn build_patch_documents(
    patches: &[(String, String, i32)],
) -> Result<String, AppError> {
    let mut sorted = patches.to_vec();
    sorted.sort_by_key(|(_, _, prio)| *prio);

    let mut docs = Vec::new();
    for (path, value, _) in &sorted {
        docs.push(path_value_to_yaml_patch(path, value)?);
    }

    Ok(docs.join("---\n"))
}

/// Deep-merge YAML mappings (patch wins on scalars/sequences).
/// Used to turn path/value patches into a full machine config for ApplyConfiguration.
pub fn deep_merge_yaml(base: &mut serde_yaml::Value, patch: serde_yaml::Value) {
    match (base, patch) {
        (serde_yaml::Value::Mapping(base_map), serde_yaml::Value::Mapping(patch_map)) => {
            for (k, v) in patch_map {
                if let Some(existing) = base_map.get_mut(&k) {
                    deep_merge_yaml(existing, v);
                } else {
                    base_map.insert(k, v);
                }
            }
        }
        (base_slot, patch_val) => {
            *base_slot = patch_val;
        }
    }
}

/// Apply ordered path/value patches onto a full machine-config YAML document.
///
/// Talos `ApplyConfiguration` validates a **full** config. Strategic-merge style
/// partial docs (as used by `talosctl patch mc`) must be merged client-side first.
pub fn merge_patches_into_machine_config(
    current_config_yaml: &str,
    patches: &[(String, String, i32)],
) -> Result<String, AppError> {
    let mut base: serde_yaml::Value = serde_yaml::from_str(current_config_yaml).map_err(|e| {
        AppError::Internal(format!("Failed to parse node machine config YAML: {}", e))
    })?;

    let mut sorted = patches.to_vec();
    sorted.sort_by_key(|(_, _, prio)| *prio);

    for (path, value, _) in &sorted {
        let patch_yaml = path_value_to_yaml_patch(path, value)?;
        let patch_val: serde_yaml::Value = serde_yaml::from_str(&patch_yaml).map_err(|e| {
            AppError::Internal(format!("Failed to parse patch YAML for {}: {}", path, e))
        })?;
        deep_merge_yaml(&mut base, patch_val);
    }

    serde_yaml::to_string(&base).map_err(|e| {
        AppError::Internal(format!("Failed to serialize merged machine config: {}", e))
    })
}

/// Resolve backup directory next to the SQLite database (or `/var/lib/tcs/backups`).
pub fn backup_root_from_sqlite_path(sqlite_path: &str) -> PathBuf {
    Path::new(sqlite_path)
        .parent()
        .map(|p| p.join("backups"))
        .unwrap_or_else(|| PathBuf::from("/var/lib/tcs/backups"))
}

/// Pick the best control-plane machine for etcd snapshot.
pub fn pick_control_plane_address(
    machines: &[(String, Option<String>)], // (machine_type, address)
    creds: &TalosCredentials,
) -> Result<String, AppError> {
    for (mtype, addr) in machines {
        if mtype == "control-plane" || mtype == "controlplane" {
            if let Some(a) = addr {
                if !a.is_empty() {
                    return Ok(a.clone());
                }
            }
        }
    }
    if let Some(ep) = creds.endpoints.first() {
        warn!("No control-plane machine address; using talosconfig endpoint {}", ep);
        return Ok(ep.clone());
    }
    Err(AppError::InvalidInput(
        "No control-plane address available for etcd snapshot".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_endpoint_adds_port() {
        assert_eq!(normalize_endpoint("10.0.0.5"), "https://10.0.0.5:50000");
        assert_eq!(
            normalize_endpoint("10.0.0.5:50000"),
            "https://10.0.0.5:50000"
        );
        assert_eq!(
            normalize_endpoint("https://10.0.0.5:50000"),
            "https://10.0.0.5:50000"
        );
    }

    #[test]
    fn path_value_builds_nested_yaml() {
        let yaml = path_value_to_yaml_patch(
            "/machine/sysctls/net.ipv4.ip_forward",
            "\"1\"",
        )
        .unwrap();
        assert!(yaml.contains("machine:"));
        assert!(yaml.contains("sysctls:"));
        assert!(yaml.contains("net.ipv4.ip_forward"));
    }

    #[test]
    fn merge_patches_preserves_existing_and_adds_leaf() {
        let current = r#"
version: v1alpha1
machine:
  type: controlplane
  network:
    nameservers:
      - 8.8.8.8
cluster:
  clusterName: demo
"#;
        let patches = vec![(
            "/machine/network/extraHostEntries".to_string(),
            r#"[{"ip":"127.0.0.1","aliases":["tcs-smoke.local"]}]"#.to_string(),
            10,
        )];
        let merged = merge_patches_into_machine_config(current, &patches).unwrap();
        assert!(merged.contains("tcs-smoke.local"));
        assert!(merged.contains("8.8.8.8"));
        assert!(merged.contains("clusterName: demo") || merged.contains("clusterName:demo"));
    }
}
