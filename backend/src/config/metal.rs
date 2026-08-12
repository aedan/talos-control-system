//! Bare-metal provisioning: DHCP, PXE, BMC defaults.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetalConfig {
    /// Master switch for metal features (BMC APIs always available when machines have BMC).
    pub enabled: bool,
    pub dhcp: MetalDhcpConfig,
    pub pxe: MetalPxeConfig,
    pub bmc: MetalBmcConfig,
}

impl Default for MetalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dhcp: MetalDhcpConfig::default(),
            pxe: MetalPxeConfig::default(),
            bmc: MetalBmcConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetalDhcpConfig {
    pub enabled: bool,
    /// Network interface to bind DHCP (required when enabled). Dedicated provision VLAN.
    pub interface: String,
    /// Optional bind IP; empty = interface primary address.
    pub bind_ip: String,
    pub subnet: String,
    pub range_start: String,
    pub range_end: String,
    pub gateway: String,
    pub dns: Vec<String>,
    pub lease_ttl_secs: u32,
    /// If false (default), only known machine MACs get leases.
    pub allow_unknown: bool,
}

impl Default for MetalDhcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface: String::new(),
            bind_ip: String::new(),
            subnet: "10.88.0.0/24".into(),
            range_start: "10.88.0.100".into(),
            range_end: "10.88.0.200".into(),
            gateway: "10.88.0.1".into(),
            dns: vec!["10.88.0.1".into()],
            lease_ttl_secs: 3600,
            allow_unknown: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetalPxeConfig {
    pub enabled: bool,
    pub http_port: u16,
    pub tftp_enabled: bool,
    pub asset_dir: String,
    pub default_talos_version: String,
    pub mirror_base: String,
    /// Extra kernel cmdline appended to metal boots.
    pub extra_cmdline: String,
    /// iPXE binary filename served over TFTP for legacy BIOS PXE clients.
    pub ipxe_bios_file: String,
    /// iPXE binary filename served over TFTP for UEFI PXE clients.
    pub ipxe_uefi_file: String,
}

impl Default for MetalPxeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            http_port: 6969,
            tftp_enabled: false,
            asset_dir: "/var/lib/tcs/pxe".into(),
            default_talos_version: "v1.13.7".into(),
            mirror_base: "https://github.com/siderolabs/talos/releases/download".into(),
            extra_cmdline: String::new(),
            ipxe_bios_file: "undionly.kpxe".into(),
            ipxe_uefi_file: "snponly.efi".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetalBmcConfig {
    pub connect_timeout_secs: u64,
    pub prefer_redfish: bool,
    pub ipmi_interface: String,
}

impl Default for MetalBmcConfig {
    fn default() -> Self {
        Self {
            connect_timeout_secs: 15,
            prefer_redfish: true,
            ipmi_interface: "lanplus".into(),
        }
    }
}
