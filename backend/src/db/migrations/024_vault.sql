-- Ciphertext vault. The data key is not stored. vault_meta holds two wraps of
-- that key: one opened by the host key file, one opened by the break-glass
-- passphrase. vault_secrets holds named secrets encrypted with the data key.
-- There is no API to read these rows back out as plaintext.
CREATE TABLE IF NOT EXISTS vault_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    host_wrap TEXT NOT NULL,
    break_glass_wrap TEXT NOT NULL,
    break_glass_salt TEXT NOT NULL,
    break_glass_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS vault_secrets (
    name TEXT PRIMARY KEY,
    ciphertext TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
