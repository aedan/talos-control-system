-- Store network config (JSON) on clusters for auto-generation during metal provisioning
ALTER TABLE clusters ADD COLUMN network_config TEXT;
