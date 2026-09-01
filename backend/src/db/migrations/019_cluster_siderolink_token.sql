-- Per-cluster persistent Siderolink join token.
--
-- The global `siderolink_join_tokens` rows are one-time-use and (optionally)
-- expiring — they're for ad-hoc node registration. A *persistent* per-cluster
-- token lets TCS auto-bake a stable `siderolink.token` into the generated
-- machine configs of a greenfield cluster so every node it provisions can dial
-- in and form the WireGuard tunnel without a human pasting a fresh token.
--
-- One row per cluster. `token` is accepted by the register endpoint in
-- addition to the one-time join tokens. Rotating replaces the token value
-- (and requires re-issuing configs); revoking deletes the row.
CREATE TABLE IF NOT EXISTS cluster_siderolink_tokens (
    cluster_id TEXT PRIMARY KEY NOT NULL,
    token TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
