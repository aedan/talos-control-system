//! Per-node network + driver capture over SSH, for the in-place conversion.
//!
//! Before a node is kexeced into Talos we must capture its host networking
//! (interfaces, bonds, VLANs, gateway, DNS, OVS bridges) and its active
//! kernel drivers, so the generated Talos machine config reproduces the same
//! networking and the operator can pick the matching Image Factory modules.
//!
//! The capture runs ONE combined shell command (best-effort, sectioned output)
//! and the pure `parse_capture` turns it into a `NodeNetworkCapture`. The parser
//! is unit-tested independently of SSH.

use crate::AppError;

use super::ssh::SshClient;

/// One interface: name, MTU, the first IPv4 (or any) address, prefix, MAC.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeNetworkInterface {
    pub name: String,
    pub mtu: u32,
    pub ip: String,
    pub cidr: String,
    pub mac: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeNetworkBond {
    pub name: String,
    pub mode: String,
    pub slaves: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeNetworkVlan {
    pub name: String,
    pub id: u32,
    pub parent: String,
}

/// The captured per-node networking (mirrors the frontend `ConvertNetwork`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NodeNetworkCapture {
    pub interfaces: Vec<NodeNetworkInterface>,
    pub bonds: Vec<NodeNetworkBond>,
    pub vlans: Vec<NodeNetworkVlan>,
    pub gateway: String,
    pub dns: Vec<String>,
    pub ovs_bridges: Vec<String>,
}

/// Capture result: networking + active kernel module (driver) names.
#[derive(Debug, Clone)]
pub struct CaptureResult {
    pub network: NodeNetworkCapture,
    pub drivers: Vec<String>,
}

pub struct NetworkCapture {
    ssh: SshClient,
}

impl NetworkCapture {
    pub fn new(ssh: SshClient) -> Self {
        Self { ssh }
    }

    /// The combined shell command that gathers everything in one round-trip.
    pub fn capture_command() -> &'static str {
        r#"set +e
echo "==INTERFACES=="; ip -j -o addr show 2>/dev/null
echo "==LINKS=="; ip -j -o link show 2>/dev/null
echo "==ROUTES=="; ip -j -o route show default 2>/dev/null
echo "==DNS=="; awk '/^nameserver/{print $2}' /etc/resolv.conf 2>/dev/null
echo "==BONDS=="; for d in /sys/class/net/*/bonding; do [ -d "$d" ] || continue; b=$(basename $(dirname "$d")); echo "$b mode=$(cat "$d/mode" 2>/dev/null) slaves=$(cat "$d/slaves" 2>/dev/null)"; done
echo "==OVS=="; ovs-vsctl list-br 2>/dev/null
echo "==LSMOD=="; lsmod 2>/dev/null | awk 'NR>1{print $1}'
echo "==END==""#
    }

    /// Capture a node's networking + drivers.
    pub async fn capture(&self, host: &str) -> Result<CaptureResult, AppError> {
        let out = self.ssh.run(host, Self::capture_command()).await?;
        Ok(parse_capture(&out))
    }
}

/// Parse the sectioned capture output into a `CaptureResult`. Pure + testable.
pub fn parse_capture(text: &str) -> CaptureResult {
    let mut net = NodeNetworkCapture::default();

    let section = |text: &str, name: &str| -> String {
        let marker = format!("=={name}==");
        let start = match text.find(&marker) {
            Some(i) => i + marker.len(),
            None => return String::new(),
        };
        let rest = &text[start..];
        // Stop at the next section marker (a line starting with "==").
        let stop = rest.find("\n==").map(|i| &rest[..i]).unwrap_or(rest);
        stop.trim().to_string()
    };

    // LINKS: `ip -j -o link show` provides per-interface name + MAC + MTU
    // (the addr show output lacks top-level ifname/mac/mtu). Build a lookup.
    let links_json = section(text, "LINKS");
    let mut link_info: std::collections::HashMap<String, (String, u32)> =
        std::collections::HashMap::new();
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&links_json) {
        for v in arr {
            let name = v.get("ifname").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            let mac = v.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let mtu = v.get("mtu").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            link_info.insert(name, (mac, mtu));
        }
    }

    // INTERFACES: `ip -j -o addr show`. Each entry's device name lives in
    // addr_info[].dev (modern iproute2) or addr_info[].label, or top-level
    // ifname (some fixtures). The first global-scope inet address is the
    // interface IP.
    let ifaces_json = section(text, "INTERFACES");
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&ifaces_json) {
        for v in arr {
            let info = v.get("addr_info").and_then(|x| x.as_array()).cloned().unwrap_or_default();
            // Resolve the interface name: addr_info[].dev -> addr_info[].label
            // -> top-level ifname.
            let name = info
                .iter()
                .find_map(|a| a.get("dev").and_then(|x| x.as_str()).filter(|s| !s.is_empty()))
                .or_else(|| {
                    info.iter().find_map(|a| a.get("label").and_then(|x| x.as_str()).filter(|s| !s.is_empty()))
                })
                .or_else(|| v.get("ifname").and_then(|x| x.as_str()))
                .unwrap_or("")
                .to_string();
            if name.is_empty() || name == "lo" {
                continue;
            }
            // First global inet address.
            let mut ip = String::new();
            let mut cidr = String::new();
            for a in &info {
                if a.get("family").and_then(|x| x.as_str()) == Some("inet") {
                    if a.get("scope").and_then(|x| x.as_str()) == Some("link") {
                        continue;
                    }
                    let local = a.get("local").and_then(|x| x.as_str()).unwrap_or("");
                    let pref = a.get("prefixlen").and_then(|x| x.as_u64()).unwrap_or(0);
                    if !local.is_empty() && pref > 0 {
                        ip = local.to_string();
                        cidr = pref.to_string();
                        break;
                    }
                }
            }
            // MAC + MTU: prefer the LINKS section (which always has them), fall
            // back to whatever top-level fields this entry carries.
            let (mac, mtu) = match link_info.get(&name) {
                Some((m, t)) => (m.clone(), *t),
                None => (
                    v.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    v.get("mtu").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
                ),
            };
            // VLANs are encoded in the name as <parent>.<id> (e.g. bond0.207).
            if let Some((parent, vid)) = parse_vlan_name(&name) {
                if !net.vlans.iter().any(|x| x.name == name) {
                    net.vlans.push(NodeNetworkVlan {
                        name,
                        id: vid,
                        parent,
                    });
                }
                continue; // don't list VLAN sub-interfaces as top-level interfaces
            }
            if !net.interfaces.iter().any(|i| i.name == name) {
                net.interfaces.push(NodeNetworkInterface {
                    name,
                    mtu,
                    ip,
                    cidr,
                    mac,
                });
            }
        }
    }

    // Bonds: "<name> mode=<mode> slaves=<s1 s2 ...>" (slaves to end of line).
    for line in section(text, "BONDS").lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let name = line.split_whitespace().next().unwrap_or("").to_string();
        let mode = line
            .find("mode=")
            .map(|m| line[m + 5..].split_whitespace().next().unwrap_or("").to_string())
            .unwrap_or_default();
        let slaves = line
            .find("slaves=")
            .map(|s| line[s + 7..].split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        if !name.is_empty() {
            net.bonds.push(NodeNetworkBond { name, mode, slaves });
        }
    }

    // Gateway from `ip -j -o route show default`.
    let routes_json = section(text, "ROUTES");
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&routes_json) {
        for r in arr.iter() {
            if let Some(gw) = r.get("gateway").and_then(|x| x.as_str()) {
                if !gw.is_empty() {
                    net.gateway = gw.to_string();
                    break;
                }
            }
        }
    }

    // DNS.
    net.dns = section(text, "DNS")
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // OVS bridges.
    net.ovs_bridges = section(text, "OVS")
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    // Active kernel modules (drivers).
    let drivers = section(text, "LSMOD")
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    CaptureResult { network: net, drivers }
}

/// Render the captured per-node networking as a Talos `machine.network` YAML
/// block. This is what gets embedded in the generated machine config so a
/// converted node keeps its bonds/VLANs/IPs/MTU/gateway/DNS. Pure + testable.
///
/// `hostname` is the node's k8s name (used as the Talos hostname). The block is
/// emitted at the `machine.network:` indent level (2-space under `machine:`).
pub fn render_node_network_yaml(c: &NodeNetworkCapture, hostname: &str) -> String {
    let mut y = String::new();
    y.push_str("    hostname: ");
    y.push_str(hostname);
    y.push('\n');
    y.push_str("    interfaces:\n");

    // If the node has a bond, the bond is the primary interface (with nested
    // VLANs); the slaves are listed under it.
    if let Some(bond) = c.bonds.first() {
        let mtu = bond_mtu(c, bond);
        y.push_str(&format!("      - interface: {}\n", bond.name));
        y.push_str(&format!("        mtu: {}\n", mtu));
        let mode = talos_bond_mode(&bond.mode);
        y.push_str("        bond:\n");
        y.push_str(&format!("          mode: {}\n", mode));
        y.push_str("          interfaces:\n");
        for s in &bond.slaves {
            y.push_str(&format!("            - {}\n", s));
        }
        // VLANs on the bond, or addresses/routes directly.
        let vlans_on_bond: Vec<&NodeNetworkVlan> =
            c.vlans.iter().filter(|v| v.parent == bond.name).collect();
        if !vlans_on_bond.is_empty() {
            y.push_str("        vlans:\n");
            for v in vlans_on_bond {
                y.push_str(&format!("          - vlanId: {}\n", v.id));
                if let Some(iface_ip) = address_for(c, &v.name) {
                    y.push_str("            addresses:\n");
                    y.push_str(&format!("              - {}/{}\n", iface_ip.0, iface_ip.1));
                    y.push_str(&format!("            mtu: {}\n", mtu));
                }
                if !c.gateway.is_empty() {
                    y.push_str("            routes:\n");
                    y.push_str("              - network: 0.0.0.0/0\n");
                    y.push_str(&format!("                gateway: {}\n", c.gateway));
                }
            }
        } else {
            if let Some((ip, cidr)) = address_for_bond(c, bond) {
                y.push_str("        addresses:\n");
                y.push_str(&format!("          - {}/{}\n", ip, cidr));
            }
            if !c.gateway.is_empty() {
                y.push_str("        routes:\n");
                y.push_str("          - network: 0.0.0.0/0\n");
                y.push_str(&format!("            gateway: {}\n", c.gateway));
            }
        }
        // Ignore unused NICs (those that aren't bond slaves).
        for i in &c.interfaces {
            if i.name.starts_with("eth") || i.name.starts_with("en") {
                if !bond.slaves.contains(&i.name.to_string()) {
                    y.push_str(&format!("      - interface: {}\n", i.name));
                    y.push_str("        ignore: true\n");
                }
            }
        }
    } else if let Some(first) = c.interfaces.first() {
        y.push_str(&format!("      - interface: {}\n", first.name));
        y.push_str(&format!("        mtu: {}\n", first.mtu));
        if !first.ip.is_empty() {
            y.push_str("        addresses:\n");
            y.push_str(&format!("          - {}{}\n", first.ip, if first.cidr.is_empty() { String::new() } else { format!("/{}", first.cidr) }));
        }
        if !c.gateway.is_empty() {
            y.push_str("        routes:\n");
            y.push_str("          - network: 0.0.0.0/0\n");
            y.push_str(&format!("            gateway: {}\n", c.gateway));
        }
    } else {
        y.push_str("      []\n");
    }

    if !c.dns.is_empty() {
        y.push_str("    nameservers:\n");
        for d in &c.dns {
            y.push_str(&format!("      - {}\n", d));
        }
    }
    y
}

fn talos_bond_mode(mode: &str) -> &'static str {
    // /sys/class/net/<bond>/bonding/mode is a number (0=rr,1=ab,4=lacp,5=tlb,6=alb).
    match mode.trim() {
        "0" | "balance-rr" => "balance-rr",
        "1" | "active-backup" => "active-backup",
        "4" | "802.3ad" | "lacp" => "802.3ad",
        "5" | "balance-tlb" => "balance-tlb",
        "6" | "balance-alb" => "balance-alb",
        _ => "802.3ad",
    }
}

fn bond_mtu(c: &NodeNetworkCapture, bond: &NodeNetworkBond) -> u32 {
    // Prefer a slave's MTU, else a default.
    c.interfaces
        .iter()
        .find(|i| bond.slaves.contains(&i.name.to_string()))
        .map(|i| i.mtu)
        .unwrap_or(1500)
}

fn address_for(c: &NodeNetworkCapture, name: &str) -> Option<(String, String)> {
    c.interfaces
        .iter()
        .find(|i| i.name == name)
        .filter(|i| !i.ip.is_empty())
        .map(|i| (i.ip.clone(), i.cidr.clone()))
}

fn address_for_bond(c: &NodeNetworkCapture, bond: &NodeNetworkBond) -> Option<(String, String)> {
    // A bond's address usually appears on a VLAN child; fall back to any
    // interface whose name starts with the bond name.
    for v in &c.vlans {
        if v.parent == bond.name {
            if let Some(a) = address_for(c, &v.name) {
                return Some(a);
            }
        }
    }
    c.interfaces
        .iter()
        .find(|i| i.name == bond.name && !i.ip.is_empty())
        .map(|i| (i.ip.clone(), i.cidr.clone()))
}

/// `<parent>.<id>` -> (parent, id) for a VLAN interface name.
fn parse_vlan_name(name: &str) -> Option<(String, u32)> {
    let idx = name.rfind('.')?;
    let parent = &name[..idx];
    let id: u32 = name[idx + 1..].parse().ok()?;
    if parent.is_empty() || id == 0 {
        return None;
    }
    Some((parent.to_string(), id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"==INTERFACES==
[{"ifindex":2,"ifname":"eth0","flags":["UP","BROADCAST","MULTICAST"],"mtu":9000,"qdisc":"noqueue","operstate":"up","linkmode":"DEFAULT","group_default":0,"link_type":"ether","address":"aa:bb:cc:dd:ee:01","broadcast":"ff:ff:ff:ff:ff:ff","addr_info":[{"family":"inet","local":"192.168.1.50","prefixlen":24,"scope":"host","label":"eth0"}]}]
==LINKS==
[]
==ROUTES==
[{"dst":"default","gateway":"192.168.1.1","dev":"bond0","flags":[],"prefsrc":"192.168.1.50"}]
==DNS==
10.0.0.2
10.0.0.3
==BONDS==
bond0 mode=802.3ad slaves=eth0 eth1
==OVS==
br-int
br-ex
==LSMOD==
bnx2x
bonding
openvswitch
==END==
"#;

    #[test]
    fn parses_interfaces_bonds_gateway_dns_ovs_drivers() {
        let r = parse_capture(SAMPLE);
        assert_eq!(r.network.interfaces.len(), 1);
        assert_eq!(r.network.interfaces[0].name, "eth0");
        assert_eq!(r.network.interfaces[0].mtu, 9000);
        assert_eq!(r.network.interfaces[0].ip, "192.168.1.50");
        assert_eq!(r.network.interfaces[0].cidr, "24");
        assert_eq!(r.network.bonds.len(), 1);
        assert_eq!(r.network.bonds[0].name, "bond0");
        assert_eq!(r.network.bonds[0].mode, "802.3ad");
        assert_eq!(r.network.bonds[0].slaves, vec!["eth0", "eth1"]);
        assert_eq!(r.network.gateway, "192.168.1.1");
        assert_eq!(r.network.dns, vec!["10.0.0.2", "10.0.0.3"]);
        assert_eq!(r.network.ovs_bridges, vec!["br-int", "br-ex"]);
        assert!(r.drivers.contains(&"bnx2x".to_string()));
        assert!(r.drivers.contains(&"bonding".to_string()));
    }

    #[test]
    fn vlan_name_is_captured_not_listed_as_interface() {
        let text = "==INTERFACES==\n[{\"ifname\":\"bond0.207\",\"mtu\":9000,\"address\":\"aa:bb:cc:dd:ee:02\",\"addr_info\":[{\"family\":\"inet\",\"local\":\"10.10.0.5\",\"prefixlen\":24}]}]\n==ROUTES==\n==DNS==\n==BONDS==\n==OVS==\n==LSMOD==\n==END==\n";
        let r = parse_capture(text);
        assert!(r.network.interfaces.is_empty(), "vlan sub-iface should not be a top-level interface");
        assert_eq!(r.network.vlans.len(), 1);
        assert_eq!(r.network.vlans[0].name, "bond0.207");
        assert_eq!(r.network.vlans[0].id, 207);
        assert_eq!(r.network.vlans[0].parent, "bond0");
    }

    #[test]
    fn empty_sections_are_tolerated() {
        let r = parse_capture("no sections here at all");
        assert!(r.network.interfaces.is_empty());
        assert!(r.drivers.is_empty());
        assert_eq!(r.network.gateway, "");
    }

    #[test]
    fn parses_real_addr_show_format_with_name_in_dev_and_mac_in_links() {
        // Regression: real `ip -j -o addr show` entries have NO top-level
        // `ifname` (device is `addr_info[].dev`) and no MAC/MTU (those come
        // from `ip -j -o link show`). The old parser keyed on `ifname` and
        // silently returned an empty interface list, dropping the bond IP.
        let text = "==INTERFACES==\n\
            [{\"addr_info\":[{\"dev\":\"lo\",\"family\":\"inet\",\"local\":\"127.0.0.1\",\"prefixlen\":8}]},\
            {\"addr_info\":[]},\
            {\"addr_info\":[{\"dev\":\"bond0\",\"family\":\"inet\",\"local\":\"172.20.0.38\",\"prefixlen\":22,\"scope\":\"global\"}]},\
            {\"addr_info\":[{\"dev\":\"bond0.326\",\"family\":\"inet6\",\"local\":\"fe80::1\",\"prefixlen\":64}]}]\n\
            ==LINKS==\n\
            [{\"ifname\":\"eno1\",\"mtu\":1500,\"address\":\"b8:ca:3a:6a:3c:20\"},\
            {\"ifname\":\"eno2\",\"mtu\":1500,\"address\":\"b8:ca:3a:6a:3c:20\"},\
            {\"ifname\":\"bond0\",\"mtu\":1500,\"address\":\"b8:ca:3a:6a:3c:20\"}]\n\
            ==ROUTES==\n\
            [{\"gateway\":\"172.20.0.1\"}]\n\
            ==DNS==\n127.0.0.53\n\
            ==BONDS==\nbond0 mode=802.3ad slaves=eno1 eno2\n\
            ==OVS==\n\
            ==LSMOD==\nigb\nbonding\n\
            ==END==\n";
        let r = parse_capture(text);
        // lo skipped, carrier-less skipped, bond0 kept, bond0.326 -> vlan.
        let bond = r.network.interfaces.iter().find(|i| i.name == "bond0").expect("bond0 interface present");
        assert_eq!(bond.ip, "172.20.0.38");
        assert_eq!(bond.cidr, "22");
        assert_eq!(bond.mtu, 1500, "MTU merged from LINKS section");
        assert_eq!(bond.mac, "b8:ca:3a:6a:3c:20", "MAC merged from LINKS section");
        assert_eq!(r.network.vlans.len(), 1);
        assert_eq!(r.network.vlans[0].name, "bond0.326");
        assert_eq!(r.network.gateway, "172.20.0.1");
        assert_eq!(r.network.dns, vec!["127.0.0.53"]);
        // The bond IP is resolvable for the kexec ip= param.
        let bondc = &r.network.bonds[0];
        let got = address_for_bond(&r.network, bondc);
        assert_eq!(got, Some(("172.20.0.38".to_string(), "22".to_string())));
    }

    #[test]
    fn renders_bonded_vlan_network_yaml() {
        let c = NodeNetworkCapture {
            interfaces: vec![],
            bonds: vec![NodeNetworkBond {
                name: "bond0".into(),
                mode: "4".into(),
                slaves: vec!["eth0".into(), "eth1".into()],
            }],
            vlans: vec![NodeNetworkVlan {
                name: "bond0.207".into(),
                id: 207,
                parent: "bond0".into(),
            }],
            gateway: "10.0.0.1".into(),
            dns: vec!["10.0.0.2".into()],
            ovs_bridges: vec!["br-int".into()],
        };
        let y = render_node_network_yaml(&c, "node1");
        assert!(y.contains("hostname: node1"));
        assert!(y.contains("- interface: bond0"));
        assert!(y.contains("mode: 802.3ad"));
        assert!(y.contains("- eth0"));
        assert!(y.contains("- vlanId: 207"));
        assert!(y.contains("gateway: 10.0.0.1"));
        assert!(y.contains("- 10.0.0.2"));
    }

    #[test]
    fn renders_single_interface_network_yaml() {
        let c = NodeNetworkCapture {
            interfaces: vec![NodeNetworkInterface {
                name: "eno1".into(),
                mtu: 9000,
                ip: "192.168.1.50".into(),
                cidr: "24".into(),
                mac: "aa:bb".into(),
            }],
            bonds: vec![],
            vlans: vec![],
            gateway: "192.168.1.1".into(),
            dns: vec![],
            ovs_bridges: vec![],
        };
        let y = render_node_network_yaml(&c, "worker1");
        assert!(y.contains("- interface: eno1"));
        assert!(y.contains("mtu: 9000"));
        assert!(y.contains("192.168.1.50/24"));
        assert!(y.contains("gateway: 192.168.1.1"));
    }
}
