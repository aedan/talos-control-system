//! Thin SSH/SCP client for the in-place (non-Talos -> Talos) conversion path.
//!
//! TCS runs on the deployer host; the operator places the trusted private key
//! there (nodes already accept it). We shell out to `ssh`/`scp` with
//! BatchMode (no interactive password) and `accept-new` host-key checking so a
//! fresh node is trusted on first contact without a prompt. There is no
//! per-node credential store — one `SshConfig` (user/key/port/timeout) applies
//! to every node, reached by its k8s InternalIP.

use std::path::Path;
use std::time::Duration;

use std::process::Stdio;

use tokio::process::Command;

use crate::config::SshConfig;
use crate::AppError;

#[derive(Clone)]
pub struct SshClient {
    cfg: SshConfig,
}

/// Outcome of a command whose non-zero exit is not itself an error (the output
/// is the data we inspect).
pub struct SshOutput {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl SshClient {
    pub fn new(cfg: SshConfig) -> Self {
        Self { cfg }
    }

    fn base_opts(&self) -> Vec<String> {
        let mut o = vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            // Trust a node on first contact, but reject a *changed* key.
            "StrictHostKeyChecking=accept-new".to_string(),
            "-o".to_string(),
            "IdentitiesOnly=yes".to_string(),
            "-o".to_string(),
            "LogLevel=ERROR".to_string(),
            "-o".to_string(),
            format!("ConnectTimeout={}", self.cfg.timeout_secs.min(10)),
        ];
        o.push("-p".into());
        o.push(self.cfg.port.to_string());
        if !self.cfg.key_path.trim().is_empty() {
            o.push("-i".into());
            o.push(self.cfg.key_path.trim().to_string());
        }
        o
    }

    fn target(&self, host: &str) -> String {
        format!("{}@{}", self.cfg.user, host)
    }

    async fn run_with_timeout(
        &self,
        bin: &str,
        args: &[String],
    ) -> Result<std::process::Output, AppError> {
        let fut = Command::new(bin)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        match tokio::time::timeout(Duration::from_secs(self.cfg.timeout_secs), fut).await {
            Ok(res) => res.map_err(|e| AppError::Network(format!("{bin} spawn: {e}"))),
            Err(_) => Err(AppError::Network(format!(
                "{bin} timed out after {}s",
                self.cfg.timeout_secs
            ))),
        }
    }

    /// Run `cmd` on the host and return stdout. Errors on non-zero exit.
    pub async fn run(&self, host: &str, cmd: &str) -> Result<String, AppError> {
        let o = self.run_capture(host, cmd).await?;
        if !o.ok {
            return Err(AppError::Network(format!(
                "ssh {host} command failed: {} {}",
                o.stdout.trim(),
                o.stderr.trim()
            )));
        }
        Ok(o.stdout)
    }

    /// Run `cmd` on the host, returning `(ok, stdout, stderr)`.
    pub async fn run_capture(&self, host: &str, cmd: &str) -> Result<SshOutput, AppError> {
        let mut args = self.base_opts();
        args.push(self.target(host));
        args.push(cmd.to_string());
        let out = self.run_with_timeout("ssh", &args).await?;
        Ok(SshOutput {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }

    /// Reachability probe: `ssh host true`.
    pub async fn ping(&self, host: &str) -> Result<(), AppError> {
        let o = self.run_capture(host, "true").await?;
        if !o.ok {
            return Err(AppError::Network(format!("ssh {host} unreachable: {}", o.stderr.trim())));
        }
        Ok(())
    }

    /// Copy a remote file back to the deployer. Returns the local byte size.
    pub async fn scp_back(&self, host: &str, remote: &str, local: &Path) -> Result<u64, AppError> {
        let mut args = self.base_opts();
        args.push("-q".into()); // quiet
        args.push(format!("{}:{remote}", self.target(host)));
        args.push(local.to_string_lossy().to_string());
        let out = self.run_with_timeout("scp", &args).await?;
        if !out.status.success() {
            return Err(AppError::Network(format!(
                "scp back {host}:{remote}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(std::fs::metadata(local).map(|m| m.len()).unwrap_or(0))
    }

    /// Copy a local file to the host.
    pub async fn scp_to(&self, host: &str, local: &Path, remote: &str) -> Result<(), AppError> {
        let mut args = self.base_opts();
        args.push("-q".into());
        args.push(local.to_string_lossy().to_string());
        args.push(format!("{}:{remote}", self.target(host)));
        let out = self.run_with_timeout("scp", &args).await?;
        if !out.status.success() {
            return Err(AppError::Network(format!(
                "scp to {host}:{remote}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_opts_default_key() {
        let c = SshClient::new(SshConfig::default());
        let o = c.base_opts();
        assert!(o.contains(&"-i".to_string()) == false);
        assert!(o.contains(&"-p".to_string()));
        assert!(o.contains(&"22".to_string()));
        assert!(o.contains(&"BatchMode=yes".to_string()));
    }

    #[test]
    fn base_opts_explicit_key() {
        let mut cfg = SshConfig::default();
        cfg.key_path = "/id_ed25519".into();
        cfg.port = 2222;
        let c = SshClient::new(cfg);
        let o = c.base_opts();
        assert!(o.contains(&"-i".to_string()));
        assert!(o.contains(&"/id_ed25519".to_string()));
        assert!(o.contains(&"2222".to_string()));
    }

    #[test]
    fn target_formats_user_host() {
        let c = SshClient::new(SshConfig { user: "admin".into(), ..Default::default() });
        assert_eq!(c.target("10.0.0.5"), "admin@10.0.0.5");
    }
}
