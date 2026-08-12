//! Maintenance-mode Talos client via `talosctl` subprocess.
//!
//! The Talos installer runs in maintenance mode with a self-signed cert that
//! Rust's rustls stack cannot connect to without `InsecureSkipVerify`.
//! talosctl handles this natively with `-i/--insecure`, so we shell out for
//! PXE installer-phase operations only.

use std::process::Stdio;
use tokio::process::Command;
use tracing::info;

use crate::AppError;

pub struct TalosctlClient;

impl TalosctlClient {
    /// Discover available disks on a PXE-installing machine.
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

        // talosctl outputs pretty-printed JSON objects, one per disk.
        // Each object starts with '{' at column 0.
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

    /// Apply a machine config to a PXE-installing machine and reboot.
    pub async fn apply_config(endpoint: &str, config_yaml: &str, reboot: bool) -> Result<(), AppError> {
        Self::ensure_installed().await?;

        let tmpfile = format!("/tmp/tcs-install-config-{:x}.yaml", std::process::id());
        tokio::fs::write(&tmpfile, config_yaml)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to write temp config: {e}")))?;

        let mode = if reboot { "reboot" } else { "no-reboot" };

        let out = Command::new("talosctl")
            .args([
                "apply-config", "-f", &tmpfile, "-i",
                "-e", endpoint, "-n", endpoint,
                "-m", mode,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("talosctl spawn: {e}")))?;

        let _ = tokio::fs::remove_file(&tmpfile).await;

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);

        if !out.status.success() {
            return Err(AppError::Network(format!(
                "talosctl apply-config failed: {} {}",
                stdout.trim(),
                stderr.trim()
            )));
        }

        info!(endpoint, mode, "talosctl apply_config");
        Ok(())
    }

    async fn ensure_installed() -> Result<(), AppError> {
        // Best-effort check — if command is on PATH, proceed.
        match Command::new("talosctl")
            .arg("--help")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
        {
            Ok(s) if s.success() => Ok(()),
            _ => Err(AppError::Network(
                "talosctl not found on PATH; install talosctl (required for PXE provisioning)"
                    .to_string(),
            )),
        }
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
