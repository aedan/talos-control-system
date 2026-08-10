-- Ensure ha_locks exists (009 could skip CREATE when a leading -- comment
-- was attached to the first statement before the splitter fix).
CREATE TABLE IF NOT EXISTS ha_locks (
    lock_name TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
