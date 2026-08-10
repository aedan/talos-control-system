-- Full metal provisioning: BMC, MAC, PXE profiles, DHCP leases

ALTER TABLE machines ADD COLUMN mac_address TEXT NOT NULL DEFAULT '';
ALTER TABLE machines ADD COLUMN hostname TEXT NOT NULL DEFAULT '';
ALTER TABLE machines ADD COLUMN bmc_address TEXT NOT NULL DEFAULT '';
ALTER TABLE machines ADD COLUMN bmc_username TEXT NOT NULL DEFAULT '';
ALTER TABLE machines ADD COLUMN bmc_password_enc TEXT;
ALTER TABLE machines ADD COLUMN bmc_type TEXT NOT NULL DEFAULT 'auto';
ALTER TABLE machines ADD COLUMN bmc_redfish_path TEXT NOT NULL DEFAULT '';
ALTER TABLE machines ADD COLUMN bmc_tls_insecure INTEGER NOT NULL DEFAULT 1;
ALTER TABLE machines ADD COLUMN pxe_profile_id TEXT;
ALTER TABLE machines ADD COLUMN last_power_state TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE machines ADD COLUMN last_seen_at TEXT;

CREATE INDEX IF NOT EXISTS idx_machines_mac ON machines(mac_address);
CREATE INDEX IF NOT EXISTS idx_machines_bmc ON machines(bmc_address);

CREATE TABLE IF NOT EXISTS pxe_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    arch TEXT NOT NULL DEFAULT 'amd64',
    kernel_url TEXT NOT NULL DEFAULT '',
    initramfs_url TEXT NOT NULL DEFAULT '',
    cmdline TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    assets_ready INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS dhcp_leases (
    mac TEXT PRIMARY KEY NOT NULL,
    ip TEXT NOT NULL,
    hostname TEXT NOT NULL DEFAULT '',
    machine_id TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dhcp_leases_ip ON dhcp_leases(ip);
CREATE INDEX IF NOT EXISTS idx_dhcp_leases_machine ON dhcp_leases(machine_id);
