//! Minimal TFTP server (RFC 1350) for legacy PXE boot.
//!
//! Serves PXE bootloaders (undionly.kpxe / pxelinux.0 / snponly.efi) and
//! Talos kernel/initramfs assets from `asset_dir`, including subdirectories
//! (PXELINUX config discovery). Optional blksize negotiation (RFC 2348).

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::db::pool::DbPool;
use crate::AppError;

const TFTP_PORT: u16 = 69;
const DEFAULT_BLOCK_SIZE: usize = 512;
const MAX_BLOCK_SIZE: usize = 1468;

/// Spawn the TFTP server when `metal.pxe.tftp_enabled`.
pub fn spawn_tftp_server(
    _pool: DbPool,
    asset_dir: String,
    bind_interface: String,
) -> Option<JoinHandle<()>> {
    Some(tokio::spawn(async move {
        if let Err(e) = run_tftp_loop(&asset_dir, &bind_interface).await {
            warn!(error = %e, "TFTP server stopped");
        }
    }))
}

async fn run_tftp_loop(asset_dir: &str, bind_interface: &str) -> Result<(), AppError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| AppError::Network(format!("TFTP socket: {e}")))?;
    socket
        .set_reuse_address(true)
        .map_err(|e| AppError::Network(format!("TFTP reuse: {e}")))?;
    if !bind_interface.is_empty() {
        if let Err(e) = socket.bind_device(Some(bind_interface.as_bytes())) {
            warn!(interface = bind_interface, error = %e, "TFTP: failed to bind to interface");
        }
    }
    socket
        .bind(&SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, TFTP_PORT)).into())
        .map_err(|e| AppError::Network(format!("TFTP bind :{TFTP_PORT}: {e}")))?;
    socket
        .set_nonblocking(true)
        .map_err(|e| AppError::Network(format!("TFTP nonblocking: {e}")))?;
    let std_sock: std::net::UdpSocket = socket.into();
    let sock = UdpSocket::from_std(std_sock)
        .map_err(|e| AppError::Network(format!("TFTP tokio socket: {e}")))?;

    info!(interface = bind_interface, "TFTP server listening on UDP/{TFTP_PORT} (asset dir {asset_dir})");

    let mut buf = vec![0u8; 1024];
    loop {
        let (n, src) = match tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
            .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                warn!(error = %e, "TFTP recv error");
                continue;
            }
            Err(_) => continue,
        };

        let Some(req) = parse_rrq(&buf[..n]) else {
            debug!(%src, "TFTP: non-RRQ packet ignored");
            continue;
        };

        let root: PathBuf = asset_dir.into();
        let path = root.join(&req.filename);
        // path traversal guard (subdirectories allowed for PXELINUX configs)
        if !req.filename.is_empty()
            && req.filename
                .split('/')
                .any(|seg| seg.is_empty() || seg == ".." || seg.contains('\\'))
        {
            debug!(file = %req.filename, "TFTP: rejected path");
            continue;
        }

        let data = match tokio::fs::read(&path).await {
            Ok(d) => d,
            Err(_) => {
                debug!(file = %req.filename, %src, "TFTP: file not found");
                let err = tftp_error(1, "File not found");
                let _ = sock.send_to(&err, src).await;
                continue;
            }
        };

        let filename = req.filename.clone();
        let block_size = req.block_size;
        tokio::spawn(async move {
            if let Err(e) = serve_file(src, &data, block_size).await {
                debug!(file = %filename, %src, error = %e, "TFTP transfer aborted");
            }
        });
    }
}

fn parse_rrq(pkt: &[u8]) -> Option<Rrq> {
    if pkt.len() < 4 || pkt[0] != 0 || pkt[1] != 1 {
        return None; // not an RRQ (opcode 1)
    }
    let body = &pkt[2..];
    let nul1 = body.iter().position(|&b| b == 0)?;
    let filename = String::from_utf8(body[..nul1].to_vec()).ok()?;
    let rest = &body[nul1 + 1..];
    let nul2 = rest.iter().position(|&b| b == 0)?;
    let mode = String::from_utf8(rest[..nul2].to_vec()).ok()?;
    if !mode.eq_ignore_ascii_case("octet") {
        return None;
    }
    let mut block_size = DEFAULT_BLOCK_SIZE;
    let opts = rest[nul2 + 1..].split(|&b| b == 0);
    let mut parts = opts.collect::<Vec<_>>().into_iter();
    while let Some(k) = parts.next() {
        let v = parts.next().unwrap_or_default();
        if k.eq_ignore_ascii_case(b"blksize") {
            if let Ok(n) = String::from_utf8_lossy(v).trim().parse::<usize>() {
                if (8..=MAX_BLOCK_SIZE).contains(&n) {
                    block_size = n;
                }
            }
        }
    }
    Some(Rrq {
        filename,
        block_size,
    })
}

struct Rrq {
    filename: String,
    block_size: usize,
}

/// Send file in blocks, waiting for ACK per block (RFC 1350 / RFC 2348).
///
/// When the client negotiated a blksize, reply with an OACK first. Block
/// numbers are tracked as u32 so transfers larger than 65535 blocks wrap
/// correctly (RFC 1350 rollover), and 512-byte multiples still terminate.
///
/// Uses a dedicated ephemeral-port socket so ACKs are never stolen by the
/// main receive loop (UDP sockets have no per-connection demultiplexing).
async fn serve_file(
    client: SocketAddr,
    data: &[u8],
    block_size: usize,
) -> Result<(), AppError> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(|e| AppError::Network(format!("TFTP transfer socket: {e}")))?;

    if block_size != DEFAULT_BLOCK_SIZE {
        let oack = format!("\x00\x06blksize\x00{block_size}\x00");
        sock.send_to(oack.as_bytes(), client)
            .await
            .map_err(|e| AppError::Network(format!("TFTP OACK: {e}")))?;
        let mut acked = false;
        for _ in 0..3 {
            let mut buf = [0u8; 64];
            match tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf)).await {
                Ok(Ok((_, src))) if src == client && buf[0] == 0 && buf[1] == 4 => {
                    acked = true;
                    break;
                }
                Ok(Ok(_)) => continue,
                Ok(Err(_)) | Err(_) => continue,
            }
        }
        if !acked {
            return Err(AppError::Network("TFTP OACK ack timeout".into()));
        }
    }

    let mut offset = 0usize;
    let mut block_no: u32 = 1;
    loop {
        let blk = (block_no as u16).to_be_bytes();
        let end = (offset + block_size).min(data.len());
        let mut pkt = Vec::with_capacity(4 + (end - offset));
        pkt.push(0);
        pkt.push(3); // DATA
        pkt.extend_from_slice(&blk);
        pkt.extend_from_slice(&data[offset..end]);
        sock.send_to(&pkt, client)
            .await
            .map_err(|e| AppError::Network(format!("TFTP send: {e}")))?;

        // Wait for ACK of this block (retry a few times)
        let mut acked = false;
        for _ in 0..3 {
            let mut buf = [0u8; 64];
            match tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf)).await
            {
                Ok(Ok((n, src))) if src == client && n >= 4 && buf[0] == 0 && buf[1] == 4 => {
                    let ack_block = u16::from_be_bytes([buf[2], buf[3]]);
                    if ack_block == block_no as u16 {
                        acked = true;
                        break;
                    }
                }
                Ok(Ok(_)) => continue, // ignore stray packets
                Ok(Err(_)) | Err(_) => continue,
            }
        }
        if !acked {
            return Err(AppError::Network("TFTP ack timeout".into()));
        }

        offset = end;
        if end == data.len() {
            // Last data block delivered. If it was a full block, send a final
            // 0-byte DATA block to signal EOF; otherwise the short block did.
            if data.len() % block_size == 0 {
                let mut final_pkt = vec![0u8; 4];
                final_pkt[2..4].copy_from_slice(&(block_no as u16).wrapping_add(1).to_be_bytes());
                final_pkt[1] = 3;
                sock.send_to(&final_pkt, client)
                    .await
                    .map_err(|e| AppError::Network(format!("TFTP send: {e}")))?;
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    sock.recv_from(&mut [0u8; 64]),
                )
                .await;
            }
            return Ok(());
        }
        block_no += 1;
    }
}

fn tftp_error(code: u16, msg: &str) -> Vec<u8> {
    let mut pkt = vec![0u8, 5, (code >> 8) as u8, code as u8];
    pkt.extend_from_slice(msg.as_bytes());
    pkt.push(0);
    pkt
}