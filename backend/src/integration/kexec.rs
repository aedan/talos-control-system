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
//!     — the OCI image that bakes in the operator-selected kernel modules.
//!     The factory image ships a single `vmlinuz.efi` (UEFI combined
//!     kernel+initrd) which we kexec directly without `--initrd`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::FactoryConfig;
use crate::AppError;

use super::network_capture::{NodeNetworkBond, NodeNetworkCapture, NodeNetworkInterface, NodeNetworkVlan};
use super::ssh::SshClient;

/// Locally-resolved installer boot assets on the deployer.
///
/// `initramfs_path` is empty when the kernel is a self-contained UEFI
/// `vmlinuz.efi` (factory image) that embeds its own initrd.
#[derive(Debug, Clone)]
pub struct KexecAssets {
    pub kernel_path: String,
    pub initramfs_path: String,
}

impl KexecAssets {
    /// True when the kernel embeds its initrd (factory `vmlinuz.efi`).
    pub fn is_combined(&self) -> bool {
        self.initramfs_path.is_empty()
    }
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
///
/// Builds the full parameter set the Talos installer needs to come up with
/// network on a statically-configured node:
///   - required: `talos.platform=metal`, `slab_nomerge`, `pti=on`, consoles
///   - kernel-level `bond=` + `ip=` so LACP forms at kernel time (switch may
///     suspend ports that don't speak LACP)
///   - `talos.config.early=<zstd|b64>` — a minimal machine config carrying the
///     node's static network (bond/IP/gw/mtu/dns) so it is applied post-boot
///     and persisted into STATE.
///
/// `extra` allows the caller to append further params.
pub fn kexec_append(network: &NodeNetworkCapture, hostname: &str, extra: &str) -> String {
    let mut a = String::from(
        "console=tty0 console=ttyS0 talos.platform=metal slab_nomerge pti=on",
    );

    // Kernel-level static network so the link is up before Talos userspace.
    if let Some(bond) = network.bonds.first() {
        let mode = talos_bond_mode(&bond.mode);
        let slaves = bond.slaves.join(",");
        a.push_str(&format!(" bond={}:{}:mode={}", bond.name, slaves, mode));
        // ip=<client>::<gw>:<netmask>::<dev>:<dns>:
        if let Some((ip, cidr)) = address_for_bond(network, bond) {
            let mask = prefix_to_mask(&cidr).unwrap_or_else(|| "255.255.255.0".into());
            a.push_str(&format!(
                " ip={ip}::{gw}:{mask}::{dev}:none",
                gw = network.gateway,
                dev = bond.name,
            ));
        }
    } else if let Some(first) = network.interfaces.first() {
        if !first.ip.is_empty() {
            let mask = prefix_to_mask(first.cidr.as_str()).unwrap_or_else(|| "255.255.255.0".into());
            a.push_str(&format!(
                " ip={ip}::{gw}:{mask}::{dev}:none",
                ip = first.ip,
                gw = network.gateway,
                dev = first.name,
            ));
        }
    }

    // talos.config.early: minimal machine config (zstd|b64) with the network.
    if let Some(early) = build_talos_config_early(network, hostname) {
        a.push_str(&format!(" talos.config.early={early}"));
    }

    if !extra.trim().is_empty() {
        a.push(' ');
        a.push_str(extra.trim());
    }
    a
}

/// Render a minimal Talos machine config (network only) and encode it as
/// `zstd | base64` for the `talos.config.early=` kernel param.
fn build_talos_config_early(network: &NodeNetworkCapture, hostname: &str) -> Option<String> {
    let yaml = minimal_machine_config_yaml(network, hostname);
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s' \"$1\" | zstd -q --no-progress -19 | base64 -w0")
        .arg("")
        .arg(&yaml)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// A minimal MachineConfig YAML carrying just the network section. Kept small
/// so it fits the 4096-byte kernel cmdline budget after zstd|b64.
fn minimal_machine_config_yaml(network: &NodeNetworkCapture, hostname: &str) -> String {
    let mut y = String::new();
    y.push_str("apiVersion: v1alpha1\nkind: Config\nmetadata:\n  name: talos.yaml\nmachine:\n  network:\n");
    if let Some(bond) = network.bonds.first() {
        let mode = talos_bond_mode(&bond.mode);
        let mtu = bond_mtu(network, bond);
        // ignore non-bond physical NICs to avoid DHCP probe delays
        for i in &network.interfaces {
            if !bond.slaves.contains(&i.name.to_string()) {
                y.push_str(&format!("    interfaces:\n"));
                break;
            }
        }
        // We always emit the interfaces list: bond first, then ignored slaves' siblings.
        // Rebuild cleanly:
        let mut interfaces_yaml = String::new();
        interfaces_yaml.push_str(&format!("      - interface: {}\n        mtu: {}\n        bond:\n", bond.name, mtu));
        interfaces_yaml.push_str(&format!("          mode: {}\n", mode));
        interfaces_yaml.push_str(&format!("          miimon: 100\n"));
        interfaces_yaml.push_str(&format!("          updelay: 200\n"));
        interfaces_yaml.push_str(&format!("          downdelay: 200\n"));
        interfaces_yaml.push_str(&format!("          xmitHashPolicy: layer3+4\n"));
        interfaces_yaml.push_str(&format!("          lacpRate: slow\n"));
        interfaces_yaml.push_str(&format!("          interfaces:\n"));
        for s in &bond.slaves {
            interfaces_yaml.push_str(&format!("            - {}\n", s));
        }
        if let Some((ip, cidr)) = address_for_bond(network, bond) {
            interfaces_yaml.push_str(&format!("        addresses:\n          - address: {}/{}\n", ip, cidr));
        }
        if !network.gateway.is_empty() {
            interfaces_yaml.push_str(&format!("        routes:\n          - destination: 0.0.0.0/0\n            gateway: {}\n", network.gateway));
        }
        // ignore other NICs
        for i in &network.interfaces {
            if !bond.slaves.contains(&i.name.to_string()) {
                interfaces_yaml.push_str(&format!("      - interface: {}\n        ignore: true\n", i.name));
            }
        }
        // VLANs on the bond
        let vlans_on_bond: Vec<&NodeNetworkVlan> =
            network.vlans.iter().filter(|v| v.parent == bond.name).collect();
        if !vlans_on_bond.is_empty() {
            // append vlans under the bond interface
            interfaces_yaml.push_str(&format!("        vlans:\n"));
            for v in &vlans_on_bond {
                interfaces_yaml.push_str(&format!("          - vlanId: {}\n", v.id));
                if let Some((vip, vcidr)) = network
                    .interfaces
                    .iter()
                    .find(|i| i.name == v.name)
                    .filter(|i| !i.ip.is_empty())
                    .map(|i| (i.ip.clone(), i.cidr.clone()))
                {
                    interfaces_yaml.push_str(&format!("            addresses:\n              - address: {}/{}\n", vip, vcidr));
                    interfaces_yaml.push_str(&format!("            mtu: {}\n", mtu));
                }
                if !network.gateway.is_empty() {
                    interfaces_yaml.push_str(&format!("            routes:\n              - destination: 0.0.0.0/0\n                gateway: {}\n", network.gateway));
                }
            }
        }
        y.push_str("    interfaces:\n");
        y.push_str(&interfaces_yaml);
    } else if let Some(first) = network.interfaces.first() {
        y.push_str("    interfaces:\n");
        y.push_str(&format!("      - interface: {}\n        mtu: {}\n", first.name, first.mtu));
        if !first.ip.is_empty() {
            y.push_str(&format!("        addresses:\n          - address: {}{}\n", first.ip, if first.cidr.is_empty() { String::new() } else { format!("/{}", first.cidr) }));
        }
        if !network.gateway.is_empty() {
            y.push_str(&format!("        routes:\n          - destination: 0.0.0.0/0\n            gateway: {}\n", network.gateway));
        }
    }
    if !network.dns.is_empty() {
        y.push_str("    nameservers:\n");
        for d in &network.dns {
            y.push_str(&format!("      - {}\n", d));
        }
    }
    let _ = hostname; // hostname is set via talos.hostname / machine config later
    y
}

/// /sys/class/net/<bond>/bonding/mode number -> Talos bond mode string.
fn talos_bond_mode(mode: &str) -> &'static str {
    match mode.trim() {
        "0" | "balance-rr" => "balance-rr",
        "1" | "active-backup" => "active-backup",
        "4" | "802.3ad" | "lacp" => "802.3ad",
        "5" | "balance-tlb" => "balance-tlb",
        "6" | "balance-alb" => "balance-alb",
        _ => "802.3ad",
    }
}

/// IP prefix length -> dotted-decimal netmask.
fn prefix_to_mask(prefix: &str) -> Option<String> {
    let p: u32 = prefix.trim().parse().ok()?;
    if p == 0 || p > 32 {
        return None;
    }
    let mask = 0xFFFF_FFFFu32 << (32 - p);
    Some(format!(
        "{}.{}.{}.{}",
        (mask >> 24) & 0xFF,
        (mask >> 16) & 0xFF,
        (mask >> 8) & 0xFF,
        mask & 0xFF
    ))
}

fn bond_mtu(network: &NodeNetworkCapture, bond: &NodeNetworkBond) -> u32 {
    network
        .interfaces
        .iter()
        .find(|i| bond.slaves.contains(&i.name.to_string()))
        .map(|i| i.mtu)
        .unwrap_or(1500)
}

fn address_for_bond(
    network: &NodeNetworkCapture,
    bond: &NodeNetworkBond,
) -> Option<(String, String)> {
    for v in &network.vlans {
        if v.parent == bond.name {
            if let Some(a) = network
                .interfaces
                .iter()
                .find(|i| i.name == v.name)
                .filter(|i| !i.ip.is_empty())
                .map(|i| (i.ip.clone(), i.cidr.clone()))
            {
                return Some(a);
            }
        }
    }
    network
        .interfaces
        .iter()
        .find(|i| i.name == bond.name && !i.ip.is_empty())
        .map(|i| (i.ip.clone(), i.cidr.clone()))
}

/// The remote shell command that loads + executes the kexec (reboots the node
/// into the installer). Kernel (and initramfs, unless combined) must already
/// be on the node.
pub fn kexec_command(kernel_remote: &str, initramfs_remote: &str, append: &str) -> String {
    if initramfs_remote.is_empty() {
        // Factory UEFI combined image: no separate initrd.
        format!(
            "kexec -l {kernel_remote} --append='{append}' && kexec -e"
        )
    } else {
        format!(
            "kexec -l {kernel_remote} --initrd={initramfs_remote} --append='{append}' && kexec -e"
        )
    }
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
    if !assets.initramfs_path.is_empty() {
        ssh.scp_to(host, Path::new(&assets.initramfs_path), INITRAMFS_REMOTE).await?;
    }
    let init = if assets.initramfs_path.is_empty() { String::new() } else { INITRAMFS_REMOTE.to_string() };
    let o = ssh
        .run_capture(host, &kexec_command(KERNEL_REMOTE, &init, append))
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

/// Resolve a factory installer image's boot assets by pulling the OCI image
/// with `skopeo` and extracting the kernel from its layers.
///
/// The factory metal-installer image ships a single `vmlinuz.efi` (UEFI
/// combined kernel+initrd) — no separate `initramfs.xz`. We extract that file
/// and return it with an empty `initramfs_path` (signalling "combined").
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
    // Cache: accept either a combined vmlinuz.efi OR a classic vmlinuz+initramfs pair.
    let kernel_efi = dir.join("vmlinuz.efi");
    let kernel = dir.join("vmlinuz");
    let initramfs = dir.join("initramfs.xz");
    if kernel_efi.is_file() {
        return Ok(KexecAssets {
            kernel_path: kernel_efi.to_string_lossy().to_string(),
            initramfs_path: String::new(),
        });
    }
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
    // Unpack each layer tar and look for vmlinuz.efi / vmlinuz / initramfs.
    let blobs = oci_dir.join("blobs");
    let mut found_efi = false;
    let mut found_kernel = false;
    let mut found_init = false;
    let entries = list_dir_recursive(&blobs).await?;
    for e in entries {
        if !e.extension().map(|x| x == "tar").unwrap_or(false) {
            continue;
        }
        match extract_boot_files(&e, &kernel_efi, &kernel, &initramfs).await {
            Ok(()) => {}
            Err(_) => continue,
        }
        if !found_efi && kernel_efi.is_file() {
            found_efi = true;
        }
        if !found_kernel && kernel.is_file() {
            found_kernel = true;
        }
        if !found_init && initramfs.is_file() {
            found_init = true;
        }
        if found_efi || (found_kernel && found_init) {
            break;
        }
    }
    if found_efi {
        return Ok(KexecAssets {
            kernel_path: kernel_efi.to_string_lossy().to_string(),
            initramfs_path: String::new(),
        });
    }
    if found_kernel && found_init {
        return Ok(KexecAssets {
            kernel_path: kernel.to_string_lossy().to_string(),
            initramfs_path: initramfs.to_string_lossy().to_string(),
        });
    }
    Err(AppError::Network(format!(
        "could not extract boot assets (vmlinuz.efi or vmlinuz+initramfs) from factory image {image_ref}"
    )))
}

/// Stream-extract a layer tar, copying any `vmlinuz.efi` / `vmlinuz*` /
/// `initramfs*` file out to the dest paths. Uses the `tar` binary (present on
/// the deployer).
async fn extract_boot_files(
    layer: &Path,
    kernel_efi_dest: &Path,
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
        if is_file(&f).await {
            // Factory UEFI combined kernel.
            if nlow.starts_with("vmlinuz") && nlow.ends_with(".efi") && !kernel_efi_dest.is_file() {
                copy(&f, kernel_efi_dest).await?;
            } else if nlow.starts_with("vmlinuz") && !kernel_dest.is_file() {
                // Classic bare-metal vmlinuz (not .efi).
                copy(&f, kernel_dest).await?;
            } else if nlow.starts_with("initramfs") && !initramfs_dest.is_file() {
                copy(&f, initramfs_dest).await?;
            }
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
    fn kexec_command_shape_separate_initrd() {
        let c = kexec_command("/tmp/vmlinuz", "/tmp/initramfs.xz", "console=ttyS0");
        assert!(c.starts_with("kexec -l /tmp/vmlinuz"));
        assert!(c.contains("--initrd=/tmp/initramfs.xz"));
        assert!(c.ends_with("kexec -e"));
    }

    #[test]
    fn kexec_command_shape_combined_efi_no_initrd() {
        let c = kexec_command("/tmp/vmlinuz.efi", "", "console=ttyS0 talos.platform=metal");
        assert!(c.starts_with("kexec -l /tmp/vmlinuz.efi"));
        assert!(!c.contains("--initrd="));
        assert!(c.ends_with("kexec -e"));
    }

    #[test]
    fn kexec_append_includes_required_params() {
        let net = NodeNetworkCapture::default();
        let a = kexec_append(&net, "host1", "");
        assert!(a.contains("talos.platform=metal"));
        assert!(a.contains("slab_nomerge"));
        assert!(a.contains("pti=on"));
    }

    #[test]
    fn kexec_append_bond_injects_ip_and_bond_params() {
        let net = NodeNetworkCapture {
            interfaces: vec![
                NodeNetworkInterface { name: "bond0".into(), mtu: 1500, ip: "172.20.0.38".into(), cidr: "24".into(), mac: String::new() },
            ],
            bonds: vec![NodeNetworkBond { name: "bond0".into(), mode: "4".into(), slaves: vec!["eno1".into(), "eno2".into()] }],
            vlans: vec![],
            gateway: "172.20.0.1".into(),
            dns: vec![],
            ovs_bridges: vec![],
        };
        let a = kexec_append(&net, "host1", "");
        assert!(a.contains("bond=bond0:eno1,eno2:mode=802.3ad"));
        assert!(a.contains("ip=172.20.0.38::172.20.0.1:255.255.255.0::bond0:none"));
        assert!(a.contains("talos.config.early="));
    }

    #[test]
    fn prefix_to_mask_works() {
        assert_eq!(prefix_to_mask("24").as_deref(), Some("255.255.255.0"));
        assert_eq!(prefix_to_mask("32").as_deref(), Some("255.255.255.255"));
        assert_eq!(prefix_to_mask("16").as_deref(), Some("255.255.0.0"));
        assert_eq!(prefix_to_mask("0"), None);
        assert_eq!(prefix_to_mask("33"), None);
    }

    #[test]
    fn reboot_dropout_is_not_an_error() {
        // connection reset / no kexec error text -> success
        assert!(!looks_like_kexec_error("Connection to 10.0.0.5 closed by remote host."));
        assert!(looks_like_kexec_error("kexec: command not found"));
    }

    #[test]
    fn assets_is_combined() {
        let a = KexecAssets { kernel_path: "/x/vmlinuz.efi".into(), initramfs_path: String::new() };
        assert!(a.is_combined());
        let b = KexecAssets { kernel_path: "/x/vmlinuz".into(), initramfs_path: "/x/initramfs.xz".into() };
        assert!(!b.is_combined());
    }
}
