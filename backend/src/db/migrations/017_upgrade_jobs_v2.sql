-- Rolling upgrade jobs v2: per-target derived images + optional in-place
-- Kubernetes upgrade phase.
--
-- `upgrade_jobs.image` stays NOT NULL for backward compatibility: when only the
-- k8s phase runs, it holds the *current* installer image (informational). The
-- authoritative per-node image for the Talos phase lives on each target row.
--
-- Phase lifecycle: 'talos' -> 'k8s' -> done. A job with no Talos change starts
-- directly at 'k8s'; a job with no k8s target finishes the 'talos' phase.
ALTER TABLE upgrade_jobs ADD COLUMN target_talos_version TEXT;
ALTER TABLE upgrade_jobs ADD COLUMN target_k8s_version TEXT;
ALTER TABLE upgrade_jobs ADD COLUMN phase TEXT NOT NULL DEFAULT 'talos';
ALTER TABLE upgrade_jobs ADD COLUMN steps TEXT;

ALTER TABLE upgrade_job_targets ADD COLUMN image TEXT;
ALTER TABLE upgrade_job_targets ADD COLUMN k8s_version TEXT;
ALTER TABLE upgrade_job_targets ADD COLUMN phase TEXT NOT NULL DEFAULT 'talos';
ALTER TABLE upgrade_job_targets ADD COLUMN completed_steps TEXT;
