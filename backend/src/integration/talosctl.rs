//! Talos client via `talosctl` subprocess.
//!
//! All operations shell out to the official `talosctl` binary. Maintenance-mode
//! operations use `-i/--insecure`; post-install operations use `--talosconfig`
//! for mTLS authentication.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
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

/// A single installed Talos extension (module) on a node.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct MachineExtension {
    pub id: String,
    pub source: String,
    pub hash: String,
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
            .env("TCS_INTERNAL", "1")
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
            .env("TCS_INTERNAL", "1")
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

    /// Run a talosctl command, returning `(success, stdout, stderr)` instead of
    /// mapping non-zero exit to `Err`. Needed for commands whose *output* is the
    /// data we inspect (e.g. `upgrade-k8s --dry-run` printing its plan while a
    /// node is briefly unreachable, or `--to` validation errors).
    async fn run_capture(args: &[String]) -> Result<(bool, String, String), AppError> {
        let talosconfig_tmp = args
            .iter()
            .position(|a| a == "--talosconfig")
            .and_then(|i| args.get(i + 1).cloned())
            .filter(|p| p.starts_with("/var/lib/tcs/talosctl-tmp/"));

        let out = Command::new("talosctl")
            .env("TCS_INTERNAL", "1")
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
        Ok((out.status.success(), stdout, stderr))
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
            .env("TCS_INTERNAL", "1")
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
        dry_run: bool,
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
        if dry_run {
            args.push("--dry-run".into());
        }
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

    /// Upgrade the cluster's Kubernetes control plane in place (no node reboots).
    ///
    /// Wraps `talosctl upgrade-k8s`, which talks to a single control-plane node,
    /// discovers the rest of the cluster itself, pre-pulls the new images, and
    /// patches every node's machineconfig. `from`/`to` are exact versions
    /// (e.g. "v1.36.3"). `dry_run` only prints the plan — used to probe which
    /// target versions this Talos build supports.
    ///
    /// NOTE: this can run for minutes (image pre-pull + reconcile), so it is
    /// invoked by callers via `run_capture_k8s_upgrade` with a generous timeout,
    /// never through `Self::run`'s default behavior.
    pub fn k8s_upgrade_args(
        endpoint: &str,
        from: &str,
        to: &str,
        dry_run: bool,
        talosconfig: Option<&str>,
    ) -> Vec<String> {
        let mut args: Vec<String> = vec![
            "upgrade-k8s".into(),
            "-e".into(),
            endpoint.into(),
            "--from".into(),
            from.into(),
            "--to".into(),
            to.into(),
        ];
        if dry_run {
            args.push("--dry-run".into());
        }
        args.extend(Self::talosconfig_args(talosconfig));
        args
    }

    /// Dry-run probe: true when `upgrade-k8s` accepts the from→to path on this
    /// build (i.e. the target is supported), false on "unsupported upgrade path"
    /// or transport errors (a node flapping during the probe is not a verdict).
    pub async fn k8s_upgrade_supported(
        endpoint: &str,
        from: &str,
        to: &str,
        talosconfig: Option<&str>,
    ) -> Result<bool, AppError> {
        Self::ensure_installed().await?;
        let args = Self::k8s_upgrade_args(endpoint, from, to, true, talosconfig);
        let (_ok, stdout, stderr) = Self::run_capture(&args).await?;
        let combined = format!("{stdout}\n{stderr}");
        Ok(!combined.contains("unsupported upgrade path"))
    }

    /// Perform the real in-place k8s upgrade. Long-running; callers wrap in a timeout.
    pub async fn k8s_upgrade(
        endpoint: &str,
        from: &str,
        to: &str,
        talosconfig: Option<&str>,
    ) -> Result<String, AppError> {
        Self::ensure_installed().await?;
        let args = Self::k8s_upgrade_args(endpoint, from, to, false, talosconfig);
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(1800),
            Self::run_capture(&args),
        )
        .await
        .map_err(|_| AppError::Network("talosctl upgrade-k8s timed out".to_string()))?;
        let (ok, stdout, stderr) = out?;
        if !ok {
            return Err(AppError::Network(format!(
                "talosctl upgrade-k8s failed: {} {}",
                stdout.trim(),
                stderr.trim()
            )));
        }
        info!(endpoint, from, to, "talosctl upgrade-k8s");
        Ok(stdout)
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
            "get".into(), "mc".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        spec_from_mc_json(&out)
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

    /// Get a single installed extension (Talos module) from the extensions list JSON.
    ///
    /// `talosctl get extensions -o json` emits one document per module. Each doc has
    /// `metadata.id` (a numeric index, NOT the name) and the real module info under
    /// `spec.metadata.name` + `spec.image` + `spec.metadata.version`.
    fn extension_from_item(v: &serde_json::Value) -> Option<MachineExtension> {
        let name = v["spec"]["metadata"]["name"].as_str()?.to_string();
        if name.is_empty() {
            return None;
        }
        Some(MachineExtension {
            id: name,
            source: v["spec"]["image"].as_str().unwrap_or("").to_string(),
            hash: v["spec"]["metadata"]["version"].as_str().unwrap_or("").to_string(),
        })
    }

    /// List installed Talos extensions (modules) on a node.
    pub async fn list_extensions(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<Vec<MachineExtension>, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "extensions".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        let stream: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&out)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                AppError::Network(format!("Failed to parse talosctl extensions JSON: {e}"))
            })?;

        let extensions = stream
            .iter()
            .filter(|v| v.is_object())
            .filter_map(Self::extension_from_item)
            .collect();

        Ok(extensions)
    }

    /// Get the node's Talos version info (from `talosctl get versions -o json`).
    ///
    /// Returns a normalized `{ version, upgradable }` object. Newer Talos releases may
    /// add an `installed` image URI under `spec`; we surface it as `installed` when
    /// present, otherwise `installed` is empty and the UI falls back to `version`.
    pub async fn get_versions(
        endpoint: &str,
        talosconfig: Option<&str>,
    ) -> Result<serde_json::Value, AppError> {
        Self::ensure_installed().await?;

        let mut args: Vec<String> = vec![
            "get".into(), "versions".into(), "-e".into(), endpoint.into(), "-n".into(), endpoint.into(), "-o".into(), "json".into(),
        ];
        args.extend(Self::talosconfig_args(talosconfig));

        let out = Self::run(&args).await?;
        let parsed: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&out)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Network(format!("Failed to parse talosctl versions JSON: {e}")))?;

        let doc = parsed
            .into_iter()
            .find(|v| v.is_object())
            .ok_or_else(|| AppError::Network("talosctl get versions returned no object".to_string()))?;

        let spec = &doc["spec"];
        Ok(serde_json::json!({
            "version": spec.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "installed": spec.get("installed").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "upgradable": spec.get("upgradable").and_then(|v| v.as_str()),
        }))
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
            .env("TCS_INTERNAL", "1")
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
            .env("TCS_INTERNAL", "1")
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
        if is_standalone_config_doc(&patch) {
            upsert_standalone_doc(&mut docs, patch);
        } else {
            deep_merge_yaml(&mut docs[target_idx], patch);
        }
    }

    drop_redundant_standalone_vlans(&mut docs);

    serialize_yaml_documents(&docs)
}

/// Drop standalone `VLANConfig` docs whose VLAN is now expressed as a nested
/// `vlans:` entry on the parent interface in the machine config. Both forms
/// create the same `<parent>.<vlanID>` link, so keeping both makes Talos
/// reject the config with a link conflict.
fn drop_redundant_standalone_vlans(docs: &mut Vec<serde_yaml::Value>) {
    let Some(primary) = docs.iter().find(|d| is_primary_machine_config_doc(d)) else {
        return;
    };
    let mut nested: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    if let Some(ifaces) = primary
        .get("machine")
        .and_then(|m| m.get("network"))
        .and_then(|n| n.get("interfaces"))
        .and_then(|i| i.as_sequence())
    {
        for iface in ifaces {
            let Some(name) = iface.get("interface").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(vlans) = iface.get("vlans").and_then(|v| v.as_sequence()) else {
                continue;
            };
            for vlan in vlans {
                if let Some(id) = vlan.get("vlanId").and_then(|v| v.as_u64()) {
                    nested.insert((name.to_string(), id.to_string()));
                }
            }
        }
    }

    docs.retain(|doc| {
        if !is_standalone_config_doc(doc) {
            return true;
        }
        if doc.get("kind").and_then(|v| v.as_str()) != Some("VLANConfig") {
            return true;
        }
        let Some(parent) = doc.get("parent").and_then(|v| v.as_str()) else {
            return true;
        };
        let Some(id) = doc.get("vlanID").and_then(|v| v.as_u64()) else {
            return true;
        };
        !nested.contains(&(parent.to_string(), id.to_string()))
    });
}

/// A standalone network config document (e.g. `kind: VLANConfig`) is
/// identified by the presence of `apiVersion` + `kind` — it must not be
/// deep-merged into the machine config doc.
fn is_standalone_config_doc(doc: &serde_yaml::Value) -> bool {
    let Some(map) = doc.as_mapping() else {
        return false;
    };
    map.contains_key(serde_yaml::Value::String("kind".into()))
}

/// Replace a standalone doc with the same `kind`+`name`, or append it.
fn upsert_standalone_doc(docs: &mut Vec<serde_yaml::Value>, patch: serde_yaml::Value) {
    let kind = patch.get("kind").map(|v| v.as_str().unwrap_or("").to_string());
    let name = patch.get("name").map(|v| v.as_str().unwrap_or("").to_string());
    if let Some((idx, _)) = docs.iter().enumerate().find(|(_, d)| {
        is_standalone_config_doc(d)
            && d.get("kind").and_then(|v| v.as_str()) == kind.as_deref()
            && d.get("name").and_then(|v| v.as_str()) == name.as_deref()
    }) {
        docs[idx] = patch;
    } else {
        docs.push(patch);
    }
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

/// Extract the machine config from `talosctl get mc -o json` output.
///
/// The MachineConfig resource wraps the real config in `spec` as an opaque
/// YAML string (fields like `node`/`metadata` are resource plumbing that
/// must not be fed back to the node).
fn spec_from_mc_json(out: &str) -> Result<String, AppError> {
    // talosctl get mc -o json emits one JSON document per machine config
    // resource (multi-doc configs produce several). Pick the primary
    // machine config document (metadata.id == "v1alpha1"), falling back to
    // the first document that carries a spec string.
    let stream: Vec<serde_json::Value> = serde_json::Deserializer::from_str(out)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            AppError::Network(format!("Failed to parse talosctl mc JSON: {e}"))
        })?;
    let primary = stream
        .iter()
        .find(|v| v["metadata"]["id"].as_str() == Some("v1alpha1"))
        .or_else(|| stream.iter().find(|v| v["spec"].is_string()));
    let spec = primary
        .and_then(|v| v["spec"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::Network("talosctl get mc returned no spec".to_string()))?;
    // The primary resource spec may itself be a multi-doc YAML (machine
    // config + standalone config documents). Keep only the machine config.
    Ok(spec.split("\n---\n").next().unwrap_or(&spec).to_string())
}

// ─── Kubernetes upgrade helpers ──────────────────────────────────────────────
//
// `talosctl upgrade-k8s` (verified on Talos v1.13.7) performs an in-place k8s
// control-plane + kubelet upgrade: it takes ONE control-plane endpoint,
// discovers the rest of the cluster itself, pre-pulls images, and patches every
// node's machineconfig. No node reboots. It validates the path up front — an
// unsupported from→to yields "unsupported upgrade path X.Y->A.B" on the first
// line and exit 1 — which is exactly the signal we use to discover which
// targets this Talos build supports (`k8s_upgrade_supported`).

/// Parse "v1.36.3" / "1.36.3" → (1, 36, 3). None when not a plain version.
pub fn parse_k8s_version(v: &str) -> Option<(u32, u32, u32)> {
    let v = v.trim().trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let major: u32 = parts[0].parse().ok()?;
    let minor: u32 = parts[1].parse().ok()?;
    let patch: u32 = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Compare two k8s versions; None when either is unparseable.
pub fn cmp_k8s_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    Some(parse_k8s_version(a)?.cmp(&parse_k8s_version(b)?))
}

/// Latest GA patch for a minor line from the Kubernetes release API
/// (https://github.com/kubernetes/kubernetes/releases). Fails open: callers
/// treat an error as "no candidates" rather than breaking the UI.
pub async fn latest_k8s_patch_for_minor(minor: u32) -> Option<String> {
    let url = format!(
        "https://api.github.com/repos/kubernetes/kubernetes/tags?per_page=100"
    );
    let resp = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "tcs-k8s-release-check")
        .send()
        .await
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let tags = v.as_array()?;
    let mut best: Option<String> = None;
    for t in tags {
        let name = t.get("name")?.as_str()?.to_string();
        let (ma, mi, pa) = parse_k8s_version(&name)?;
        if ma == 1 && mi == minor && pa > 0 {
            let bump = best
                .as_ref()
                .and_then(|b| parse_k8s_version(b))
                .map(|b| pa > b.2)
                .unwrap_or(true);
            if bump {
                best = Some(name);
            }
        }
    }
    best
}

/// Candidate k8s upgrade targets for a cluster's current version:
///   * the newest patch of the current minor (when not already there),
///   * the newest patch of the next minor.
/// Each candidate is verified against the node with a `upgrade-k8s --dry-run`
/// probe; only supported ones are returned.
pub async fn k8s_upgrade_candidates(
    endpoint: &str,
    current: &str,
    talosconfig: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let (ma, mi, pa) = parse_k8s_version(current)
        .ok_or_else(|| AppError::InvalidInput(format!("unparseable k8s version: {current}")))?;
    if ma != 1 {
        return Ok(Vec::new());
    }

    let mut candidates: Vec<String> = Vec::new();
    if let Ok(latest_same) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        latest_k8s_patch_for_minor(mi),
    )
    .await
    {
        if let Some(v) = latest_same {
            if parse_k8s_version(&v).map(|p| p.2 > pa).unwrap_or(false) {
                candidates.push(v);
            }
        }
    }
    if let Ok(latest_next) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        latest_k8s_patch_for_minor(mi + 1),
    )
    .await
    {
        if let Some(v) = latest_next {
            candidates.push(v);
        }
    }
    candidates.sort();

    let mut supported = Vec::new();
    for c in candidates {
        match TalosctlClient::k8s_upgrade_supported(endpoint, current, &c, talosconfig).await {
            Ok(true) => supported.push(c),
            Ok(false) => {}
            Err(e) => warn!(candidate = %c, error = %e, "k8s upgrade probe failed"),
        }
    }
    Ok(supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_from_item_parses() {
        let v = serde_json::json!({
            "metadata": { "id": 1, "phase": "running" },
            "spec": {
                "image": "1.sqsh",
                "metadata": { "name": "schematic", "version": "fd096f676a", "author": "Image Factory" }
            }
        });
        let ext = TalosctlClient::extension_from_item(&v).unwrap();
        assert_eq!(ext.id, "schematic");
        assert_eq!(ext.source, "1.sqsh");
        assert_eq!(ext.hash, "fd096f676a");
    }

    #[test]
    fn extension_from_item_missing_name_is_none() {
        let v = serde_json::json!({ "metadata": { "id": 0 }, "spec": { "image": "0.sqsh" } });
        assert!(TalosctlClient::extension_from_item(&v).is_none());
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

    #[test]
    fn spec_from_mc_json_extracts_opaque_spec() {
        let out = r#"{
  "node": "192.168.1.200",
  "metadata": {
    "namespace": "config",
    "type": "MachineConfigs.config.talos.dev",
    "id": "v1alpha1"
  },
  "spec": "version: v1alpha1\nmachine:\n  type: controlplane\n  network:\n    hostname: 914333-infra01\ncluster:\n  clusterName: kronos\n"
}"#;
        let spec = spec_from_mc_json(out).unwrap();
        assert!(spec.starts_with("version: v1alpha1"));
        assert!(spec.contains("914333-infra01"));
        assert!(!spec.contains("\"spec\""));
    }

    #[test]
    fn spec_from_mc_json_handles_multi_doc_stream_and_splits_spec() {
        let out = format!(
            r#"{{"node":"192.168.1.200","metadata":{{"id":"persistent"}},"spec":"version: v1alpha1\nmachine:\n  type: worker\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\n"}}{{"node":"192.168.1.200","metadata":{{"id":"v1alpha1"}},"spec":"version: v1alpha1\nmachine:\n  type: worker\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\n"}}"#
        );
        let spec = spec_from_mc_json(&out).unwrap();
        assert!(spec.contains("type: worker"));
        assert!(!spec.contains("VLANConfig"));
    }

    #[test]
    fn spec_from_mc_json_errors_on_missing_spec() {
        let err = spec_from_mc_json(r#"{"node":"x","metadata":{}}"#).unwrap_err();
        assert!(err.to_string().contains("no spec"));
    }

    #[test]
    fn merge_appends_standalone_doc_and_dedupes_by_kind_name() {
        let current = "version: v1alpha1\nmachine:\n  type: worker\ncluster:\n  clusterName: demo\n";
        let patch = "machine:\n  network:\n    interfaces:\n      - interface: bond0\n        bond:\n          mode: 802.3ad\n          interfaces:\n            - eno49\n            - eno50\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\n";
        let merged = merge_yaml_docs_into_machine_config(current, patch).unwrap();
        assert!(merged.contains("kind: VLANConfig"));
        assert!(merged.contains("vlanID: 207"));
        assert!(merged.contains("mode: 802.3ad"));

        // Second merge with the same VLANConfig must replace, not duplicate.
        let patch2 = "machine:\n  network:\n    interfaces:\n      - interface: bond0\n        bond:\n          mode: active-backup\n          interfaces:\n            - eno49\n            - eno50\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 208\nparent: bond0\n";
        let merged2 = merge_yaml_docs_into_machine_config(&merged, patch2).unwrap();
        assert_eq!(merged2.matches("kind: VLANConfig").count(), 1);
        assert!(merged2.contains("vlanID: 208"));
        assert!(merged2.contains("mode: active-backup"));
    }

    #[test]
    fn merge_drops_standalone_vlan_duplicated_by_nested_vlans() {
        // Current config carries a standalone VLANConfig for bond0.207.
        let current = "version: v1alpha1\nmachine:\n  type: worker\n  network:\n    interfaces:\n      - interface: bond0\n        bond:\n          mode: 802.3ad\n          interfaces:\n            - eno49\n            - eno50\ncluster:\n  clusterName: demo\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\nup: true\naddresses:\n- address: 162.242.191.68/26\n";
        // Patch expresses the same vlan nested under the interface.
        let patch = "machine:\n  network:\n    interfaces:\n      - interface: bond0\n        bond:\n          mode: 802.3ad\n          interfaces:\n            - eno49\n            - eno50\n        vlans:\n          - vlanId: 207\n            addresses:\n              - 162.242.191.68/26\n            routes:\n              - network: 0.0.0.0/0\n                gateway: 162.242.191.65\n                metric: 100\n";
        let merged = merge_yaml_docs_into_machine_config(current, patch).unwrap();
        assert!(merged.contains("vlans:"));
        assert!(merged.contains("vlanId: 207"));
        // The standalone doc for the same parent+vlan must be gone.
        assert!(!merged.contains("kind: VLANConfig"));
    }

    #[test]
    fn merge_keeps_standalone_vlan_not_expressed_nested() {
        let current = "version: v1alpha1\nmachine:\n  type: worker\ncluster:\n  clusterName: demo\n---\napiVersion: v1alpha1\nkind: VLANConfig\nname: bond0.207\nvlanID: 207\nparent: bond0\n";
        // Patch adds a different vlan nested; the existing standalone stays.
        let patch = "machine:\n  network:\n    interfaces:\n      - interface: bond0\n        vlans:\n          - vlanId: 300\n";
        let merged = merge_yaml_docs_into_machine_config(current, patch).unwrap();
        assert!(merged.contains("kind: VLANConfig"));
        assert!(merged.contains("vlanID: 207"));
        assert!(merged.contains("vlanId: 300"));
    }
}
