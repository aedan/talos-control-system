use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::db::models::machine::Machine;
use crate::db::repos;
use crate::AppError;

pub struct MachineController {
    pool: SqlitePool,
}

impl MachineController {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn reconcile(&self) -> Result<(), AppError> {
        let machines = repos::machine::list(&self.pool).await?;

        for machine in machines {
            if self.reconcile_machine(&machine).await.is_err() {
                warn!(machine_id = %machine.id, "Failed to reconcile machine");
            }
        }

        Ok(())
    }

    async fn reconcile_machine(&self, machine: &Machine) -> Result<(), AppError> {
        let mut updated_machine = machine.clone();

        match machine.status.as_str() {
            "pending" => {
                if machine.siderolink_connected {
                    updated_machine.status = "booting".to_string();
                }
            },
            "booting" => {
                if !machine.talos_version.is_empty() {
                    updated_machine.status = "configuring".to_string();
                }
            },
            "configuring" => {
                if machine.talos_version != "0.0.0" {
                    updated_machine.status = "running".to_string();
                }
            },
            "installing" => {
                if !machine.talos_version.is_empty() && machine.talos_version != "0.0.0" {
                    updated_machine.status = "configuring".to_string();
                }
            },
            "destroying" => {
                if !machine.siderolink_connected {
                    if let Some(_cluster_id) = machine.cluster_id {
                        let _ = repos::machine::delete(&self.pool, machine.id).await;
                        return Ok(());
                    }
                }
            },
            "running" => {
                if !machine.siderolink_connected {
                    updated_machine.status = "pending".to_string();
                }
            },
            _ => {}
        }

        if updated_machine.status != machine.status {
            repos::machine::update(&self.pool, &updated_machine).await?;
            info!(
                machine_id = %machine.id,
                old_status = %machine.status,
                new_status = %updated_machine.status,
                "Machine status updated"
            );
        }

        Ok(())
    }

    pub async fn track_status(&self, machine_id: uuid::Uuid, new_status: &str, siderolink_connected: bool) -> Result<(), AppError> {
        if let Some(mut machine) = repos::machine::get(&self.pool, machine_id).await? {
            if machine.status != new_status || machine.siderolink_connected != siderolink_connected {
                machine.status = new_status.to_string();
                machine.siderolink_connected = siderolink_connected;
                machine.updated_at = chrono::Utc::now();

                repos::machine::update(&self.pool, &machine).await?;
            }
        }

        Ok(())
    }
}
