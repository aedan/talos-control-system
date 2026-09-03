-- Per-machine Talos MUID (machine ID).
--
-- Talos nodes report a stable MUID in their hardware system information
-- (talosctl get systeminformations.hardware.talos.dev -> spec.uuid). This is
-- the SAME identifier a node sends as `node_uuid` when it calls the SideroLink
-- Provision API, and it is stored as `siderolink_peers.system_uuid`.
--
-- `machines.system_uuid` is a TCS-invented `mac-<MAC>` alias, so it never
-- matches the SideroLink peer's MUID — which is why the `siderolink_connected`
-- flag was never set and the UI reported 0 nodes connected even with all
-- tunnels up. Storing the real MUID on the machine lets the SideroLink
-- peer correlate back to its machine.

ALTER TABLE machines ADD COLUMN muid TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_machines_muid ON machines(muid);
