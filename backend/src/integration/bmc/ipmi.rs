//! Classic IPMI via `ipmitool` subprocess (lanplus).

use std::process::Stdio;
use tokio::process::Command;

use super::{BootTarget, BmcCredentials, PowerState};
use crate::AppError;

pub struct IpmiClient {
    host: String,
    user: String,
    password: String,
    interface: String,
}

impl IpmiClient {
    pub fn new(creds: &BmcCredentials) -> Result<Self, AppError> {
        let host = creds
            .address
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();
        if host.is_empty() {
            return Err(AppError::InvalidInput("Invalid BMC address for IPMI".into()));
        }
        Ok(Self {
            host,
            user: creds.username.clone(),
            password: creds.password.clone(),
            interface: if creds.ipmi_interface.is_empty() {
                "lanplus".into()
            } else {
                creds.ipmi_interface.clone()
            },
        })
    }

    async fn run(&self, args: &[&str]) -> Result<String, AppError> {
        // Ensure ipmitool exists
        let check = Command::new("ipmitool")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if check.is_err() || !check.map(|s| s.success()).unwrap_or(false) {
            return Err(AppError::Network(
                "ipmitool not found on PATH; install ipmitool or use Redfish".into(),
            ));
        }

        let mut cmd = Command::new("ipmitool");
        cmd.arg("-I")
            .arg(&self.interface)
            .arg("-H")
            .arg(&self.host)
            .arg("-U")
            .arg(&self.user)
            .arg("-P")
            .arg(&self.password);
        for a in args {
            cmd.arg(a);
        }
        let out = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| AppError::Network(format!("ipmitool spawn: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let stderr_trimmed = stderr.trim();
        if !out.status.success() || stderr_trimmed.to_lowercase().starts_with("error:") {
            return Err(AppError::Network(format!(
                "ipmitool failed: {} {}",
                stdout.trim(),
                stderr_trimmed
            )));
        }
        Ok(stdout)
    }

    pub async fn get_power_state(&self) -> Result<PowerState, AppError> {
        let out = self.run(&["chassis", "power", "status"]).await?;
        let lower = out.to_ascii_lowercase();
        if lower.contains("is on") {
            Ok(PowerState::On)
        } else if lower.contains("is off") {
            Ok(PowerState::Off)
        } else {
            Ok(PowerState::Unknown)
        }
    }

    pub async fn power(&self, action: &str) -> Result<(), AppError> {
        let sub = match action {
            "on" => "on",
            "off" => "off",
            "reset" => "reset",
            "cycle" => "cycle",
            other => {
                return Err(AppError::InvalidInput(format!(
                    "Unknown power action: {other}"
                )))
            }
        };
        let _ = self.run(&["chassis", "power", sub]).await?;
        Ok(())
    }

    pub async fn set_boot(&self, target: BootTarget, _once: bool) -> Result<(), AppError> {
        match target {
            BootTarget::Pxe => {
                let _ = self.run(&["chassis", "bootdev", "pxe"]).await?;
            }
            BootTarget::Disk => {
                let _ = self.run(&["chassis", "bootdev", "disk"]).await?;
            }
        }
        Ok(())
    }

    /// Start an interactive Serial-over-LAN session.
    ///
    /// Spawns a long-lived `ipmitool … sol activate` process under a PTY (via
    /// util-linux `script -qec`) with piped stdio. ipmitool's SOL loop calls
    /// `tcgetattr` on stdin to enter raw terminal mode; when stdin is a plain
    /// pipe that fails and ipmitool activates the SOL session but never streams
    /// serial data (blank terminal). A PTY makes `tcgetattr` succeed so the
    /// session actually flows. The caller owns the returned `Child` and bridges
    /// its stdin/stdout (e.g. over a WebSocket). The BMC password is passed via
    /// the `TCS_SOL_PW` env var (referenced inside the `script -c` shell string)
    /// to keep it out of the argv / shell. Sending the SOL escape `~.` (or
    /// dropping the child) ends the session.
    pub async fn sol_activate(&self) -> Result<tokio::process::Child, AppError> {
        let check = Command::new("ipmitool")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if check.is_err() || !check.map(|s| s.success()).unwrap_or(false) {
            return Err(AppError::Network(
                "ipmitool not found on PATH; install ipmitool for SOL console".into(),
            ));
        }
        let script_check = Command::new("script")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        if script_check.is_err() || !script_check.map(|s| s.success()).unwrap_or(false) {
            return Err(AppError::Network(
                "util-linux `script` not found on PATH; needed for the SOL PTY".into(),
            ));
        }

        // inner command; host/user are operator-controlled (validated at entry),
        // password via env var to avoid argv/shell exposure.
        let inner = format!(
            "ipmitool -I {} -H {} -U {} -P \"$TCS_SOL_PW\" -N 0 sol activate",
            shell_quote(&self.interface),
            shell_quote(&self.host),
            shell_quote(&self.user),
        );

        let mut cmd = Command::new("script");
        cmd.arg("-qec") // quiet, echo off, command
            .arg(&inner)
            .arg("/dev/null"); // type-out file discarded
        cmd.env("TCS_SOL_PW", &self.password);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| AppError::Network(format!("script sol spawn: {e}")))?;
        Ok(child)
    }
}

/// Quote a shell word for safe inclusion in a `script -c` command string.
fn shell_quote(s: &str) -> String {
    // Single-quote, escaping embedded single quotes (' -> '\'').
    format!("'{}'", s.replace('\'', r"'\''"))
}
