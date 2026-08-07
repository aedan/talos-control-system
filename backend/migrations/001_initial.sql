-- Talos Control System Database Migrations
-- Migration 001: Initial schema

CREATE TABLE IF NOT EXISTS clusters (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    control_plane_version TEXT NOT NULL,
    talos_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',
    control_plane_size INTEGER NOT NULL DEFAULT 1,
    worker_size INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_clusters_status ON clusters(status);
CREATE INDEX IF NOT EXISTS idx_clusters_name ON clusters(name);

CREATE TABLE IF NOT EXISTS machines (
    id TEXT PRIMARY KEY,
    system_uuid TEXT NOT NULL UNIQUE,
    machine_type TEXT NOT NULL DEFAULT 'worker',
    cluster_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    talos_version TEXT NOT NULL DEFAULT '',
    secure_boot INTEGER NOT NULL DEFAULT 0,
    siderolink_connected INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (cluster_id) REFERENCES clusters(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_machines_cluster_id ON machines(cluster_id);
CREATE INDEX IF NOT EXISTS idx_machines_status ON machines(status);
CREATE INDEX IF NOT EXISTS idx_machines_system_uuid ON machines(system_uuid);

CREATE TABLE IF NOT EXISTS machine_sets (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    name TEXT NOT NULL,
    role TEXT NOT NULL,
    size INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (cluster_id) REFERENCES clusters(id) ON DELETE CASCADE,
    UNIQUE(cluster_id, name)
);

CREATE INDEX IF NOT EXISTS idx_machine_sets_cluster_id ON machine_sets(cluster_id);

CREATE TABLE IF NOT EXISTS cluster_machines (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    machine_id TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (cluster_id) REFERENCES clusters(id) ON DELETE CASCADE,
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE CASCADE,
    UNIQUE(cluster_id, machine_id)
);

CREATE INDEX IF NOT EXISTS idx_cluster_machines_cluster_id ON cluster_machines(cluster_id);
CREATE INDEX IF NOT EXISTS idx_cluster_machines_machine_id ON cluster_machines(machine_id);

CREATE TABLE IF NOT EXISTS config_patches (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    machine_id TEXT,
    path TEXT NOT NULL,
    value TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (cluster_id) REFERENCES clusters(id) ON DELETE CASCADE,
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_config_patches_cluster_id ON config_patches(cluster_id);
CREATE INDEX IF NOT EXISTS idx_config_patches_path ON config_patches(path);

CREATE TABLE IF NOT EXISTS machine_configs (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    config_hash TEXT NOT NULL UNIQUE,
    config_data TEXT NOT NULL,
    version TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (machine_id) REFERENCES machines(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_machine_configs_machine_id ON machine_configs(machine_id);
CREATE INDEX IF NOT EXISTS idx_machine_configs_hash ON machine_configs(config_hash);

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'reader',
    is_active INTEGER NOT NULL DEFAULT 1,
    last_login TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_role ON users(role);

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
    logo_data BLOB,
    favicon_data BLOB,
    docs_url TEXT,
    support_url TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tenant_branding_tenant_id ON tenant_branding(tenant_id);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    action TEXT NOT NULL,
    details TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_id ON audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource ON audit_logs(resource_type, resource_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at);
