-- Talos Image Factory: chosen system extensions (modules) per cluster, with a
-- per-machine override. Stored as a JSON array of official extension names,
-- e.g. ["siderolabs/bnx2-bnx2x"]. A machine's effective modules are its own
-- factory_modules if set, otherwise the cluster's.
ALTER TABLE clusters ADD COLUMN factory_modules TEXT;
ALTER TABLE machines ADD COLUMN factory_modules TEXT;
