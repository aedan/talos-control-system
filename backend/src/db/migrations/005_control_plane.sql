-- Migration 005: kubeconfig storage + backup retention metadata helpers

ALTER TABLE clusters ADD COLUMN kubeconfig TEXT;

-- Optional retention override per cluster (NULL = use global default)
ALTER TABLE clusters ADD COLUMN backup_retention INTEGER;
