//! PXE HTTP boot server: iPXE scripts + Talos kernel/initramfs assets.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::fs;
use tracing::{info, warn};

use crate::config::MetalPxeConfig;
use crate::db::pool::DbPool;
use crate::db::repos::{self, machine::normalize_mac, pxe_profile::PxeProfile};
use crate::AppError;

#[derive(Clone)]
pub struct PxeState {
    pub pool: DbPool,
    pub config: MetalPxeConfig,
    pub http_base: String,
}

/// Download Talos metal kernel+initramfs for a profile into asset_dir.
pub async fn sync_profile_assets(
    config: &MetalPxeConfig,
    profile: &mut PxeProfile,
) -> Result<(), AppError> {
    let ver = profile.talos_version.trim();
    let arch = if profile.arch.is_empty() {
        "amd64"
    } else {
        profile.arch.as_str()
    };
    let dir = PathBuf::from(&config.asset_dir).join(ver).join(arch);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::Io(e))?;

    let kernel_name = format!("vmlinuz-{arch}");
    let initrd_name = format!("initramfs-{arch}.xz");

    let kernel_url = if profile.kernel_url.is_empty() {
        format!(
            "{}/{ver}/{kernel_name}",
            config.mirror_base.trim_end_matches('/')
        )
    } else {
        profile.kernel_url.clone()
    };
    let initrd_url = if profile.initramfs_url.is_empty() {
        format!(
            "{}/{ver}/{initrd_name}",
            config.mirror_base.trim_end_matches('/')
        )
    } else {
        profile.initramfs_url.clone()
    };

    download_if_missing(&kernel_url, &dir.join(&kernel_name)).await?;
    download_if_missing(&initrd_url, &dir.join(&initrd_name)).await?;

    profile.kernel_url = kernel_url;
    profile.initramfs_url = initrd_url;
    profile.assets_ready = true;
    profile.updated_at = chrono::Utc::now();
    Ok(())
}

async fn download_if_missing(url: &str, dest: &Path) -> Result<(), AppError> {
    if dest.is_file() {
        let meta = fs::metadata(dest).await.map_err(AppError::Io)?;
        if meta.len() > 0 {
            return Ok(());
        }
    }
    info!(%url, path = %dest.display(), "Downloading PXE asset");
    let resp = reqwest::get(url)
        .await
        .map_err(|e| AppError::Network(format!("download {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Network(format!(
            "download {url}: HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Network(format!("download body: {e}")))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).await.map_err(AppError::Io)?;
    }
    let tmp = dest.with_extension("partial");
    fs::write(&tmp, &bytes).await.map_err(AppError::Io)?;
    fs::rename(&tmp, dest).await.map_err(AppError::Io)?;
    Ok(())
}

pub fn build_ipxe_script(
    http_base: &str,
    version: &str,
    arch: &str,
    extra_cmdline: &str,
    profile_cmdline: &str,
) -> String {
    let base = http_base.trim_end_matches('/');
    let kernel = format!("{base}/pxe/assets/{version}/{arch}/vmlinuz-{arch}");
    let initrd = format!("{base}/pxe/assets/{version}/{arch}/initramfs-{arch}.xz");
    let mut cmdline = format!(
        "initrd=initramfs-{arch}.xz talos.platform=metal slab_nomerge pti=on ip=dhcp"
    );
    if !profile_cmdline.is_empty() {
        cmdline.push(' ');
        cmdline.push_str(profile_cmdline);
    }
    if !extra_cmdline.is_empty() {
        cmdline.push(' ');
        cmdline.push_str(extra_cmdline);
    }
    format!(
        "#!ipxe\n\
         kernel {kernel} {cmdline}\n\
         initrd {initrd}\n\
         boot\n"
    )
}

async fn ipxe_for_mac(
    State(st): State<PxeState>,
    AxumPath(mac): AxumPath<String>,
) -> Response {
    let mac_n = normalize_mac(&mac);
    let machine = match repos::machine::get_by_mac(&st.pool, &mac_n).await {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "PXE MAC lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    let (version, arch, cmdline) = if let Some(ref m) = machine {
        if let Some(ref pid) = m.pxe_profile_id {
            if let Ok(Some(p)) = repos::pxe_profile::get(
                &st.pool,
                uuid::Uuid::parse_str(pid).unwrap_or_default(),
            )
            .await
            {
                (p.talos_version, p.arch, p.cmdline)
            } else {
                (
                    st.config.default_talos_version.clone(),
                    "amd64".into(),
                    String::new(),
                )
            }
        } else {
            (
                st.config.default_talos_version.clone(),
                "amd64".into(),
                String::new(),
            )
        }
    } else {
        (
            st.config.default_talos_version.clone(),
            "amd64".into(),
            String::new(),
        )
    };

    let script = build_ipxe_script(
        &st.http_base,
        &version,
        &arch,
        &st.config.extra_cmdline,
        &cmdline,
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain")],
        script,
    )
        .into_response()
}

async fn serve_asset(
    State(st): State<PxeState>,
    AxumPath((version, arch, file)): AxumPath<(String, String, String)>,
) -> Response {
    // path traversal guard
    if version.contains("..") || arch.contains("..") || file.contains("..") || file.contains('/') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = PathBuf::from(&st.config.asset_dir)
        .join(&version)
        .join(&arch)
        .join(&file);
    match fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn default_ipxe(State(st): State<PxeState>) -> Response {
    let script = build_ipxe_script(
        &st.http_base,
        &st.config.default_talos_version,
        "amd64",
        &st.config.extra_cmdline,
        "",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain")],
        script,
    )
        .into_response()
}

pub fn pxe_router(state: PxeState) -> Router {
    Router::new()
        .route("/pxe/ipxe", get(default_ipxe))
        .route("/pxe/ipxe/:mac", get(ipxe_for_mac))
        .route(
            "/pxe/assets/:version/:arch/:file",
            get(serve_asset),
        )
        .with_state(state)
}

/// Spawn PXE HTTP listener on metal.pxe.http_port.
pub fn spawn_pxe_server(
    pool: DbPool,
    config: MetalPxeConfig,
    bind_addr: &str,
) -> Option<tokio::task::JoinHandle<()>> {
    if !config.enabled {
        return None;
    }
    let port = config.http_port;
    let host = if bind_addr.is_empty() {
        "0.0.0.0".to_string()
    } else {
        bind_addr.to_string()
    };
    let http_base = format!("http://{host}:{port}");
    // Prefer advertised-style base via env override later; for DHCP next-server use bind_ip.
    let st = PxeState {
        pool,
        config: config.clone(),
        http_base: http_base.clone(),
    };
    let addr: SocketAddr = format!("{host}:{port}").parse().ok()?;
    Some(tokio::spawn(async move {
        let app = pxe_router(st);
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                info!(%addr, "PXE HTTP boot server listening");
                if let Err(e) = axum::serve(listener, app).await {
                    warn!(error = %e, "PXE HTTP server error");
                }
            }
            Err(e) => {
                warn!(error = %e, %addr, "Failed to bind PXE HTTP server");
            }
        }
    }))
}

/// Ensure default PXE profile exists.
pub async fn ensure_default_profile(
    pool: &DbPool,
    config: &MetalPxeConfig,
) -> Result<(), AppError> {
    let list = repos::pxe_profile::list(pool).await?;
    if !list.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now();
    let p = PxeProfile {
        id: uuid::Uuid::new_v4(),
        name: "default-metal".into(),
        talos_version: config.default_talos_version.clone(),
        arch: "amd64".into(),
        kernel_url: String::new(),
        initramfs_url: String::new(),
        cmdline: String::new(),
        enabled: true,
        assets_ready: false,
        created_at: now,
        updated_at: now,
    };
    repos::pxe_profile::create(pool, &p).await?;
    info!(id = %p.id, "Created default PXE profile");
    Ok(())
}
