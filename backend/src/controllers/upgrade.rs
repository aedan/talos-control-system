use chrono::Utc;
use crate::db::pool::DbPool;
use uuid::Uuid;

use crate::db::repos::upgrade_job::{self, UpgradeJob, UpgradeJobTarget};
use crate::db::repos::{self};
use crate::AppError;

pub struct UpgradeController {
    pool: DbPool,
}

impl UpgradeController {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn start_cluster_upgrade(
        &self,
        cluster_id: Uuid,
        image: &str,
        max_unavailable: i32,
        control_plane_last: bool,
        created_by: Option<String>,
    ) -> Result<UpgradeJob, AppError> {
        if image.trim().is_empty() {
            return Err(AppError::InvalidInput("image is required".into()));
        }
        let _ = repos::cluster::get(&self.pool, cluster_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Cluster {} not found", cluster_id)))?;

        let machines = repos::machine::list_by_cluster(&self.pool, cluster_id).await?;
        if machines.is_empty() {
            return Err(AppError::InvalidInput(
                "No machines in cluster to upgrade".into(),
            ));
        }

        let now = Utc::now();
        let job = UpgradeJob {
            id: Uuid::new_v4(),
            scope: "cluster".to_string(),
            image: image.trim().to_string(),
            status: "pending".to_string(),
            max_unavailable: max_unavailable.max(1),
            control_plane_last,
            cancel_requested: false,
            created_by,
            error: None,
            created_at: now,
            updated_at: now,
        };
        upgrade_job::create_job(&self.pool, &job).await?;
        insert_ordered_targets(&self.pool, job.id, cluster_id, machines, control_plane_last, 0)
            .await?;
        Ok(job)
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

async fn insert_ordered_targets(
    pool: &DbPool,
    job_id: Uuid,
    cluster_id: Uuid,
    machines: Vec<crate::db::models::machine::Machine>,
    control_plane_last: bool,
    start_sort: i32,
) -> Result<i32, AppError> {
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
    let mut sort = start_sort;
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
            updated_at: now,
        };
        sort += 1;
        upgrade_job::insert_target(pool, &t).await?;
    }
    Ok(sort)
}

fn is_cp(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    t == "control-plane" || t == "controlplane"
}

#[cfg(test)]
mod tests {
    use super::is_cp;

    #[test]
    fn detects_control_plane_types() {
        assert!(is_cp("controlplane"));
        assert!(is_cp("control-plane"));
        assert!(is_cp("ControlPlane"));
        assert!(!is_cp("worker"));
        assert!(!is_cp(""));
    }

    #[test]
    fn control_plane_last_sort_key() {
        // Workers before CP when control_plane_last=true
        let mut types = vec!["controlplane", "worker", "worker", "control-plane"];
        types.sort_by(|a, b| {
            let a_cp = is_cp(a);
            let b_cp = is_cp(b);
            match (true, a_cp, b_cp) {
                (true, true, false) => std::cmp::Ordering::Greater,
                (true, false, true) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Equal,
            }
        });
        assert_eq!(types[0], "worker");
        assert_eq!(types[1], "worker");
        assert!(is_cp(types[2]));
        assert!(is_cp(types[3]));
    }
}
