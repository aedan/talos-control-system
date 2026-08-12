//! Minimal TFTP server (RFC 1350) for legacy PXE boot.
//!
//! Serves iPXE binaries (undionly.kpxe / snponly.efi) from `asset_dir`.
//! iPXE then chainloads the HTTP boot script served by the PXE HTTP server.

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
const BLOCK_SIZE: usize = 512;

/// Spawn the TFTP server when `metal.pxe.tftp_enabled`.
pub fn spawn_tftp_server(
    _pool: DbPool,
    asset_dir: String,
) -> Option<JoinHandle<()>> {
    Some(tokio::spawn(async move {
        if let Err(e) = run_tftp_loop(&asset_dir).await {
            warn!(error = %e, "TFTP server stopped");
        }
    }))
}

async fn run_tftp_loop(asset_dir: &str) -> Result<(), AppError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|e| AppError::Network(format!("TFTP socket: {e}")))?;
    socket
        .set_reuse_address(true)
        .map_err(|e| AppError::Network(format!("TFTP reuse: {e}")))?;
    socket
        .bind(&SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, TFTP_PORT)).into())
        .map_err(|e| AppError::Network(format!("TFTP bind :{TFTP_PORT}: {e}")))?;
    socket
        .set_nonblocking(true)
        .map_err(|e| AppError::Network(format!("TFTP nonblocking: {e}")))?;
    let std_sock: std::net::UdpSocket = socket.into();
    let sock = UdpSocket::from_std(std_sock)
        .map_err(|e| AppError::Network(format!("TFTP tokio socket: {e}")))?;

    info!("TFTP server listening on UDP/{TFTP_PORT} (asset dir {asset_dir})");

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
        // path traversal guard
        if req.filename.contains("..") || req.filename.contains('/') {
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
        tokio::spawn(async move {
            if let Err(e) = serve_file(src, &data).await {
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
    Some(Rrq { filename })
}

struct Rrq {
    filename: String,
}

/// Send file in 512-byte DATA blocks, waiting for ACK per block (RFC 1350).
///
/// Uses a dedicated ephemeral-port socket so ACKs are never stolen by the
/// main receive loop (UDP sockets have no per-connection demultiplexing).
async fn serve_file(client: SocketAddr, data: &[u8]) -> Result<(), AppError> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(|e| AppError::Network(format!("TFTP transfer socket: {e}")))?;

    let mut block: u16 = 1;
    let mut offset = 0usize;
    loop {
        let end = (offset + BLOCK_SIZE).min(data.len());
        let mut pkt = Vec::with_capacity(4 + (end - offset));
        pkt.push(0);
        pkt.push(3); // DATA
        pkt.extend_from_slice(&block.to_be_bytes());
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
                    if ack_block == block {
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

        if end == data.len() {
            // Last block was < 512 → transfer complete; ACK already received.
            if end - offset < BLOCK_SIZE {
                return Ok(());
            }
            // File is an exact multiple of 512 → send a final 0-byte DATA block.
            let mut final_pkt = vec![0u8; 4];
            final_pkt[2..4].copy_from_slice(&block.wrapping_add(1).to_be_bytes());
            final_pkt[1] = 3;
            sock.send_to(&final_pkt, client)
                .await
                .map_err(|e| AppError::Network(format!("TFTP send: {e}")))?;
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                sock.recv_from(&mut [0u8; 64]),
            )
            .await;
            return Ok(());
        }

        block = block.wrapping_add(1);
        offset = end;
    }
}

fn tftp_error(code: u16, msg: &str) -> Vec<u8> {
    let mut pkt = vec![0u8, 5, (code >> 8) as u8, code as u8];
    pkt.extend_from_slice(msg.as_bytes());
    pkt.push(0);
    pkt
}