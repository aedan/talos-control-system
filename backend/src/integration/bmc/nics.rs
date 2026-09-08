//! Discover host NIC MAC addresses from a BMC (Redfish + IPMI).
//!
//! PXE matches inventory by MAC. Operators often have iDRAC/iLO credentials
//! but not the host NIC MAC. We pull host Ethernet MACs from the BMC and
//! store them so DHCP/iPXE can identify the machine.

use serde_json::Value;

use crate::db::repos::machine::normalize_mac;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NicKind {
    Host,
    Bmc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NicInfo {
    pub mac: String,
    pub kind: NicKind,
    pub name: String,
    pub link_up: Option<bool>,
}

pub fn is_placeholder_mac(mac: &str) -> bool {
    let n = normalize_mac(mac);
    n.is_empty()
        || n == "00:00:00:00:00:00"
        || n == "ff:ff:ff:ff:ff:ff"
}

pub fn looks_like_bmc(label: &str) -> bool {
    let s = label.to_ascii_lowercase();
    s.contains("idrac")
        || s.contains("ilo")
        || s.contains("bmc")
        || s.contains("idrac mac")
        || s.contains("dedicated network")
        || s.contains("manager ethernet")
}

/// Pull colon/hyphen MACs out of free-form BMC text.
pub fn macs_in_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 17 <= bytes.len() {
        if is_mac_at(bytes, i) {
            let slice = &text[i..i + 17];
            let n = normalize_mac(slice);
            if !is_placeholder_mac(&n) && !out.contains(&n) {
                out.push(n);
            }
            i += 17;
            continue;
        }
        i += 1;
    }
    out
}

fn is_mac_at(bytes: &[u8], i: usize) -> bool {
    if i + 17 > bytes.len() {
        return false;
    }
    let sep = bytes[i + 2];
    if sep != b':' && sep != b'-' {
        return false;
    }
    for k in 0..6 {
        let p = i + k * 3;
        if !bytes[p].is_ascii_hexdigit() || !bytes[p + 1].is_ascii_hexdigit() {
            return false;
        }
        if k < 5 && bytes[p + 2] != sep {
            return false;
        }
    }
    true
}

/// Parse `ipmitool delloem mac` (Dell PowerEdge / iDRAC 7+).
pub fn parse_delloem_macs(stdout: &str) -> Vec<NicInfo> {
    let mut nics = Vec::new();
    for line in stdout.lines() {
        let macs = macs_in_text(line);
        if macs.is_empty() {
            continue;
        }
        let kind = if looks_like_bmc(line) {
            NicKind::Bmc
        } else {
            NicKind::Host
        };
        let name = line
            .split_whitespace()
            .take(3)
            .collect::<Vec<_>>()
            .join(" ");
        for mac in macs {
            if nics.iter().any(|n: &NicInfo| n.mac == mac) {
                continue;
            }
            nics.push(NicInfo {
                mac,
                kind,
                name: name.clone(),
                link_up: None,
            });
        }
    }
    nics
}

/// Parse `ipmitool lan print` — this is the BMC management MAC, not a host NIC.
pub fn parse_lan_print_macs(stdout: &str) -> Vec<NicInfo> {
    let mut nics = Vec::new();
    for line in stdout.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("mac address") {
            continue;
        }
        for mac in macs_in_text(line) {
            if nics.iter().any(|n: &NicInfo| n.mac == mac) {
                continue;
            }
            nics.push(NicInfo {
                mac,
                kind: NicKind::Bmc,
                name: "bmc-lan".into(),
                link_up: None,
            });
        }
    }
    nics
}

pub fn nic_from_redfish_interface(v: &Value) -> Option<NicInfo> {
    let mac_raw = v
        .get("MACAddress")
        .or_else(|| v.get("MacAddress"))
        .or_else(|| v.get("PermanentMACAddress"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if mac_raw.is_empty() {
        return None;
    }
    let mac = normalize_mac(mac_raw);
    if is_placeholder_mac(&mac) {
        return None;
    }
    let id = v.get("Id").and_then(|x| x.as_str()).unwrap_or("");
    let name = v
        .get("Name")
        .and_then(|x| x.as_str())
        .unwrap_or(id)
        .to_string();
    let desc = v
        .get("Description")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let label = format!("{id} {name} {desc}");
    let kind = if looks_like_bmc(&label) {
        NicKind::Bmc
    } else {
        NicKind::Host
    };
    let link_up = v.get("LinkStatus").and_then(|x| x.as_str()).map(|s| {
        s.eq_ignore_ascii_case("LinkUp") || s.eq_ignore_ascii_case("Up")
    });
    Some(NicInfo {
        mac,
        kind,
        name,
        link_up,
    })
}

/// Prefer a host NIC that is link-up; else the first host NIC; never a BMC
/// MAC if a host NIC exists (BMC dedicated ports do not PXE).
pub fn pick_primary_mac(nics: &[NicInfo]) -> Option<String> {
    let host: Vec<&NicInfo> = nics.iter().filter(|n| n.kind == NicKind::Host).collect();
    if let Some(n) = host.iter().find(|n| n.link_up == Some(true)) {
        return Some(n.mac.clone());
    }
    if let Some(n) = host.first() {
        return Some(n.mac.clone());
    }
    None
}

pub fn merge_nics(into: &mut Vec<NicInfo>, extra: Vec<NicInfo>) {
    for n in extra {
        if let Some(existing) = into.iter_mut().find(|e| e.mac == n.mac) {
            if existing.kind == NicKind::Bmc && n.kind == NicKind::Host {
                existing.kind = NicKind::Host;
            }
            if existing.name.is_empty() {
                existing.name = n.name;
            }
            if existing.link_up.is_none() {
                existing.link_up = n.link_up;
            }
        } else {
            into.push(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_delloem_lists_host_and_idrac() {
        let out = "\
NIC Number\tMAC Address
0\t78:2b:cb:11:22:33
1\t78:2b:cb:11:22:34
iDRAC MAC Address 78:2b:cb:11:22:35
";
        let nics = parse_delloem_macs(out);
        assert_eq!(nics.len(), 3);
        assert_eq!(nics[0].kind, NicKind::Host);
        assert_eq!(nics[0].mac, "78:2b:cb:11:22:33");
        assert_eq!(nics[2].kind, NicKind::Bmc);
        assert_eq!(nics[2].mac, "78:2b:cb:11:22:35");
        assert_eq!(
            pick_primary_mac(&nics).as_deref(),
            Some("78:2b:cb:11:22:33")
        );
    }

    #[test]
    fn pick_primary_skips_bmc_only() {
        let nics = vec![NicInfo {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            kind: NicKind::Bmc,
            name: "iDRAC".into(),
            link_up: Some(true),
        }];
        assert_eq!(pick_primary_mac(&nics), None);
    }

    #[test]
    fn pick_primary_prefers_link_up() {
        let nics = vec![
            NicInfo {
                mac: "00:11:22:33:44:01".into(),
                kind: NicKind::Host,
                name: "eth0".into(),
                link_up: Some(false),
            },
            NicInfo {
                mac: "00:11:22:33:44:02".into(),
                kind: NicKind::Host,
                name: "eth1".into(),
                link_up: Some(true),
            },
        ];
        assert_eq!(
            pick_primary_mac(&nics).as_deref(),
            Some("00:11:22:33:44:02")
        );
    }

    #[test]
    fn redfish_interface_reads_mac() {
        let v = serde_json::json!({
            "Id": "NIC.Integrated.1-1-1",
            "Name": "Integrated NIC 1 Port 1",
            "MACAddress": "D0:94:66:AA:BB:CC",
            "LinkStatus": "LinkUp"
        });
        let n = nic_from_redfish_interface(&v).unwrap();
        assert_eq!(n.mac, "d0:94:66:aa:bb:cc");
        assert_eq!(n.kind, NicKind::Host);
        assert_eq!(n.link_up, Some(true));
    }

    #[test]
    fn redfish_ilo_dedicated_is_bmc() {
        let v = serde_json::json!({
            "Id": "1",
            "Name": "iLO Dedicated Network Port",
            "MACAddress": "94:18:82:00:00:01"
        });
        let n = nic_from_redfish_interface(&v).unwrap();
        assert_eq!(n.kind, NicKind::Bmc);
    }

    #[test]
    fn lan_print_is_bmc() {
        let out = "Set in Progress         : Set Complete\nMAC Address             : 00:1e:67:12:34:56\nIP Address              : 172.24.16.82\n";
        let nics = parse_lan_print_macs(out);
        assert_eq!(nics.len(), 1);
        assert_eq!(nics[0].kind, NicKind::Bmc);
        assert_eq!(nics[0].mac, "00:1e:67:12:34:56");
    }
}
