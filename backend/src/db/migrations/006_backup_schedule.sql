-- Migration 006: scheduled etcd backups per cluster

ALTER TABLE clusters ADD COLUMN backup_schedule_hours INTEGER;
ALTER TABLE clusters ADD COLUMN last_auto_backup_at TEXT;
