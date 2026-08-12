//! Real Talos Linux machine API client over gRPC + mTLS.
//!
//! Uses [`talos-rust-client`] against the standard machine API (port 50000).

use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::Deserialize;
use talos_rust_client::machine::apply_configuration_request::Mode as ApplyMode;
use talos_rust_client::machine::reboot_request::Mode as RebootMode;
use talos_rust_client::machine::{
    ApplyConfigurationRequest, EtcdSnapshotRequest, RebootRequest, UpgradeRequest,
};
use talos_rust_client::talosconfig::TalosConfig;
use talos_rust_client::{MachineServiceClient, StorageServiceClient, TalosConnector};
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

    /// Create a client from raw PEM data (bypasses talosconfig parsing).
    /// Used during greenfield provisioning with our generated PKI.
    pub fn from_pem(node_address: &str, ca_pem: &str, crt_pem: &str, key_pem: &str) -> Self {
        Self::new(
            node_address.to_string(),
            ca_pem.as_bytes().to_vec(),
            crt_pem.as_bytes().to_vec(),
            key_pem.as_bytes().to_vec(),
        )
    }

    async fn connect(&self) -> Result<MachineServiceClient<talos_rust_client::Channel>, AppError> {
        // rustls Identity::from_pem rejects OpenSSL "BEGIN ED25519 PRIVATE KEY"
        // (common in talosconfig). Convert to PKCS#8 labels first.
        let key = ensure_pkcs8_pem(&self.key)?;

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
        self.apply_config_with_options(config, false, false).await
    }

    pub async fn apply_config_with_options(
        &self,
        config: &str,
        dry_run: bool,
        reboot: bool,
    ) -> Result<(), AppError> {
        let mut client = self.connect().await?;
        let request = ApplyConfigurationRequest {
            data: config.as_bytes().to_vec(),
            mode: if reboot { ApplyMode::Reboot as i32 } else { ApplyMode::NoReboot as i32 },
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

    /// Discover available disks on the machine via StorageService.
    pub async fn list_disks(&self) -> Result<Vec<talos_rust_client::storage::Disk>, AppError> {
        let channel = self.connect_channel().await?;
        let mut client = StorageServiceClient::new(channel);
        let request = tonic::Request::new(
            talos_rust_client::generated::google::protobuf::Empty {}
        );
        let response = client.disks(request).await.map_err(|e| {
            AppError::Grpc(format!("Disks RPC failed on {}: {}", self.endpoint, e))
        })?;
        let inner = response.into_inner();
        let mut result = Vec::new();
        for msg in &inner.messages {
            result.extend(msg.disks.clone());
        }
        Ok(result)
    }

    /// Wipe/reset a machine (destructive). Prefer graceful=true for etcd leave.
    pub async fn reset(&self, graceful: bool, reboot: bool) -> Result<(), AppError> {
        use talos_rust_client::machine::ResetRequest;
        use talos_rust_client::machine::reset_request::WipeMode;
        let mut client = self.connect().await?;
        let request = ResetRequest {
            graceful,
            reboot,
            system_partitions_to_wipe: vec![],
            user_disks_to_wipe: vec![],
            mode: WipeMode::All as i32,
        };
        client.reset(request).await.map_err(|e| {
            AppError::Grpc(format!("Reset failed on {}: {}", self.endpoint, e))
        })?;
        info!(endpoint = %self.endpoint, graceful, reboot, "Machine reset initiated");
        Ok(())
    }

    /// Bootstrap a control-plane node (initial etcd formation).
    pub async fn bootstrap(&self) -> Result<(), AppError> {
        use talos_rust_client::machine::BootstrapRequest;
        let mut client = self.connect().await?;
        let request = BootstrapRequest {
            recover_etcd: false,
            recover_skip_hash_check: false,
        };
        client.bootstrap(request).await.map_err(|e| {
            AppError::Grpc(format!("Bootstrap failed on {}: {}", self.endpoint, e))
        })?;
        info!(endpoint = %self.endpoint, "Bootstrap initiated");
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

    /// Read the running machine config YAML (multi-document supported).
    ///
    /// Prefer COSI `MachineConfigs.config.talos.dev` (Talos 1.6+). Fall back to the
    /// legacy `/system/state/config.yaml` file path on older images.
    pub async fn get_machine_config(&self) -> Result<String, AppError> {
        match self.get_machine_config_via_cosi().await {
            Ok(s) if !s.trim().is_empty() => return Ok(s),
            Ok(_) => {}
            Err(e) => {
                warn!(endpoint = %self.endpoint, error = %e, "COSI MachineConfig get failed");
            }
        }

        match self.get_machine_config_via_file().await {
            Ok(s) if !s.trim().is_empty() => Ok(s),
            Ok(_) => Err(AppError::NotFound(format!(
                "Empty machine config on {}",
                self.endpoint
            ))),
            Err(e) => Err(e),
        }
    }

    /// Apply a strategic-merge style patch against the live machine config.
    ///
    /// Pure-Rust: COSI/file Get → multi-doc deep-merge → `ApplyConfiguration`.
    /// No host `talosctl` dependency.
    pub async fn apply_config_patch(&self, patch_yaml: &str, dry_run: bool) -> Result<(), AppError> {
        let current = self.get_machine_config().await?;
        if !current.contains("cluster:") && !current.contains("machine:") {
            return Err(AppError::Internal(format!(
                "Fetched machine config from {} does not look like a Talos config ({} bytes)",
                self.endpoint,
                current.len()
            )));
        }
        let merged = merge_yaml_docs_into_machine_config(&current, patch_yaml)?;
        if !merged.contains("cluster:") {
            return Err(AppError::Internal(format!(
                "Merged config for {} lost cluster section (current={} bytes, patch={} bytes, merged={} bytes)",
                self.endpoint,
                current.len(),
                patch_yaml.len(),
                merged.len()
            )));
        }
        self.apply_config_with_options(&merged, dry_run, false).await?;
        info!(
            endpoint = %self.endpoint,
            dry_run,
            current_bytes = current.len(),
            merged_bytes = merged.len(),
            "Config patch applied via pure-Rust merge+ApplyConfiguration"
        );
        Ok(())
    }

    /// COSI State.Get for MachineConfigs.config.talos.dev.
    ///
    /// Spec encoding (Talos): `proto_spec` is `resource.config.MachineConfigSpec`
    /// with field 1 = raw machine-config YAML bytes. `yaml_spec` may be a
    /// YAML-quoted string of the same content.
    async fn get_machine_config_via_cosi(&self) -> Result<String, AppError> {
        use prost::Message;
        use tonic::Request;

        let channel = self.connect_channel().await?;
        let mut client = CosiStateClient::new(channel);

        // Active applied config is `v1alpha1`; on-disk is `persistent`.
        for id in ["v1alpha1", "persistent", "active"] {
            let req = CosiGetRequest {
                namespace: "config".to_string(),
                r#type: "MachineConfigs.config.talos.dev".to_string(),
                id: id.to_string(),
                options: None,
            };
            match client.get(Request::new(req)).await {
                Ok(resp) => {
                    let inner = resp.into_inner();
                    if let Some(resource) = inner.resource {
                        if let Some(spec) = resource.spec {
                            info!(
                                endpoint = %self.endpoint,
                                id,
                                yaml_len = spec.yaml_spec.len(),
                                proto_len = spec.proto_spec.len(),
                                "COSI MachineConfig Get response"
                            );
                            if let Some(yaml) = decode_machine_config_spec(&spec) {
                                return Ok(yaml);
                            }
                        } else {
                            warn!(endpoint = %self.endpoint, id, "COSI MachineConfig has no spec");
                        }
                    } else {
                        warn!(endpoint = %self.endpoint, id, "COSI Get returned empty resource");
                    }
                }
                Err(status) if status.code() == tonic::Code::NotFound => {
                    warn!(endpoint = %self.endpoint, id, "COSI MachineConfig not found");
                    continue;
                }
                Err(e) => {
                    return Err(AppError::Grpc(format!(
                        "COSI Get MachineConfig failed on {}: {}",
                        self.endpoint, e
                    )));
                }
            }
        }

        Err(AppError::NotFound(format!(
            "No MachineConfig resource on {}",
            self.endpoint
        )))
    }

    async fn get_machine_config_via_file(&self) -> Result<String, AppError> {
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

    async fn connect_channel(&self) -> Result<tonic::transport::Channel, AppError> {
        let key = ensure_pkcs8_pem(&self.key)?;
        TalosConnector::new(&self.endpoint)
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
            })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

// ─── Minimal COSI State.Get client (MachineConfig only) ─────────────────

#[derive(Clone, PartialEq, prost::Message)]
struct CosiGetRequest {
    #[prost(string, tag = "1")]
    namespace: String,
    #[prost(string, tag = "2")]
    r#type: String,
    #[prost(string, tag = "3")]
    id: String,
    #[prost(message, optional, tag = "4")]
    options: Option<CosiGetOptions>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CosiGetOptions {}

#[derive(Clone, PartialEq, prost::Message)]
struct CosiGetResponse {
    #[prost(message, optional, tag = "1")]
    resource: Option<CosiResource>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CosiResource {
    #[prost(message, optional, tag = "1")]
    metadata: Option<CosiMetadata>,
    #[prost(message, optional, tag = "2")]
    spec: Option<CosiSpec>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CosiMetadata {
    #[prost(string, tag = "1")]
    namespace: String,
    #[prost(string, tag = "2")]
    r#type: String,
    #[prost(string, tag = "3")]
    id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct CosiSpec {
    #[prost(bytes = "vec", tag = "1")]
    proto_spec: Vec<u8>,
    #[prost(string, tag = "2")]
    yaml_spec: String,
}

/// Talos `resource.config.MachineConfigSpec` (api/resource/config/config.proto).
#[derive(Clone, PartialEq, prost::Message)]
struct TalosMachineConfigSpec {
    #[prost(bytes = "vec", tag = "1")]
    yaml_marshalled: Vec<u8>,
}

fn decode_machine_config_spec(spec: &CosiSpec) -> Option<String> {
    use prost::Message;

    // Preferred: protobuf wrapper with raw YAML bytes.
    if !spec.proto_spec.is_empty() {
        if let Ok(mcs) = TalosMachineConfigSpec::decode(spec.proto_spec.as_slice()) {
            if !mcs.yaml_marshalled.is_empty() {
                if let Ok(s) = String::from_utf8(mcs.yaml_marshalled) {
                    if s.contains("machine:") || s.contains("cluster:") || s.contains("version:") {
                        return Some(s);
                    }
                }
            }
        }
        // Some builds may put raw YAML in proto_spec.
        if let Ok(s) = String::from_utf8(spec.proto_spec.clone()) {
            if s.contains("machine:") || s.contains("version:") {
                return Some(s);
            }
        }
    }

    if !spec.yaml_spec.trim().is_empty() {
        // MarshalYAML of the resource often yields a YAML string scalar.
        if let Ok(unquoted) = serde_yaml::from_str::<String>(&spec.yaml_spec) {
            if unquoted.contains("machine:") || unquoted.contains("version:") {
                return Some(unquoted);
            }
        }
        if spec.yaml_spec.contains("machine:") || spec.yaml_spec.contains("version:") {
            return Some(spec.yaml_spec.clone());
        }
    }

    None
}

#[derive(Debug, Clone)]
struct CosiStateClient {
    inner: tonic::client::Grpc<tonic::transport::Channel>,
}

impl CosiStateClient {
    fn new(channel: tonic::transport::Channel) -> Self {
        Self {
            inner: tonic::client::Grpc::new(channel),
        }
    }

    async fn get(
        &mut self,
        request: tonic::Request<CosiGetRequest>,
    ) -> Result<tonic::Response<CosiGetResponse>, tonic::Status> {
        self.inner
            .ready()
            .await
            .map_err(|e| tonic::Status::unknown(format!("COSI service not ready: {e}")))?;
        let codec = tonic::codec::ProstCodec::default();
        let path = http::uri::PathAndQuery::from_static("/cosi.resource.State/Get");
        self.inner.unary(request, path, codec).await
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

/// Apply ordered path/value patches onto a (possibly multi-document) machine config.
///
/// Talos `ApplyConfiguration` validates a **full** config. Strategic-merge style
/// partial docs (as used by `talosctl patch mc`) must be merged client-side first.
/// Only the primary v1alpha1 machine document is mutated; other docs (LinkConfig, …)
/// are preserved in order.
pub fn merge_patches_into_machine_config(
    current_config_yaml: &str,
    patches: &[(String, String, i32)],
) -> Result<String, AppError> {
    let patch_yaml = build_patch_documents(patches)?;
    merge_yaml_docs_into_machine_config(current_config_yaml, &patch_yaml)
}

/// Merge one or more strategic-merge YAML documents into a multi-doc machine config.
pub fn merge_yaml_docs_into_machine_config(
    current_config_yaml: &str,
    patch_yaml: &str,
) -> Result<String, AppError> {
    let mut docs = parse_yaml_documents(current_config_yaml)?;
    if docs.is_empty() {
        return Err(AppError::Internal(
            "Node machine config is empty".to_string(),
        ));
    }

    let target_idx = docs
        .iter()
        .position(is_primary_machine_config_doc)
        .unwrap_or(0);

    let patch_docs = parse_yaml_documents(patch_yaml)?;
    for patch in patch_docs {
        deep_merge_yaml(&mut docs[target_idx], patch);
    }

    serialize_yaml_documents(&docs)
}

fn parse_yaml_documents(yaml: &str) -> Result<Vec<serde_yaml::Value>, AppError> {
    let mut docs = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let v = serde_yaml::Value::deserialize(doc).map_err(|e| {
            AppError::Internal(format!("Failed to parse YAML document: {}", e))
        })?;
        if !v.is_null() {
            docs.push(v);
        }
    }
    if docs.is_empty() {
        // Single-document fallback for parsers that skip empty streams.
        if let Ok(v) = serde_yaml::from_str::<serde_yaml::Value>(yaml) {
            if !v.is_null() {
                docs.push(v);
            }
        }
    }
    Ok(docs)
}

fn is_primary_machine_config_doc(doc: &serde_yaml::Value) -> bool {
    let Some(map) = doc.as_mapping() else {
        return false;
    };
    // Multi-doc "document configs" use kind/apiVersion; the classic machine config
    // has top-level `machine` / `cluster` / `version: v1alpha1`.
    if map.contains_key(serde_yaml::Value::String("kind".into())) {
        return false;
    }
    map.contains_key(serde_yaml::Value::String("machine".into()))
        || map.contains_key(serde_yaml::Value::String("cluster".into()))
}

fn serialize_yaml_documents(docs: &[serde_yaml::Value]) -> Result<String, AppError> {
    let mut out = String::new();
    for (i, doc) in docs.iter().enumerate() {
        if i > 0 {
            out.push_str("---\n");
        }
        let s = serde_yaml::to_string(doc).map_err(|e| {
            AppError::Internal(format!("Failed to serialize machine config YAML: {}", e))
        })?;
        // serde_yaml may prefix with "---\n"; normalize.
        let trimmed = s.trim_start_matches("---\n").trim_start_matches("---\r\n");
        out.push_str(trimmed);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
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

    #[test]
    fn merge_preserves_secondary_docs() {
        let current = r#"
version: v1alpha1
machine:
  type: controlplane
  ca:
    crt: BASE64CA
  network:
    nameservers:
      - 8.8.8.8
cluster:
  clusterName: demo
  id: abc
---
apiVersion: v1alpha1
kind: LinkConfig
name: eno49
up: true
mtu: 9000
---
apiVersion: v1alpha1
kind: LinkConfig
name: eno50
up: true
"#;
        let patch = r#"
machine:
  network:
    extraHostEntries:
      - ip: 127.0.0.1
        aliases:
          - tcs-smoke.local
"#;
        let merged = merge_yaml_docs_into_machine_config(current, patch).unwrap();
        assert!(merged.contains("tcs-smoke.local"));
        assert!(merged.contains("BASE64CA"));
        assert!(merged.contains("clusterName: demo") || merged.contains("clusterName:demo"));
        assert!(merged.contains("LinkConfig"));
        assert!(merged.contains("eno49"));
        assert!(merged.contains("eno50"));
        assert!(merged.contains("---"));
        // Exactly two document separators for 3 docs
        assert_eq!(merged.matches("---\n").count(), 2);
    }

    #[test]
    fn decode_machine_config_spec_from_proto() {
        use prost::Message;
        let yaml = "version: v1alpha1\nmachine:\n  type: worker\ncluster:\n  clusterName: x\n";
        let mcs = TalosMachineConfigSpec {
            yaml_marshalled: yaml.as_bytes().to_vec(),
        };
        let mut buf = Vec::new();
        mcs.encode(&mut buf).unwrap();
        let spec = CosiSpec {
            proto_spec: buf,
            yaml_spec: String::new(),
        };
        let out = decode_machine_config_spec(&spec).expect("decode");
        assert!(out.contains("worker"));
        assert!(out.contains("clusterName"));
    }
}
