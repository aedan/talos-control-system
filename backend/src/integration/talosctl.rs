//! Talos client via `talosctl` subprocess.
//!
//! All operations shell out to the official `talosctl` binary. Maintenance-mode
//! operations use `-i/--insecure`; post-install operations use `--talosconfig`
//! for mTLS authentication.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

use crate::AppError;

// ─── TalosCredentials ────────────────────────────────────────────────────────

/// Credentials + endpoints from a talosconfig document.
#[derive(Debug, Clone)]
pub struct TalosCredentials {
    pub endpoints: Vec<String>,
    pub nodes: Vec<String>,
}

impl TalosCredentials {
    /// Parse a talosconfig YAML string (same format as `~/.talos/config`).
    pub fn from_talosconfig_yaml(yaml: &str) -> Result<Self, AppError> {
        #[derive(serde::Deserialize)]
        struct TalosConfig {
            context: String,
            contexts: std::collections::HashMap<String, Context>,
        }
        #[derive(serde::Deserialize)]
        struct Context {
            endpoints: Vec<String>,
            #[serde(default)]
            nodes: Vec<String>,
        }

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

        if context.endpoints.is_empty() {
            return Err(AppError::InvalidInput(
                "talosconfig has no endpoints".to_string(),
            ));
        }

        Ok(Self {
            endpoints: context.endpoints.clone(),
            nodes: context.nodes.clone(),
        })
    }
}

// ─── TalosctlClient ──────────────────────────────────────────────────────────

pub struct TalosctlClient;

impl TalosctlClient {
    // ── Helpers ──────────────────────────────────────────────────────────────

    async fn ensure_installed() -> Result<(), AppError> {
        match Command::new("talosctl")
            .arg("--help")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Ok(s) if s.success() => Ok(()),
            _ => Err(AppError::Network(
                "talosctl not found on PATH; install talosctl (required for Talos operations)"
                    .to_string(),
            )),
        }
    }

    /// Build common args: `--talosconfig <path>` if provided.
    ///
    /// The talosconfig is written to a per-call unique temp file (systemd
    /// ProtectSystem makes /tmp read-only). A unique name avoids clobbering
    /// when multiple talosctl commands run concurrently.
    fn talosconfig_args(talosconfig: Option<&str>) -> Vec<String> {
        match talosconfig {
            Some(tc) => {
                let tmpdir = PathBuf::from("/var/lib/tcs/talosctl-tmp");
                if let Err(e) = std::fs::create_dir_all(&tmpdir) {
                    warn!(error = %e, "Failed to create talosctl temp dir");
                }
                let name = format!(
                    "talosconfig.{}.{}",
                    std::process::id(),
                    TalosctlClient::tmp_counter().fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let tmpfile = tmpdir.join(name);
                match std::fs::write(&tmpfile, tc) {
                    Ok(_) => vec![
                        "--talosconfig".to_string(),
                        tmpfile.to_string_lossy().to_string(),
                    ],
                    Err(e) => {
                        warn!(error = %e, "Failed to write talosconfig temp file, falling back to inline");
                        vec!["--talosconfig".to_string(), tc.to_string()]
                    }
                }
            }
            None => vec![],
        }
    }

    fn tmp_counter() -> &'static std::sync::atomic::AtomicU64 {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        &COUNTER
    }

    /// Run a talosctl command and return stdout as a string.
    async fn run(args: &[String]) -> Result<String, AppError> {
        // Remember the per-call talosconfig temp file so we can remove it after.
        let talosconfig_tmp = args
            .iter()
            .position(|a| a == "--talosconfig")
            .and_then(|i| args.get(i + 1).cloned())
            .filter(|p| p.starts_with("/var/lib/tcs/talosctl-tmp/"));

        let out = Command::new("talosctl")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("talosctl spawn: {e}")))?;

        if let Some(p) = &talosconfig_tmp {
            let _ = std::fs::remove_file(p);
        }

        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();

        if !out.status.success() {
            return Err(AppError::Network(format!(
                "talosctl {} failed: {} {}",
                args.first().map(|s| s.as_str()).unwrap_or("unknown"),
                stdout.trim(),
                stderr.trim()
            )));
        }

        Ok(stdout)
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Get the Talos version string from a node.
    ///
    /// `talosctl version` does not support `-o json`; it prints text. We use
    /// `--short` and parse the `Tag:` line (e.g. `Tag: v1.13.7`).
    pub async fn get_version(endpoint: &str, talosconfig: Option<&str>) -> Result<String, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "version".into(), "--short".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            Self::run(&args),
        )
        .await
        .map_err(|_| AppError::Network("talosctl version timed out".to_string()))?
        .map_err(|e| AppError::Network(format!("talosctl version failed: {e}")))?;

        for line in out.lines() {
            let line = line.trim();
            if let Some(tag) = line.strip_prefix("Tag:") {
                let tag = tag.trim();
                if !tag.is_empty() {
                    return Ok(tag.to_string());
                }
            }
        }

        Err(AppError::Network(
            "talosctl version returned no Tag field".to_string(),
        ))
    }

    /// Discover available disks on a machine (maintenance mode, PXE installer).
    pub async fn list_disks(endpoint: &str) -> Result<Vec<serde_json::Value>, AppError> {
        Self::ensure_installed().await?;

        let out = Command::new("talosctl")
            .args(["get", "disks", "-i", "-e", endpoint, "-n", endpoint, "-o", "json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("talosctl spawn: {e}")))?;

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        if !out.status.success() {
            return Err(AppError::Network(format!(
                "talosctl get disks failed: {} {}",
                stdout.trim(),
                stderr.trim()
            )));
        }

        let mut disks = Vec::new();
        let mut current = String::new();
        for line in stdout.lines() {
            if line.trim_start().starts_with('{') && !current.is_empty() {
                let obj: serde_json::Value = serde_json::from_str(&current)
                    .map_err(|e| AppError::Network(format!("Failed to parse disk JSON: {e}")))?;
                disks.push(extract_disk(&obj));
                current.clear();
            }
            current.push_str(line);
            current.push('\n');
        }
        if !current.trim().is_empty() {
            let obj: serde_json::Value = serde_json::from_str(&current)
                .map_err(|e| AppError::Network(format!("Failed to parse disk JSON: {e}")))?;
            disks.push(extract_disk(&obj));
        }

        info!(endpoint, disk_count = disks.len(), "talosctl list_disks");
        Ok(disks)
    }

    /// Discover available disks on a post-install machine (uses talosconfig for auth).
    pub async fn list_disks_postinstall(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "disks".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;

        let mut disks = Vec::new();
        let mut current = String::new();
        for line in out.lines() {
            if line.trim_start().starts_with('{') && !current.is_empty() {
                let obj: serde_json::Value = serde_json::from_str(&current)
                    .map_err(|e| AppError::Network(format!("Failed to parse disk JSON: {e}")))?;
                disks.push(extract_disk(&obj));
                current.clear();
            }
            current.push_str(line);
            current.push('\n');
        }
        if !current.trim().is_empty() {
            let obj: serde_json::Value = serde_json::from_str(&current)
                .map_err(|e| AppError::Network(format!("Failed to parse disk JSON: {e}")))?;
            disks.push(extract_disk(&obj));
        }

        info!(endpoint, disk_count = disks.len(), "talosctl list_disks_postinstall");
        Ok(disks)
    }

    /// Apply a machine config to a machine.
    pub async fn apply_config(
        endpoint: &str,
        config_yaml: &str,
        reboot: bool,
        talosconfig: Option<&str>,
    ) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let tmpfile = format!("/tmp/tcs-apply-config-{:x}.yaml", std::process::id());
        tokio::fs::write(&tmpfile, config_yaml)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write temp config: {e}")))?;

        let mode = if reboot { "reboot" } else { "no-reboot" };

        let mut args: Vec<String> = vec![
            "apply-config".into(), "-f".into(), tmpfile.clone(),
            "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
            "-m".into(), mode.into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        let _ = tokio::fs::remove_file(&tmpfile).await;

        info!(endpoint, mode, "talosctl apply_config");
        Ok(())
    }

    /// Apply a machine config in maintenance mode (for PXE installers).
    pub async fn apply_config_maintenance(
        endpoint: &str,
        config_yaml: &str,
        reboot: bool,
        _talosconfig: Option<&str>,
    ) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let tmpfile = format!("/tmp/tcs-install-config-{:x}.yaml", std::process::id());
        tokio::fs::write(&tmpfile, config_yaml)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write temp config: {e}")))?;

        let mode = if reboot { "reboot" } else { "no-reboot" };

        // Maintenance mode is unauthenticated — do NOT pass talosconfig.
        let args: Vec<String> = vec![
            "apply-config".into(), "-f".into(), tmpfile.clone(), "-i".into(),
            "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
            "-m".into(), mode.into(),
        ];

        Self::run(&args).await?;
        let _ = tokio::fs::remove_file(&tmpfile).await;

        info!(endpoint, mode, "talosctl apply_config (maintenance)");
        Ok(())
    }

    /// Bootstrap a control-plane node (initial etcd formation).
    pub async fn bootstrap(endpoint: &str, talosconfig: Option<&str>) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec!["bootstrap".into(), "-e".into(), endpoint.into()];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        info!(endpoint, "talosctl bootstrap");
        Ok(())
    }

    /// Bootstrap with etcd recovery from a previously uploaded snapshot.
    pub async fn bootstrap_recover_etcd(
        endpoint: &str,
        skip_hash_check: bool,
        talosconfig: Option<&str>,
    ) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "bootstrap".into(), "-e".into(), endpoint.into(), "--recover-etcd".into(),
        ];
        if skip_hash_check {
            args.push("--recover-skip-hash-check".into());
        }
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        info!(endpoint, "talosctl bootstrap (recover_etcd)");
        Ok(())
    }

    /// Reboot a machine.
    pub async fn reboot(endpoint: &str, talosconfig: Option<&str>) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "reboot".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        info!(endpoint, "talosctl reboot");
        Ok(())
    }

    /// Reset a machine (destructive).
    pub async fn reset(
        endpoint: &str,
        graceful: bool,
        reboot: bool,
        talosconfig: Option<&str>,
    ) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "reset".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
            "--system-labels".into(), "--data-partitions".into(), "--system-partitions".into(),
        ];
        if graceful {
            args.push("--graceful".into());
        }
        if reboot {
            args.push("--reboot".into());
        }
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        info!(endpoint, graceful, reboot, "talosctl reset");
        Ok(())
    }

    /// Upgrade a machine to a new Talos image.
    pub async fn upgrade(
        endpoint: &str,
        image: &str,
        talosconfig: Option<&str>,
    ) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "upgrade".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(),
            "--image".into(), image.into(),
            "--preserve".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;
        info!(endpoint, image, "talosctl upgrade");
        Ok(())
    }

    /// Stream an etcd snapshot to a local file.
    pub async fn etcd_snapshot(
        endpoint: &str,
        dest_path: &str,
        talosconfig: Option<&str>,
    ) -> Result<u64, AppError> {
        Self::ensure_installed().await?;

        if let Some(parent) = Path::new(dest_path).parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::Io(std::io::Error::new(
                    e.kind(),
                    format!("create backup dir {}: {}", parent.display(), e),
                ))
            })?;
        }

        let mut args: Vec<String> = vec![
            "etcd".into(), "snapshot".into(), "create".into(),
            "-e".into(), endpoint.into(),
            "--output".into(), dest_path.into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;

        let metadata = tokio::fs::metadata(dest_path).await.map_err(AppError::Io)?;
        let size = metadata.len();

        if size == 0 {
            let _ = tokio::fs::remove_file(dest_path).await;
            return Err(AppError::Network(format!(
                "Etcd snapshot from {} returned empty file",
                endpoint
            )));
        }

        info!(endpoint, path = dest_path, bytes = size, "talosctl etcd_snapshot");
        Ok(size)
    }

    /// Upload an etcd snapshot to a control-plane node for recovery.
    pub async fn etcd_recover(
        endpoint: &str,
        snapshot_path: &str,
        talosconfig: Option<&str>,
    ) -> Result<u64, AppError> {
        Self::ensure_installed().await?;

        let metadata = tokio::fs::metadata(snapshot_path).await.map_err(|e| {
            AppError::Io(std::io::Error::new(
                e.kind(),
                format!("read snapshot {}: {}", snapshot_path, e),
            ))
        })?;
        let size = metadata.len();
        if size == 0 {
            return Err(AppError::InvalidInput("Snapshot file is empty".to_string()));
        }

        let mut args: Vec<String> = vec![
            "etcd".into(), "snapshot".into(), "recover".into(),
            "-e".into(), endpoint.into(),
            "--from".into(), snapshot_path.into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        Self::run(&args).await?;

        info!(endpoint, path = snapshot_path, bytes = size, "talosctl etcd_recover");
        Ok(size)
    }

    /// Get the running machine config as YAML.
    pub async fn get_machine_config(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<String, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "mc".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "yaml".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        Ok(out)
    }

    /// List machined services on a node.
    pub async fn service_list(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "services".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        let stream: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&out)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                AppError::Network(format!("Failed to parse talosctl services JSON: {e}"))
            })?;

        let services = stream
            .iter()
            .filter(|v| v.is_object())
            .map(|svc| {
                let id = svc["metadata"]["id"].as_str().unwrap_or("").to_string();
                let state = svc["metadata"]["phase"].as_str().unwrap_or("").to_string();
                let healthy = svc["spec"]["healthy"].as_bool().unwrap_or(false);
                let unknown = svc["spec"]["unknown"].as_bool().unwrap_or(false);
                serde_json::json!({
                    "id": id,
                    "state": state,
                    "healthy": healthy,
                    "unknown": unknown,
                })
            })
            .collect();

        Ok(services)
    }

    /// Get hostname of a node.
    pub async fn hostname(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<String, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "mc".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        let parsed: serde_json::Value = serde_json::from_str(&out).map_err(|e| {
            AppError::Network(format!("Failed to parse talosctl mc JSON: {e}"))
        })?;

        // The MachineConfig `spec` is an opaque YAML string, not a JSON object.
        let spec_yaml = parsed["spec"].as_str().ok_or_else(|| {
            AppError::Network("talosctl get mc returned no spec".to_string())
        })?;
        let spec: serde_yaml::Value = serde_yaml::from_str(spec_yaml).map_err(|e| {
            AppError::Network(format!("Failed to parse mc spec YAML: {e}"))
        })?;

        spec["machine"]["network"]["hostname"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::Network("talosctl get mc returned no hostname".to_string()))
    }

    /// Probe a node via talosctl maintenance mode.
    pub async fn probe_maintenance(endpoint: &str) -> Result<String, AppError> {
        Self::ensure_installed().await?;

        let out = Command::new("talosctl")
            .args(["get", "disks", "-i", "-e", endpoint, "-n", endpoint, "-o", "json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("talosctl spawn: {e}")))?;

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        if !out.status.success() {
            return Err(AppError::Network(format!(
                "talosctl probe failed: {} {}",
                stdout.trim(),
                stderr.trim()
            )));
        }

        info!(endpoint, "talosctl probe_maintenance successful");
        Ok("reachable".to_string())
    }

    /// Probe a running Talos node (post-install) using talosconfig auth.
    pub async fn probe_node(endpoint: &str, talosconfig: Option<&str>) -> Result<String, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "version".into(), "-e".into(), endpoint.into(),
            "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Command::new("talosctl")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("talosctl spawn: {e}")))?;

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        if !out.status.success() {
            return Err(AppError::Network(format!(
                "talosctl probe node failed: {} {}",
                stdout.trim(),
                stderr.trim()
            )));
        }

        info!(endpoint, "talosctl probe_node successful");
        Ok("reachable".to_string())
    }
}

fn extract_disk(obj: &serde_json::Value) -> serde_json::Value {
    let name = obj["metadata"]["id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let dev_path = obj["spec"]["dev_path"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let size = obj["spec"]["size"].as_u64().unwrap_or(0);
    let model = obj["spec"]["model"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let transport = obj["spec"]["transport"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let wwid = obj["spec"]["wwid"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let readonly = obj["spec"]["readonly"].as_bool().unwrap_or(false);
    let cdrom = obj["spec"]["cdrom"].as_bool().unwrap_or(false);
    let pretty_size = obj["spec"]["pretty_size"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let sector_size = obj["spec"]["sector_size"].as_u64().unwrap_or(512);

    serde_json::json!({
        "deviceName": dev_path,
        "name": name,
        "serial": wwid,
        "size": size,
        "type": transport,
        "model": model,
        "systemDisk": false,
        "readonly": readonly,
        "cdrom": cdrom,
        "prettySize": pretty_size,
        "sectorSize": sector_size,
    })
}

// ─── Utility functions (moved from talos.rs) ─────────────────────────────────

/// Resolve backup directory next to the SQLite database.
pub fn backup_root_from_sqlite_path(sqlite_path: &str) -> PathBuf {
    Path::new(sqlite_path)
        .parent()
        .map(|p| p.join("backups"))
        .unwrap_or_else(|| PathBuf::from("/var/lib/tcs/backups"))
}

/// Pick the best control-plane machine for etcd snapshot.
pub fn pick_control_plane_address(
    machines: &[(String, Option<String>)],
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

/// Convert a JSON-path style patch (`/machine/sysctls/foo`) + value into nested YAML.
pub fn path_value_to_yaml_patch(path: &str, value: &str) -> Result<String, AppError> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
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

/// Merge multiple path/value patches into a multi-document YAML string.
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
    use serde::Deserialize;
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
        let trimmed = s.trim_start_matches("---\n").trim_start_matches("---\r\n");
        out.push_str(trimmed);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(merged.matches("---\n").count(), 2);
    }

    #[test]
    fn merge_interfaces_list_is_replaced_not_appended() {
        let current = r#"
version: v1alpha1
machine:
  type: controlplane
  network:
    interfaces:
      - interface: eno1
        mtu: 1500
        addresses:
          - 192.168.1.200/24
        routes:
          - network: 0.0.0.0/0
            gateway: 192.168.1.2
      - interface: eno2
        ignore: true
cluster:
  clusterName: demo
"#;
        // Operator pastes ONLY the bond into the Network YAML helper.
        let patch = r#"
machine:
  network:
    interfaces:
      - interface: bond0
        mtu: 9000
        addresses:
          - 192.168.1.200/24
        routes:
          - network: 0.0.0.0/0
            gateway: 192.168.1.2
        bonds:
          bond0:
            interfaces:
              - eno1
              - eno2
            mode: 802.3ad
"#;
        let merged = merge_yaml_docs_into_machine_config(current, patch).unwrap();
        // The bond is present...
        assert!(merged.contains("bond0"));
        // ...but the pre-existing eno1/eno2 entries are GONE (list replaced).
        assert!(!merged.contains("interface: eno1"));
        assert!(!merged.contains("interface: eno2"));
        // Other keys survive the deep merge.
        assert!(merged.contains("clusterName: demo") || merged.contains("clusterName:demo"));
    }
}
