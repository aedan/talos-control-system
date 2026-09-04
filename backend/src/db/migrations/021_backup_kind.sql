-- Distinguish the kind of backup stored in cluster_backups.
--
-- `etcd` (default) is a Kubernetes etcd snapshot taken via
-- `talosctl etcd snapshot create` (disaster recovery for the K8s control
-- plane data). `db` is a consistent backup of TCS's own database — the
-- source of truth for clusters, machines, inventory, config patches,
-- SideroLink peers/tokens, users, etc. — so the whole TCS deployment can be
-- restored without re-importing inventory or re-establishing tunnels.
--
-- Both kinds share the same list / download / delete / retention machinery;
-- `kind` only tags the row and drives the download file extension.

ALTER TABLE cluster_backups ADD COLUMN kind TEXT NOT NULL DEFAULT 'etcd';
