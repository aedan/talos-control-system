//! Live metal config: overlay file + restart DHCP/PXE listeners without process restart.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{MetalConfig, MetalDhcpConfig, MetalPxeConfig};
use crate::db::pool::DbPool;
use crate::network::dhcp::{self, DhcpServerConfig};
use crate::network::pxe;
use crate::network::tftp;
use crate::AppError;

/// Runtime metal subsystem: mutable config + service handles.
pub struct MetalRuntime {
    pub config: Arc<RwLock<MetalConfig>>,
    pool: DbPool,
    data_dir: PathBuf,
    dhcp_handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    pxe_handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    tftp_handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

impl MetalRuntime {
    pub fn overlay_path(data_dir: &Path) -> PathBuf {
        data_dir.join("metal.toml")
    }

    /// Merge base config with optional `$data_dir/metal.toml` overlay.
    pub fn load_merged(base: &MetalConfig, data_dir: &Path) -> MetalConfig {
        let path = Self::overlay_path(data_dir);
        if !path.is_file() {
            return base.clone();
        }
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<MetalOverlayFile>(&s) {
                Ok(overlay) => {
                    info!(path = %path.display(), "Loaded metal config overlay");
                    overlay.apply_to(base.clone())
                }
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "Invalid metal.toml overlay; using base");
                    base.clone()
                }
            },
            Err(e) => {
                warn!(error = %e, "Failed to read metal.toml");
                base.clone()
            }
        }
    }

    pub fn start(pool: DbPool, base: MetalConfig, data_dir: impl Into<PathBuf>) -> Arc<Self> {
        let data_dir = data_dir.into();
        let merged = Self::load_merged(&base, &data_dir);
        let rt = Arc::new(Self {
            config: Arc::new(RwLock::new(merged.clone())),
            pool,
            data_dir,
            dhcp_handle: tokio::sync::Mutex::new(None),
            pxe_handle: tokio::sync::Mutex::new(None),
            tftp_handle: tokio::sync::Mutex::new(None),
        });
        // spawn initial services
        let rt2 = Arc::clone(&rt);
        tokio::spawn(async move {
            if let Err(e) = rt2.rebind_services().await {
                warn!(error = %e, "Initial metal service bind failed");
            }
        });
        rt
    }

    pub async fn snapshot(&self) -> MetalConfig {
        self.config.read().await.clone()
    }

    pub async fn write_overlay_and_apply(&self, next: MetalConfig) -> Result<MetalConfig, AppError> {
        validate_metal(&next)?;
        let path = Self::overlay_path(&self.data_dir);
        let body = toml::to_string_pretty(&MetalOverlayFile::from_config(&next))
            .map_err(|e| AppError::Internal(format!("metal.toml encode: {e}")))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(AppError::Io)?;
        }
        std::fs::write(&path, body).map_err(AppError::Io)?;
        info!(path = %path.display(), "Wrote metal config overlay");

        {
            let mut w = self.config.write().await;
            *w = next.clone();
        }
        self.rebind_services().await?;
        Ok(next)
    }

    pub async fn rebind_services(&self) -> Result<(), AppError> {
        let cfg = self.config.read().await.clone();

        // Stop previous listeners
        {
            let mut h = self.dhcp_handle.lock().await;
            if let Some(handle) = h.take() {
                handle.abort();
                info!("Stopped previous DHCP server task");
            }
        }
        {
            let mut h = self.pxe_handle.lock().await;
            if let Some(handle) = h.take() {
                handle.abort();
                info!("Stopped previous PXE HTTP server task");
            }
        }
        {
            let mut h = self.tftp_handle.lock().await;
            if let Some(handle) = h.take() {
                handle.abort();
                info!("Stopped previous TFTP server task");
            }
        }

        // PXE
        if cfg.pxe.enabled || (cfg.enabled && cfg.pxe.enabled) {
            let _ = pxe::ensure_default_profile(&self.pool, &cfg.pxe).await;
        }
        let next_server = resolve_next_server(&cfg.dhcp);
        if let Some(handle) = pxe::spawn_pxe_server(
            self.pool.clone(),
            cfg.pxe.clone(),
            &next_server.to_string(),
        ) {
            *self.pxe_handle.lock().await = Some(handle);
        }

        // TFTP (legacy PXE chainloader)
        if cfg.pxe.tftp_enabled {
            if let Some(handle) = tftp::spawn_tftp_server(
                self.pool.clone(),
                cfg.pxe.asset_dir.clone(),
            ) {
                *self.tftp_handle.lock().await = Some(handle);
            }
        }

        // DHCP
        let http_boot_base = if next_server.is_unspecified() {
            format!("http://127.0.0.1:{}", cfg.pxe.http_port)
        } else {
            format!("http://{}:{}", next_server, cfg.pxe.http_port)
        };
        if let Some(handle) = dhcp::spawn_dhcp_server(
            self.pool.clone(),
            DhcpServerConfig {
                dhcp: cfg.dhcp.clone(),
                next_server,
                http_boot_base,
                boot_file: String::new(),
                tftp_enabled: cfg.pxe.tftp_enabled,
                ipxe_bios_file: cfg.pxe.ipxe_bios_file.clone(),
                ipxe_uefi_file: cfg.pxe.ipxe_uefi_file.clone(),
            },
        ) {
            *self.dhcp_handle.lock().await = Some(handle);
        }

        if cfg.dhcp.enabled {
            info!(
                iface = %cfg.dhcp.interface,
                "Metal DHCP (re)started"
            );
        }
        if cfg.pxe.enabled {
            info!(port = cfg.pxe.http_port, "Metal PXE HTTP (re)started");
        }
        if cfg.pxe.tftp_enabled {
            info!("Metal TFTP (re)started");
        }
        Ok(())
    }
}

fn resolve_next_server(dhcp: &MetalDhcpConfig) -> Ipv4Addr {
    if !dhcp.bind_ip.is_empty() {
        if let Ok(ip) = Ipv4Addr::from_str(dhcp.bind_ip.trim()) {
            return ip;
        }
    }
    if !dhcp.interface.is_empty() {
        if let Some(ip) = dhcp::interface_ipv4(&dhcp.interface) {
            return ip;
        }
    }
    Ipv4Addr::new(0, 0, 0, 0)
}

fn validate_metal(cfg: &MetalConfig) -> Result<(), AppError> {
    if cfg.dhcp.enabled {
        if cfg.dhcp.interface.trim().is_empty() && cfg.dhcp.bind_ip.trim().is_empty() {
            return Err(AppError::InvalidInput(
                "metal.dhcp.enabled requires interface or bind_ip".into(),
            ));
        }
    }
    Ok(())
}

/// TOML shape written to metal.toml (same fields as MetalConfig).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct MetalOverlayFile {
    enabled: Option<bool>,
    dhcp: Option<MetalDhcpConfig>,
    pxe: Option<MetalPxeConfig>,
    bmc: Option<crate::config::MetalBmcConfig>,
}

impl MetalOverlayFile {
    fn from_config(c: &MetalConfig) -> Self {
        Self {
            enabled: Some(c.enabled),
            dhcp: Some(c.dhcp.clone()),
            pxe: Some(c.pxe.clone()),
            bmc: Some(c.bmc.clone()),
        }
    }

    fn apply_to(self, mut base: MetalConfig) -> MetalConfig {
        if let Some(e) = self.enabled {
            base.enabled = e;
        }
        if let Some(d) = self.dhcp {
            base.dhcp = d;
        }
        if let Some(p) = self.pxe {
            base.pxe = p;
        }
        if let Some(b) = self.bmc {
            base.bmc = b;
        }
        base
    }
}
