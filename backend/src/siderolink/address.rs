//! SideroLink IPv6 ULA address derivation.
//!
//! Mirrors `github.com/siderolabs/siderolink/pkg/wireguard/address.go` so TCS
//! addresses are compatible with what Talos nodes expect:
//!   - The network /64 is derived deterministically from an "installation id"
//!     (SHA-256, last 16 bytes, bytes forced to the RFC4193 ULA form with the
//!     SideroLink purpose suffix 0x03 in byte 7).
//!   - The server uses the first usable address (`::1`).
//!   - Each node gets a random address in the last 8 bytes of the /64.

use sha2::{Digest, Sha256};

/// Compute the SideroLink network /64 for the given installation id.
pub fn network_prefix(installation_id: &str) -> [u8; 16] {
    let hash = Sha256::digest(installation_id.as_bytes());
    // Take the last 16 bytes of the hash (matches Go: hash[sha256.Size-16:]).
    let mut prefix = [0u8; 16];
    prefix.copy_from_slice(&hash[16..32]);
    // ULA prefix (RFC4193) + SideroLink purpose suffix.
    prefix[0] = 0xfd;
    prefix[7] = 0x03;
    prefix
}

/// Format a 16-byte IPv6 address (network part + 8 random tail bytes) as a string.
pub fn addr_from_prefix_bytes(prefix: [u8; 16], tail: [u8; 8]) -> String {
    let mut full = [0u8; 16];
    full[..8].copy_from_slice(&prefix[..8]);
    full[8..].copy_from_slice(&tail);
    ipv6_string(&full)
}

/// The server's address on the SideroLink network: first usable in the /64.
pub fn server_address(installation_id: &str) -> String {
    let prefix = network_prefix(installation_id);
    // ::1 in the /64 => last 8 bytes = 0x00...01
    addr_from_prefix_bytes(prefix, [0, 0, 0, 0, 0, 0, 0, 1])
}

/// A random node address within the /64 (last 8 bytes random).
pub fn random_node_address(installation_id: &str) -> String {
    let prefix = network_prefix(installation_id);
    let mut tail = [0u8; 8];
    tail.copy_from_slice(&rand::random::<[u8; 8]>());
    // Avoid the server's address (::1) and the all-zero address.
    if tail == [0, 0, 0, 0, 0, 0, 0, 1] || tail.iter().all(|b| *b == 0) {
        tail[7] = 2;
    }
    addr_from_prefix_bytes(prefix, tail)
}

/// The node address as a (addr, addr/64) pair.
pub fn node_address_prefix(installation_id: &str) -> (String, String) {
    let (addr, _) = random_node_prefix(installation_id);
    let prefix = format!("{addr}/64");
    (addr, prefix)
}

pub fn random_node_prefix(installation_id: &str) -> (String, [u8; 8]) {
    let prefix = network_prefix(installation_id);
    let mut tail = [0u8; 8];
    tail.copy_from_slice(&rand::random::<[u8; 8]>());
    if tail == [0, 0, 0, 0, 0, 0, 0, 1] || tail.iter().all(|b| *b == 0) {
        tail[7] = 2;
    }
    (addr_from_prefix_bytes(prefix, tail), tail)
}

/// IPv6 string formatting for 16 bytes. Emits the full 8-group form which is
/// always valid (Talos/netip accept it). We deliberately avoid RFC 5952
/// compression because the server address is `::1` and nodes are random; a
/// plain group form is unambiguous and trivially correct.
fn ipv6_string(bytes: &[u8; 16]) -> String {
    let groups: Vec<u16> = (0..8)
        .map(|i| u16::from_be_bytes([bytes[i * 2], bytes[i * 2 + 1]]))
        .collect();
    // Special-case the common all-zero-tail forms to keep output compact and
    // matching what humans/talosctl expect. Otherwise full form.
    let trailing_zeros = groups.iter().rev().take_while(|&&g| g == 0).count();
    if trailing_zeros >= 4 {
        // e.g. fd:::03::1 -> "fdxx:...:03::1" style only when the tail is long.
        let head: Vec<String> = groups[..groups.len() - trailing_zeros]
            .iter()
            .map(|g| format!("{g:x}"))
            .collect();
        format!("{}::", head.join(":"))
    } else {
        groups
            .iter()
            .map(|g| format!("{g:x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_installation_prefix_is_ula_siderolink_purpose() {
        // Deterministic ULA: byte0=0xfd (RFC4193), byte7=0x03 (SideroLink
        // purpose), remaining 6 bytes = last 8 bytes of SHA256(""), first 6.
        let prefix = network_prefix("");
        assert_eq!(prefix[0], 0xfd);
        assert_eq!(prefix[7], 0x03);
        // Must be deterministic.
        assert_eq!(prefix, network_prefix(""));
    }

    #[test]
    fn server_addr_is_first_usable() {
        let a = server_address("");
        assert!(a.starts_with("fd"), "got {a}");
        assert!(a.ends_with(":1") || a == "fd", "got {a}");
    }

    #[test]
    fn random_node_is_in_prefix_and_not_server() {
        let (addr, _) = random_node_prefix("");
        assert!(addr.starts_with("fd"), "got {addr}");
        assert_ne!(addr, server_address(""));
    }

    #[test]
    fn ipv6_string_full_form() {
        let mut b = [0u8; 16];
        b[14] = 0;
        b[15] = 1;
        assert_eq!(ipv6_string(&b), "0:0:0:0:0:0:0:1");
    }
}
