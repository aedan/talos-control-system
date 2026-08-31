use chrono::Utc;
use uuid::Uuid;

use crate::db::pool::DbPool;

use crate::db::repos::upgrade_job::{self, UpgradeJob, UpgradeJobTarget};
use crate::db::repos::{self};
use crate::integration::image_factory::ImageFactoryClient;
use crate::integration::talosctl::{
    cmp_k8s_versions, latest_k8s_patch_for_minor, parse_k8s_version,
};
use crate::AppError;

pub struct UpgradeController {
    pool: DbPool,
}

impl UpgradeController {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Queue a cluster-wide rolling upgrade.
    ///
    /// Phases (in order):
    ///   1. `talos` — when `talos_version` or the cluster module set changes:
    ///      each node rolls to the Image Factory installer for its *effective*
    ///      modules (machine override, else cluster default) at the target
    ///      version. Nodes reboot.
    ///   2. `k8s` — when `k8s_version` is given: an in-place, no-reboot
    ///      `talosctl upgrade-k8s` from the current version to the target.
    ///
    /// Returns the created job plus the resolved k8s steps (for the UI to show
    /// the sequential plan when >1 step).
    pub async fn start_cluster_upgrade(
        &self,
        cluster_id: Uuid,
        talos_version: Option<&str>,
        k8s_version: Option<&str>,
        modules: Option<Vec<String>>,
        max_unavailable: i32,
        control_plane_last: bool,
        factory: &crate::config::FactoryConfig,
        created_by: Option<String>,
    ) -> Result<(UpgradeJob, Vec<String>), AppError> {
        let cluster = repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        // ── module handling ───────────────────────────────────────────────
        // When the caller supplies a module set that differs from the stored
        // cluster default, persist it first (same semantics as the UI's
        // "Save cluster modules"). Node-level overrides are untouched.
        let stored_modules: Vec<String> = cluster
            .factory_modules
            .clone()
            .filter(|s| !s.trim().is_empty())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let (cluster_modules, modules_changed) = match &modules {
            Some(m) => {
                let cleaned: Vec<String> = m
                    .iter()
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect();
                let changed = sorted_key(&cleaned) != sorted_key(&stored_modules);
                if changed {
                    let ctrl = crate::controllers::cluster::ClusterController::new(self.pool.clone());
                    ctrl.set_cluster_factory_modules(cluster_id, Some(cleaned.clone()), factory)
                        .await?;
                }
                (cleaned, changed)
            }
            None => (stored_modules.clone(), false),
        };

        // ── phase determination ───────────────────────────────────────────
        let target_talos = talos_version
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cluster.talos_version.clone());

        let talos_changed = norm_v(&target_talos) != norm_v(&cluster.talos_version);

        let current_k8s = cluster.control_plane_version.clone();
        let target_k8s = k8s_version
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let k8s_steps: Vec<String> = match &target_k8s {
            Some(t) => build_k8s_ladder(&current_k8s, &t).await?,
            None => Vec::new(),
        };
        let k8s_changed = !k8s_steps.is_empty();

        if !talos_changed && !modules_changed && !k8s_changed {
            return Err(AppError::InvalidInput(
                "No change requested: same Talos version, same modules, same k8s version".into(),
            ));
        }

        // ── per-target image resolution (talos phase) ─────────────────────
        let machines = repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        if machines.is_empty() {
            return Err(AppError::InvalidInput(
                "No machines in cluster to upgrade".into(),
            ));
        }

        let factory_client = ImageFactoryClient::new(&factory.normalized_base());
        let mut images: std::collections::HashMap<Uuid, String> = std::collections::HashMap::new();
        if talos_changed || modules_changed {
            let cluster_schematic = factory_client
                .create_schematic(&cluster_modules)
                .await
                .map_err(|e| AppError::Network(format!("Failed to resolve cluster module schematic: {e}")))?;
            let cluster_sorted = cluster_modules_sorted(&cluster_modules);
            for m in &machines {
                let eff = effective_modules_json(&m.factory_modules, &cluster_modules);
                let schematic = if sorted_key(&eff) == cluster_sorted {
                    cluster_schematic.clone()
                } else {
                    factory_client
                        .create_schematic(&eff)
                        .await
                        .map_err(|e| {
                            AppError::Network(format!(
                                "Failed to resolve schematic for machine {}: {e}",
                                m.system_uuid
                            ))
                        })?
                };
                images.insert(m.id, factory.installer_image(&schematic, &target_talos));
            }
        }

        // Job-wide `image` column: the cluster-default image for the target
        // version (informational; per-target images are authoritative).
        let job_image = if talos_changed || modules_changed {
            images[&machines[0].id].clone()
        } else {
            // k8s-only job: keep the cluster's current installer ref if known.
            format!("ghcr.io/siderolabs/installer:{}", norm_v(&cluster.talos_version))
        };

        let now = Utc::now();
        let phase = if talos_changed || modules_changed {
            "talos".to_string()
        } else {
            "k8s".to_string()
        };
        let job = UpgradeJob {
            id: Uuid::new_v4(),
            scope: "cluster".to_string(),
            image: job_image,
            status: "pending".to_string(),
            max_unavailable: max_unavailable.max(1),
            control_plane_last,
            cancel_requested: false,
            created_by,
            error: None,
            target_talos_version: Some(target_talos.clone()),
            target_k8s_version: target_k8s.clone(),
            phase,
            steps: if k8s_changed {
                Some(serde_json::to_string(&k8s_steps).unwrap_or_default())
            } else {
                None
            },
            created_at: now,
            updated_at: now,
        };
        upgrade_job::create_job(&self.pool, &job).await?;
        insert_ordered_targets(
            &self.pool,
            job.id,
            cluster_id,
            machines,
            control_plane_last,
            &images,
            &target_talos,
            &k8s_steps,
        )
        .await?;
        Ok((job, k8s_steps))
    }

    pub async fn get_job_detail(&self, job_id: Uuid) -> Result<serde_json::Value, AppError> {
        let job = upgrade_job::get_job(&self.pool, job_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Upgrade job {} not found", job_id)))?;
        let targets = upgrade_job::list_targets(&self.pool, job_id).await?;
        Ok(serde_json::json!({
            "job": job,
            "targets": targets,
            "summary": {
                "total": targets.len(),
                "pending": targets.iter().filter(|t| t.status == "pending").count(),
                "running": targets.iter().filter(|t| t.status == "running").count(),
                "completed": targets.iter().filter(|t| t.status == "completed").count(),
                "failed": targets.iter().filter(|t| t.status == "failed").count(),
            }
        }))
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn sorted_key(v: &[String]) -> Vec<String> {
    let mut s: Vec<String> = v.to_vec();
    s.sort();
    s.dedup();
    s
}

fn cluster_modules_sorted(m: &[String]) -> Vec<String> {
    sorted_key(m)
}

/// Effective module list for a machine: its own absolute override when set,
/// else the cluster default. (Absolute model — a per-machine list *is* the
/// end state; "reset to cluster" clears the override to null.)
fn effective_modules_json(
    machine_override: &Option<String>,
    cluster_default: &[String],
) -> Vec<String> {
    match machine_override {
        Some(raw) if !raw.trim().is_empty() => {
            serde_json::from_str::<Vec<String>>(raw)
                .unwrap_or_else(|_| cluster_default.to_vec())
        }
        _ => cluster_default.to_vec(),
    }
}

/// Normalize a version string for comparison: "1.13.7" == "v1.13.7".
fn norm_v(v: &str) -> String {
    let t = v.trim();
    if t.starts_with('v') {
        t.to_string()
    } else {
        format!("v{t}")
    }
}

/// Build the sequential k8s ladder from `current` to `target`:
///   * reject downgrades / same-version,
///   * never jump more than one minor in a single step (Talos `upgrade-k8s`
///     enforces this), but DO support multi-minor upgrades by stepping one
///     minor at a time: the job's `steps` list contains one entry per hop and
///     the scheduler runs them in order.
/// Each hop targets the latest patch release of the next minor (the version
/// that actually ships in a Talos build), ending at the exact requested target.
async fn build_k8s_ladder(current: &str, target: &str) -> Result<Vec<String>, AppError> {
    let cur = parse_k8s_version(current)
        .ok_or_else(|| AppError::InvalidInput(format!("unparseable current k8s version: {current}")))?;
    let tgt = parse_k8s_version(target)
        .ok_or_else(|| AppError::InvalidInput(format!("unparseable target k8s version: {target}")))?;
    if cur.0 != tgt.0 {
        return Err(AppError::InvalidInput("Only Kubernetes major 1 is supported".into()));
    }
    match cmp_k8s_versions(current, target) {
        Some(std::cmp::Ordering::Less) => {}
        Some(std::cmp::Ordering::Equal) => {
            return Err(AppError::InvalidInput(
                "Target k8s version equals the current version".into(),
            ))
        }
        Some(std::cmp::Ordering::Greater) => {
            return Err(AppError::InvalidInput(format!(
                "Kubernetes downgrade from {current} to {target} is not permitted"
            )))
        }
        None => {
            return Err(AppError::InvalidInput(
                "Could not compare k8s versions".into(),
            ))
        }
    }
    let mut steps: Vec<String> = Vec::new();
    let mut minor = cur.1;
    while minor < tgt.1 {
        let next = minor + 1;
        if next == tgt.1 {
            // Final hop: land exactly on the requested target.
            steps.push(norm_v(target));
        } else {
            // Intermediate hop: step through the latest patch of the next minor.
            // Fall back to v1.{next}.0 if the release lookup is unavailable —
            // `upgrade-k8s` will still work as long as that exact version ships
            // in the Talos build (the scheduler's dry-run probe catches it).
            match latest_k8s_patch_for_minor(next).await {
                Some(v) => steps.push(v),
                None => steps.push(format!("v1.{next}.0")),
            }
        }
        minor = next;
    }
    // Same-minor patch bump: single step to the exact target.
    if steps.is_empty() {
        steps.push(norm_v(target));
    }
    Ok(steps)
}

async fn insert_ordered_targets(
    pool: &DbPool,
    job_id: Uuid,
    cluster_id: Uuid,
    machines: Vec<crate::db::models::machine::Machine>,
    control_plane_last: bool,
    images: &std::collections::HashMap<Uuid, String>,
    target_talos: &str,
    k8s_steps: &[String],
) -> Result<(), AppError> {
    let mut ordered = machines;
    ordered.sort_by(|a, b| {
        let a_cp = is_cp(&a.machine_type);
        let b_cp = is_cp(&b.machine_type);
        match (control_plane_last, a_cp, b_cp) {
            (true, true, false) => std::cmp::Ordering::Greater,
            (true, false, true) => std::cmp::Ordering::Less,
            (false, true, false) => std::cmp::Ordering::Less,
            (false, false, true) => std::cmp::Ordering::Greater,
            _ => a.system_uuid.cmp(&b.system_uuid),
        }
    });
    let now = Utc::now();
    let mut sort = 0;
    for m in ordered {
        let t = UpgradeJobTarget {
            id: Uuid::new_v4(),
            job_id,
            cluster_id,
            machine_id: m.id,
            address: if m.address.is_empty() {
                None
            } else {
                Some(m.address.clone())
            },
            machine_type: Some(m.machine_type.clone()),
            status: "pending".to_string(),
            error: None,
            sort_order: sort,
            image: images.get(&m.id).cloned(),
            k8s_version: None,
            phase: if images.contains_key(&m.id) {
                "talos".to_string()
            } else {
                "k8s".to_string()
            },
            completed_steps: None,
            updated_at: now,
        };
        sort += 1;
        upgrade_job::insert_target(pool, &t).await?;
    }
    let _ = (target_talos, k8s_steps);
    Ok(())
}

fn is_cp(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    t == "control-plane" || t == "controlplane"
}

#[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn detects_control_plane_types() {
            assert!(is_cp("controlplane"));
            assert!(is_cp("control-plane"));
            assert!(is_cp("ControlPlane"));
            assert!(!is_cp("worker"));
            assert!(!is_cp(""));
        }

        // The multi-step ladder (jump >1 minor) resolves intermediate versions
        // from the GitHub releases API, so it is exercised on the lab cluster
        // rather than in unit tests. Single-hop cases below cover the logic.
        #[tokio::test]
        async fn ladder_rejects_downgrade() {
            assert!(build_k8s_ladder("v1.36.3", "v1.35.8").await.is_err());
        }

        #[tokio::test]
        async fn ladder_rejects_same() {
            assert!(build_k8s_ladder("v1.36.3", "v1.36.3").await.is_err());
        }

        #[tokio::test]
        async fn ladder_same_minor_patch() {
            let l = build_k8s_ladder("v1.36.3", "v1.36.4").await.unwrap();
            assert_eq!(l, vec!["v1.36.4"]);
        }

        #[tokio::test]
        async fn ladder_next_minor() {
            let l = build_k8s_ladder("v1.36.3", "v1.37.0").await.unwrap();
            assert_eq!(l, vec!["v1.37.0"]);
        }

        #[tokio::test]
        async fn ladder_accepts_without_v_prefix() {
            let l = build_k8s_ladder("1.36.3", "1.36.4").await.unwrap();
            assert_eq!(l, vec!["v1.36.4"]);
        }

        #[tokio::test]
        async fn ladder_multi_minor_steps_sequentially() {
            // v1.36.3 -> v1.38.0 must produce two steps: a v1.37.x hop, then v1.38.0.
            let l = build_k8s_ladder("v1.36.3", "v1.38.0").await.unwrap();
            assert_eq!(l.len(), 2);
            assert!(
                l[0].starts_with("v1.37."),
                "first hop should be v1.37.x, got {}",
                l[0]
            );
            assert_eq!(l[1], "v1.38.0");
        }

    #[test]
    fn effective_modules_override_wins() {
        let cluster = vec!["a".to_string(), "b".to_string()];
        let machine = Some(serde_json::to_string(&vec!["c".to_string()]).unwrap());
        assert_eq!(effective_modules_json(&machine, &cluster), vec!["c"]);
    }

    #[test]
    fn effective_modules_falls_back_to_cluster() {
        let cluster = vec!["a".to_string(), "b".to_string()];
        assert_eq!(effective_modules_json(&None, &cluster), vec!["a", "b"]);
        let empty = Some("".to_string());
        assert_eq!(effective_modules_json(&empty, &cluster), vec!["a", "b"]);
    }
}
