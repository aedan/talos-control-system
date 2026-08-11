-- Desired (editable) Talos machine config YAML per inventory machine
ALTER TABLE machines ADD COLUMN desired_config TEXT;
