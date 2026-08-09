-- Migration 003: Machine classes table

CREATE TABLE IF NOT EXISTS machine_classes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    min_cpu INTEGER NOT NULL DEFAULT 1,
    min_memory INTEGER NOT NULL DEFAULT 0,
    min_disk INTEGER NOT NULL DEFAULT 0,
    arch TEXT NOT NULL DEFAULT 'x86_64',
    secure_boot INTEGER NOT NULL DEFAULT 0,
    allowed_roles TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_machine_classes_name ON machine_classes(name);
CREATE INDEX IF NOT EXISTS idx_machine_classes_arch ON machine_classes(arch);
