-- Migration 004: Talos control-plane credentials and machine addresses

ALTER TABLE clusters ADD COLUMN talosconfig TEXT;

ALTER TABLE machines ADD COLUMN address TEXT NOT NULL DEFAULT '';
