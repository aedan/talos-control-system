use crate::db::models::machine::Machine;
use crate::db::pool::{DbPool, SqlVal};
use crate::AppError;

const COLS: &str = "id, system_uuid, muid, machine_type, cluster_id, status, talos_version, secure_boot, siderolink_connected, address, install_disk, desired_config, mac_address, hostname, bmc_address, bmc_username, bmc_password_enc, bmc_type, bmc_redfish_path, bmc_tls_insecure, pxe_profile_id, last_power_state, last_seen_at, created_at, updated_at, factory_modules, module_adds, module_removes";

pub async fn create(pool: &DbPool, machine: &Machine) -> Result<Machine, AppError> {
    if get(pool, machine.id).await?.is_some() {
        return Err(AppError::InvalidInput(format!(
            "Machine {} already exists",
            machine.id
        )));
    }
    let n = pool
        .execute(
            &format!(
                "INSERT INTO machines ({COLS})
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
            ),
            &[
                SqlVal::Uuid(machine.id),
                SqlVal::text(&machine.system_uuid),
                SqlVal::text(&machine.muid),
                SqlVal::text(&machine.machine_type),
                SqlVal::OptUuid(machine.cluster_id),
                SqlVal::text(&machine.status),
                SqlVal::text(&machine.talos_version),
                SqlVal::Bool(machine.secure_boot),
                SqlVal::Bool(machine.siderolink_connected),
                SqlVal::text(&machine.address),
                SqlVal::text(&machine.install_disk),
                SqlVal::OptText(machine.desired_config.clone()),
                SqlVal::text(&machine.mac_address),
                SqlVal::text(&machine.hostname),
                SqlVal::text(&machine.bmc_address),
                SqlVal::text(&machine.bmc_username),
                SqlVal::OptText(machine.bmc_password_enc.clone()),
                SqlVal::text(&machine.bmc_type),
                SqlVal::text(&machine.bmc_redfish_path),
                SqlVal::Bool(machine.bmc_tls_insecure),
                SqlVal::OptText(machine.pxe_profile_id.clone()),
                SqlVal::text(&machine.last_power_state),
                SqlVal::OptDateTime(machine.last_seen_at),
                SqlVal::DateTime(machine.created_at),
                SqlVal::DateTime(machine.updated_at),
                SqlVal::OptText(machine.factory_modules.clone()),
                SqlVal::OptText(machine.module_adds.clone()),
                SqlVal::OptText(machine.module_removes.clone()),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::Internal("Failed to create machine".into()));
    }
    Ok(machine.clone())
}

pub async fn get(pool: &DbPool, id: uuid::Uuid) -> Result<Option<Machine>, AppError> {
    pool.fetch_optional_as(
        &format!("SELECT {COLS} FROM machines WHERE id = ?"),
        &[SqlVal::Uuid(id)],
    )
    .await
}

pub async fn get_by_system_uuid(
    pool: &DbPool,
    system_uuid: &str,
) -> Result<Option<Machine>, AppError> {
    pool.fetch_optional_as(
        &format!("SELECT {COLS} FROM machines WHERE system_uuid = ?"),
        &[SqlVal::text(system_uuid)],
    )
    .await
}

pub async fn get_by_mac(pool: &DbPool, mac: &str) -> Result<Option<Machine>, AppError> {
    let normalized = normalize_mac(mac);
    pool.fetch_optional_as(
        &format!("SELECT {COLS} FROM machines WHERE mac_address = ? OR mac_address = ?"),
        &[SqlVal::text(&normalized), SqlVal::text(mac)],
    )
    .await
}

/// Set the siderolink_connected flag for the machine that matches the given
/// identifier. The SideroLink peer's `system_uuid` is the node's Talos **MUID**,
/// so we match `machines.muid` first; the `system_uuid` match is kept as a
/// fallback for any machine still identified by the old `mac-<MAC>` alias.
/// Returns true if a row was updated.
pub async fn set_siderolink_connected(
    pool: &DbPool,
    machine_id: &str,
    connected: bool,
) -> Result<bool, AppError> {
    let now = chrono::Utc::now();
    let n_muid = pool
        .execute(
            "UPDATE machines SET siderolink_connected = ?, updated_at = ? WHERE muid = ? AND muid != ''",
            &[
                SqlVal::Bool(connected),
                SqlVal::DateTime(now),
                SqlVal::text(machine_id),
            ],
        )
        .await?;
    if n_muid > 0 {
        return Ok(true);
    }
    let n_uuid = pool
        .execute(
            "UPDATE machines SET siderolink_connected = ?, updated_at = ? WHERE system_uuid = ?",
            &[
                SqlVal::Bool(connected),
                SqlVal::DateTime(now),
                SqlVal::text(machine_id),
            ],
        )
        .await?;
    Ok(n_uuid > 0)
}

/// Set the machine's Talos MUID (from `talosctl get systeminformation`).
/// Returns true if a row was updated.
pub async fn set_muid(
    pool: &DbPool,
    machine_id: uuid::Uuid,
    muid: &str,
) -> Result<bool, AppError> {
    let n = pool
        .execute(
            "UPDATE machines SET muid = ?, updated_at = ? WHERE id = ?",
            &[
                SqlVal::text(muid),
                SqlVal::DateTime(chrono::Utc::now()),
                SqlVal::Uuid(machine_id),
            ],
        )
        .await?;
    Ok(n > 0)
}


pub async fn list(pool: &DbPool) -> Result<Vec<Machine>, AppError> {
    pool.fetch_all_as(
        &format!("SELECT {COLS} FROM machines ORDER BY created_at DESC"),
        &[],
    )
    .await
}

pub async fn list_by_cluster(
    pool: &DbPool,
    cluster_id: uuid::Uuid,
) -> Result<Vec<Machine>, AppError> {
    pool.fetch_all_as(
        &format!("SELECT {COLS} FROM machines WHERE cluster_id = ? ORDER BY created_at DESC"),
        &[SqlVal::Uuid(cluster_id)],
    )
    .await
}

pub async fn list_with_mac(pool: &DbPool) -> Result<Vec<Machine>, AppError> {
    pool.fetch_all_as(
        &format!(
            "SELECT {COLS} FROM machines WHERE mac_address != '' AND mac_address IS NOT NULL ORDER BY created_at DESC"
        ),
        &[],
    )
    .await
}

pub async fn update(pool: &DbPool, machine: &Machine) -> Result<Machine, AppError> {
    let n = pool
        .execute(
            "UPDATE machines SET system_uuid = ?, muid = ?, machine_type = ?, cluster_id = ?, status = ?, talos_version = ?, secure_boot = ?, siderolink_connected = ?, address = ?, install_disk = ?, desired_config = ?, mac_address = ?, hostname = ?, bmc_address = ?, bmc_username = ?, bmc_password_enc = ?, bmc_type = ?, bmc_redfish_path = ?, bmc_tls_insecure = ?, pxe_profile_id = ?, last_power_state = ?, last_seen_at = ?, factory_modules = ?, module_adds = ?, module_removes = ?, updated_at = ?
              WHERE id = ?",
            &[
                SqlVal::text(&machine.system_uuid),
                SqlVal::text(&machine.muid),
                SqlVal::text(&machine.machine_type),
                SqlVal::OptUuid(machine.cluster_id),
                SqlVal::text(&machine.status),
                SqlVal::text(&machine.talos_version),
                SqlVal::Bool(machine.secure_boot),
                SqlVal::Bool(machine.siderolink_connected),
                SqlVal::text(&machine.address),
                SqlVal::text(&machine.install_disk),
                SqlVal::OptText(machine.desired_config.clone()),
                SqlVal::text(&machine.mac_address),
                SqlVal::text(&machine.hostname),
                SqlVal::text(&machine.bmc_address),
                SqlVal::text(&machine.bmc_username),
                SqlVal::OptText(machine.bmc_password_enc.clone()),
                SqlVal::text(&machine.bmc_type),
                SqlVal::text(&machine.bmc_redfish_path),
                SqlVal::Bool(machine.bmc_tls_insecure),
                SqlVal::OptText(machine.pxe_profile_id.clone()),
                SqlVal::text(&machine.last_power_state),
                SqlVal::OptDateTime(machine.last_seen_at),
                SqlVal::OptText(machine.factory_modules.clone()),
                SqlVal::OptText(machine.module_adds.clone()),
                SqlVal::OptText(machine.module_removes.clone()),
                SqlVal::DateTime(machine.updated_at),
                SqlVal::Uuid(machine.id),
            ],
        )
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", machine.id)));
    }
    Ok(machine.clone())
}

pub async fn delete(pool: &DbPool, id: uuid::Uuid) -> Result<(), AppError> {
    let n = pool
        .execute("DELETE FROM machines WHERE id = ?", &[SqlVal::Uuid(id)])
        .await?;
    if n == 0 {
        return Err(AppError::NotFound(format!("Machine {} not found", id)));
    }
    Ok(())
}

/// Normalize MAC to lowercase colon-separated form.
pub fn normalize_mac(mac: &str) -> String {
    let hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if hex.len() != 12 {
        return mac.trim().to_ascii_lowercase();
    }
    hex.as_bytes()
        .chunks(2)
        .map(|c| std::str::from_utf8(c).unwrap_or("00"))
        .collect::<Vec<_>>()
        .join(":")
}
