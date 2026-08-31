-- Node-level module overrides as deltas against the cluster default set.
--
-- `machines.factory_modules` (migration 016) remains the absolute override
-- mechanism for legacy rows and for the per-machine picker's "Apply modules"
-- action. The new columns implement the user-requested delta model:
--
--   effective_modules(machine) =
--     (cluster.factory_modules
--        - machine.module_removes
--        + machine.module_adds)
--     | machine.factory_modules   -- if set, wins outright (legacy absolute)
--
-- Both add/removes are stored as JSON arrays of official extension names
-- (e.g. `["siderolabs/bnx2-bnx2x"]`). Empty arrays (or NULL) mean "no delta".
-- "Reset to cluster defaults" clears all three (factory_modules + adds + removes).
ALTER TABLE machines ADD COLUMN module_adds TEXT;
ALTER TABLE machines ADD COLUMN module_removes TEXT;
