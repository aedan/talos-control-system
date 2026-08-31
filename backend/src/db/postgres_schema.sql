
CREATE TABLE IF NOT EXISTS _tcs_migrations (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS clusters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    control_plane_version TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',
    control_plane_size INTEGER NOT NULL DEFAULT 1,
    worker_size INTEGER NOT NULL DEFAULT 1,
    talosconfig TEXT,
    kubeconfig TEXT,
    backup_retention INTEGER NOT NULL DEFAULT 10,
    backup_schedule_hours INTEGER,
    last_auto_backup_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    system_uuid TEXT NOT NULL UNIQUE,
    machine_type TEXT NOT NULL DEFAULT 'worker',
    cluster_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    talos_version TEXT NOT NULL DEFAULT '',
    secure_boot INTEGER NOT NULL DEFAULT 0,
    siderolink_connected INTEGER NOT NULL DEFAULT 0,
    address TEXT NOT NULL DEFAULT '',
    install_disk TEXT NOT NULL DEFAULT '',
    mac_address TEXT NOT NULL DEFAULT '',
    hostname TEXT NOT NULL DEFAULT '',
    bmc_address TEXT NOT NULL DEFAULT '',
    bmc_username TEXT NOT NULL DEFAULT '',
    bmc_password_enc TEXT,
    bmc_type TEXT NOT NULL DEFAULT 'auto',
    bmc_redfish_path TEXT NOT NULL DEFAULT '',
    bmc_tls_insecure INTEGER NOT NULL DEFAULT 1,
    pxe_profile_id TEXT,
    last_power_state TEXT NOT NULL DEFAULT 'unknown',
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

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

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'reader',
    is_active INTEGER NOT NULL DEFAULT 1,
    password_hash TEXT,
    auth_provider TEXT NOT NULL DEFAULT 'local',
    ldap_dn TEXT,
    password_needs_change INTEGER NOT NULL DEFAULT 0,
    last_login TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tenant_branding (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL UNIQUE,
    name TEXT,
    short_name TEXT,
    tagline TEXT,
    primary_color TEXT,
    secondary_color TEXT,
    background_color TEXT,
    surface_color TEXT,
    text_color TEXT,
    text_muted_color TEXT,
    font_family TEXT,
    logo_data BYTEA,
    favicon_data BYTEA,
    docs_url TEXT,
    support_url TEXT,
    created_at TEXT,
    updated_at TEXT
);

CREATE TABLE IF NOT EXISTS config_patches (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    machine_id TEXT,
    path TEXT NOT NULL,
    value TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cluster_backups (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    file_path TEXT,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    resource_type TEXT,
    resource_id TEXT,
    action TEXT NOT NULL,
    details TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cluster_access (
    user_id TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (user_id, cluster_id)
);

CREATE TABLE IF NOT EXISTS upgrade_jobs (
    id TEXT PRIMARY KEY,
    scope TEXT NOT NULL,
    image TEXT NOT NULL,
    status TEXT NOT NULL,
    max_unavailable INTEGER NOT NULL DEFAULT 1,
    control_plane_last INTEGER NOT NULL DEFAULT 1,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    created_by TEXT,
    error TEXT,
    target_talos_version TEXT,
    target_k8s_version TEXT,
    phase TEXT NOT NULL DEFAULT 'talos',
    steps TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS upgrade_job_targets (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    cluster_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    address TEXT,
    machine_type TEXT,
    status TEXT NOT NULL,
    error TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    image TEXT,
    k8s_version TEXT,
    phase TEXT NOT NULL DEFAULT 'talos',
    completed_steps TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS siderolink_peers (
    id TEXT PRIMARY KEY,
    system_uuid TEXT NOT NULL,
    public_key TEXT NOT NULL,
    assigned_ip TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS siderolink_join_tokens (
    token TEXT PRIMARY KEY,
    label TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provision_artifacts (
    id TEXT PRIMARY KEY,
    cluster_id TEXT,
    name TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    kubernetes_version TEXT NOT NULL,
    secrets_enc TEXT,
    controlplane_config TEXT,
    worker_config TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ha_locks (
    lock_name TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS oidc_states (
    state TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provision_jobs (
    id TEXT PRIMARY KEY,
    cluster_id TEXT,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    desired_workers INTEGER NOT NULL DEFAULT 0,
    payload TEXT,
    error TEXT,
    created_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
