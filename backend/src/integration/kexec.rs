//! kexec-based Talos install for the in-place conversion path.
//!
//! Instead of BMC + PXE network boot, we `kexec` the node from its running OS
//! into the Talos installer's kernel + initramfs (kept in RAM, no firmware
//! touch), then drive `talosctl install` once it's up. The installer's
//! kernel/initramfs are the boot assets:
//!   * standard installer (`ghcr.io/siderolabs/installer:<v>`) — the plain
//!     `vmlinuz-<arch>` + `initramfs-<arch>.xz` from the GitHub release mirror
//!     (the same files PXE uses);
//!   * factory installer (`factory.talos.dev/metal-installer/<schematic>:<v>`)
//!     — the OCI image that bakes in the operator-selected kernel modules; we
//!     extract its kernel/initramfs with `skopeo` (requires skopeo on the
//!     deployer).
//!
//! The pure helpers (asset URLs, kexec command, module->driver mapping) are
//! unit-tested; the network/SSH orchestration is best-effort and needs a real
//! node to validate.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::FactoryConfig;
use crate::AppError;

use super::ssh::SshClient;

/// Locally-resolved installer boot assets on the deployer.
#[derive(Debug, Clone)]
pub struct KexecAssets {
    pub kernel_path: String,
    pub initramfs_path: String,
}

/// The installer image reference (what the node installs / what we pull).
#[derive(Debug, Clone)]
pub struct InstallerImage {
    pub ref_: String,
    /// Whether this image bakes in custom kernel modules (factory schematic).
    pub has_modules: bool,
}

/// Build the installer image ref for a (version, modules) pair.
pub fn installer_image(
    factory: &FactoryConfig,
    version: &str,
    modules: &[String],
    schematic_for_modules: Option<&str>,
) -> InstallerImage {
    if modules.is_empty() {
        InstallerImage {
            ref_: format!("ghcr.io/siderolabs/installer:{}", norm_version(version)),
            has_modules: false,
        }
    } else {
        let schematic = schematic_for_modules
            .expect("schematic id required when modules are selected");
        InstallerImage {
            ref_: factory.installer_image(schematic, version),
            has_modules: true,
        }
    }
}

/// Standard installer kernel/initramfs URLs from the release mirror.
pub fn standard_asset_urls(mirror_base: &str, version: &str, arch: &str) -> (String, String) {
    let v = norm_version(version);
    let base = mirror_base.trim_end_matches('/');
    (
        format!("{base}/{v}/vmlinuz-{arch}"),
        format!("{base}/{v}/initramfs-{arch}.xz"),
    )
}

/// The `--append` kernel cmdline for kexec'ing into the installer.
pub fn kexec_append(extra: &str) -> String {
    let mut a = String::from("console=tty0 console=ttyS0 talos.platform=metal");
    if !extra.trim().is_empty() {
        a.push(' ');
        a.push_str(extra.trim());
    }
    a
}

/// The remote shell command that loads + executes the kexec (reboots the node
/// into the installer). Kernel/initramfs must already be on the node.
pub fn kexec_command(kernel_remote: &str, initramfs_remote: &str, append: &str) -> String {
    format!(
        "kexec -l {kernel_remote} --initrd={initramfs_remote} --append='{append}' && kexec -e"
    )
}

const KERNEL_REMOTE: &str = "/tmp/tcs-kexec-vmlinuz";
const INITRAMFS_REMOTE: &str = "/tmp/tcs-kexec-initramfs.xz";

/// Transfer the boot assets to the node and kexec into the installer.
///
/// The final `kexec -e` reboots the node, so the SSH session drops — a
/// non-zero / connection-lost result on that step is treated as success.
pub async fn kexec_node(
    ssh: &SshClient,
    host: &str,
    assets: &KexecAssets,
    append: &str,
) -> Result<(), AppError> {
    ssh.scp_to(host, Path::new(&assets.kernel_path), KERNEL_REMOTE).await?;
    ssh.scp_to(host, Path::new(&assets.initramfs_path), INITRAMFS_REMOTE).await?;
    let o = ssh
        .run_capture(host, &kexec_command(KERNEL_REMOTE, INITRAMFS_REMOTE, append))
        .await?;
    // kexec -e reboots the host; the ssh connection is torn down mid-command,
    // so `ok` is often false. Only treat it as a hard failure if the failure
    // happened *before* the reboot (a real kexec error message).
    if !o.ok && looks_like_kexec_error(&o.stderr) {
        return Err(AppError::Network(format!("kexec failed on {host}: {}", o.stderr.trim())));
    }
    Ok(())
}

fn looks_like_kexec_error(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("kexec:")
        || s.contains("kexec: command not found")
        || s.contains("cannot open")
        || s.contains("no such file")
        || s.contains("kexec_load")
}

fn norm_version(v: &str) -> String {
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

// ── asset resolution (deployer-local) ────────────────────────────────────

async fn download_file(url: &str, dest: &Path) -> Result<(), AppError> {
    if let Ok(meta) = tokio::fs::metadata(dest).await {
        if meta.len() > 0 {
            return Ok(());
        }
    }
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(AppError::Io)?;
    }
    let resp = reqwest::get(url)
        .await
        .map_err(|e| AppError::Network(format!("download {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "download {url}: HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp.bytes().await.map_err(|e| AppError::Network(format!("download {url}: {e}")))?;
    let tmp = dest.with_extension("partial");
    tokio::fs::write(&tmp, &bytes).await.map_err(AppError::Io)?;
    tokio::fs::rename(&tmp, dest).await.map_err(AppError::Io)?;
    Ok(())
}

/// Resolve the standard installer's kernel + initramfs by downloading them
/// from the release mirror into the asset dir.
pub async fn resolve_standard_assets(
    mirror_base: &str,
    version: &str,
    arch: &str,
    asset_dir: &Path,
) -> Result<KexecAssets, AppError> {
    let (kurl, iurl) = standard_asset_urls(mirror_base, version, arch);
    let dir = asset_dir.join(norm_version(version)).join(arch);
    let kernel = dir.join(format!("vmlinuz-{arch}"));
    let initramfs = dir.join(format!("initramfs-{arch}.xz"));
    download_file(&kurl, &kernel).await?;
    download_file(&iurl, &initramfs).await?;
    Ok(KexecAssets {
        kernel_path: kernel.to_string_lossy().to_string(),
        initramfs_path: initramfs.to_string_lossy().to_string(),
    })
}

/// Resolve a factory installer image's kernel + initramfs by pulling the OCI
/// image with `skopeo` and extracting the boot files from its layers.
///
/// Requires `skopeo` on the deployer. This is the path that carries the
/// operator-selected kernel modules into the kexeced installer.
pub async fn resolve_factory_assets(
    image_ref: &str,
    asset_dir: &Path,
) -> Result<KexecAssets, AppError> {
    let probe = tokio::process::Command::new("skopeo")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
    if probe.is_err() || !probe.map(|s| s.success()).unwrap_or(false) {
        return Err(AppError::Network(
            "skopeo not found on the deployer; install skopeo to convert nodes with custom kernel modules".into(),
        ));
    }
    let key = format!("factory-{}", md5_short(image_ref));
    let dir = asset_dir.join(key);
    let kernel = dir.join("vmlinuz");
    let initramfs = dir.join("initramfs.xz");
    if kernel.is_file() && initramfs.is_file() {
        return Ok(KexecAssets {
            kernel_path: kernel.to_string_lossy().to_string(),
            initramfs_path: initramfs.to_string_lossy().to_string(),
        });
    }
    let oci_dir = dir.join("oci");
    tokio::fs::create_dir_all(&dir).await.map_err(AppError::Io)?;
    let out = tokio::process::Command::new("skopeo")
        .args(["copy", "--overwrite", &format!("docker://{image_ref}"), &format!("oci:{}", oci_dir.display())])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("skopeo spawn: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Network(format!(
            "skopeo copy {image_ref}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    // Unpack each layer tar (newest last) and look for the boot files.
    let blobs = oci_dir.join("blobs");
    let mut found_kernel: Option<std::path::PathBuf> = None;
    let mut found_init: Option<std::path::PathBuf> = None;
    let entries = list_dir_recursive(&blobs).await?;
    for e in entries {
        if !e.extension().map(|x| x == "tar").unwrap_or(false) {
            continue;
        }
        match extract_boot_files(&e, &kernel, &initramfs).await {
            Ok(()) => {}
            Err(_) => continue,
        }
        if found_kernel.is_none() && kernel.is_file() {
            found_kernel = Some(kernel.clone());
        }
        if found_init.is_none() && initramfs.is_file() {
            found_init = Some(initramfs.clone());
        }
        if found_kernel.is_some() && found_init.is_some() {
            break;
        }
    }
    if !kernel.is_file() || !initramfs.is_file() {
        return Err(AppError::Network(format!(
            "could not extract kernel/initramfs from factory image {image_ref}"
        )));
    }
    Ok(KexecAssets {
        kernel_path: kernel.to_string_lossy().to_string(),
        initramfs_path: initramfs.to_string_lossy().to_string(),
    })
}

/// Stream-extract a layer tar, copying any `vmlinuz*` / `initramfs*` file out
/// to the dest paths. Uses the `tar` binary (present on the deployer).
async fn extract_boot_files(
    layer: &Path,
    kernel_dest: &Path,
    initramfs_dest: &Path,
) -> Result<(), AppError> {
    let work = std::env::temp_dir().join(format!("tcs-kexec-{}", md5_short(&layer.to_string_lossy())));
    tokio::fs::create_dir_all(&work).await.map_err(AppError::Io)?;
    let out = tokio::process::Command::new("tar")
        .args(["-xf", &layer.to_string_lossy(), "-C", &work.to_string_lossy()])
        .output()
        .await
        .map_err(|e| AppError::Network(format!("tar spawn: {e}")))?;
    if !out.status.success() {
        let _ = tokio::fs::remove_dir_all(&work).await;
        return Err(AppError::Network(format!(
            "tar extract: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    for f in list_dir_recursive(&work).await? {
        let name = f.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let nlow = name.to_lowercase();
        if (nlow.starts_with("vmlinuz") && kernel_dest.is_file() == false)
            && is_file(&f).await
        {
            copy(&f, kernel_dest).await?;
        } else if (nlow.starts_with("initramfs") && initramfs_dest.is_file() == false)
            && is_file(&f).await
        {
            copy(&f, initramfs_dest).await?;
        }
    }
    let _ = tokio::fs::remove_dir_all(&work).await;
    Ok(())
}

async fn copy(src: &Path, dest: &Path) -> Result<(), AppError> {
    if let Some(p) = dest.parent() {
        tokio::fs::create_dir_all(p).await.map_err(AppError::Io)?;
    }
    tokio::fs::copy(src, dest).await.map_err(AppError::Io)?;
    Ok(())
}

async fn is_file(p: &Path) -> bool {
    tokio::fs::metadata(p).await.map(|m| m.is_file()).unwrap_or(false)
}

async fn list_dir_recursive(dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut rd = match tokio::fs::read_dir(&d).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        while let Ok(Some(e)) = rd.next_entry().await {
            let p = e.path();
            if e.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// Tiny stable hash for a cache key (avoids pulling the md5 crate).
fn md5_short(s: &str) -> String {
    // FNV-1a 64-bit, hex. Not cryptographic — only a local cache key.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_image_no_modules() {
        let img = installer_image(
            &FactoryConfig::default(),
            "1.13.7",
            &[],
            None,
        );
        assert_eq!(img.ref_, "ghcr.io/siderolabs/installer:v1.13.7");
        assert!(!img.has_modules);
    }

    #[test]
    fn factory_image_with_modules() {
        let img = installer_image(&FactoryConfig::default(), "v1.13.7", &["siderolabs/bnx2-bnx2x".into()], Some("abc123"));
        assert!(img.has_modules);
        assert!(img.ref_.starts_with("factory.talos.dev/metal-installer/abc123"));
    }

    #[test]
    fn standard_asset_urls_use_mirror_and_version() {
        let (k, i) = standard_asset_urls(
            "https://github.com/siderolabs/talos/releases/download",
            "v1.13.7",
            "amd64",
        );
        assert!(k.ends_with("/v1.13.7/vmlinuz-amd64"));
        assert!(i.ends_with("/v1.13.7/initramfs-amd64.xz"));
    }

    #[test]
    fn kexec_command_shape() {
        let c = kexec_command("/tmp/vmlinuz", "/tmp/initramfs.xz", "console=ttyS0");
        assert!(c.starts_with("kexec -l /tmp/vmlinuz"));
        assert!(c.contains("--initrd=/tmp/initramfs.xz"));
        assert!(c.ends_with("kexec -e"));
    }

    #[test]
    fn kexec_append_defaults_and_extra() {
        assert!(kexec_append("").contains("talos.platform=metal"));
        assert!(kexec_append(" talos.systemextensions.enabled=true ").contains("systemextensions.enabled=true"));
    }

    #[test]
    fn reboot_dropout_is_not_an_error() {
        // connection reset / no kexec error text -> success
        assert!(!looks_like_kexec_error("Connection to 10.0.0.5 closed by remote host."));
        assert!(looks_like_kexec_error("kexec: command not found"));
    }
}
