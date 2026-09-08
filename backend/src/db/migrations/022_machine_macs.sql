-- Extra NIC MACs discovered from the BMC (PXE may boot any host NIC).
CREATE TABLE IF NOT EXISTS machine_macs (
    mac TEXT PRIMARY KEY NOT NULL,
    machine_id TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'host',
    name TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_machine_macs_machine ON machine_macs(machine_id);
