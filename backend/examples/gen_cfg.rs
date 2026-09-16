use talos_control_system::integration::network_capture::render_node_network_yaml;

fn main() {
    let node = talos_control_system::integration::network_capture::parse_capture(
        r#"==INTERFACES==
[{"ifindex":1,"ifname":"lo","flags":["LOOPBACK","UP","LOWER_UP"],"mtu":65536,"operstate":"UNKNOWN","linkmode":0,"group":1926220992,"txqueuelen":1000,"qdisc":"noqueue","kind":"loopback","address":"00:00:00:00:00:00","broadcast":"00:00:00:00:00:00","linkaddress":"00:00:00:00:00:00"}]
==LINKS==
[{"ifindex":1,"ifname":"lo"}]
==ROUTES==
[{"dst":"","gateway":"172.20.0.1","dev":"bond0","flags":0,"metric":0}]
==DNS==
172.20.0.126
==BONDS==
bond0 mode=802.3ad slaves=eno49 eno50
==OVS==
==LSMOD==
bnx2x
iscsi_tcp
==END=="#);
    let yaml = render_node_network_yaml(&node.network, "605091-worker01");
    let mut cfg = String::new();
    cfg.push_str("version: v1alpha1\n");
    cfg.push_str("persist: true\n");
    cfg.push_str("machine:\n");
    cfg.push_str("  type: worker\n");
    cfg.push_str("  network:\n");
    cfg.push_str(&yaml);
    let ca = "-----BEGIN CERTIFICATE-----\nMIIBxx\n-----END CERTIFICATE-----";
    let ca_b64 = "TUlJQnh4";
    cfg.push_str("  acceptedCAs:\n");
    cfg.push_str(&format!("    crt: {ca_b64}\n"));
    cfg.push_str("  token: aabbcc.1111112222223333\n");
    cfg.push_str("  install:\n");
    cfg.push_str("    disk: /dev/sda\n");
    cfg.push_str("    wipe: true\n");
    cfg.push_str("    image: ghcr.io/siderolabs/installer:v1.13.10\n");
    cfg.push_str("cluster:\n");
    cfg.push_str("  id: Y2x1c3Rlci1pZA==\n");
    cfg.push_str("  secret: Y2x1c3Rlci1zZWNyZXQ=\n");
    cfg.push_str("  controlPlane:\n");
    cfg.push_str("    endpoint: https://172.20.0.39:6443\n");
    cfg.push_str("  clusterName: phobos\n");
    cfg.push_str("  token: ddeeff.4444445555556666\n");
    cfg.push_str("  ca:\n");
    cfg.push_str(&format!("    crt: {ca_b64}\n"));
    cfg.push_str("\n");
    for (i, line) in cfg.lines().enumerate() {
        println!("{:3}: {}", i + 1, line);
    }
}
