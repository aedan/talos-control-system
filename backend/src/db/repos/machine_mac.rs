use chrono::Utc;
use uuid::Uuid;

use crate::db::models::machine::Machine;
use crate::db::pool::{DbPool, SqlVal};
use crate::db::repos::machine::normalize_mac;
use crate::integration::bmc::{pick_primary_mac, NicInfo, NicKind};
use crate::AppError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MachineMac {
    pub mac: String,
    pub machine_id: String,
    pub kind: String,
    pub name: String,
}

pub async fn list_for_machine(pool: &DbPool, machine_id: Uuid) -> Result<Vec<MachineMac>, AppError> {
    pool.fetch_all_as(
        "SELECT mac, machine_id, kind, name FROM machine_macs WHERE machine_id = ? ORDER BY mac",
        &[SqlVal::text(&machine_id.to_string())],
    )
    .await
}

pub async fn extra_macs_for(pool: &DbPool, machine_id: Uuid) -> Result<Vec<String>, AppError> {
    Ok(list_for_machine(pool, machine_id)
        .await?
        .into_iter()
        .filter(|r| r.kind != "bmc")
        .map(|r| r.mac)
        .collect())
}

pub async fn machine_id_for_mac(pool: &DbPool, mac: &str) -> Result<Option<Uuid>, AppError> {
    let n = normalize_mac(mac);
    let row: Option<MachineMac> = pool
        .fetch_optional_as(
            "SELECT mac, machine_id, kind, name FROM machine_macs WHERE mac = ?",
            &[SqlVal::text(&n)],
        )
        .await?;
    Ok(row.and_then(|r| Uuid::parse_str(&r.machine_id).ok()))
}

/// Store discovered NICs. Sets `machine.mac_address` when it is empty and a
/// host NIC was found. Extra host MACs are stored so PXE/DHCP can match any
/// of them. Does not overwrite an operator-set primary MAC.
pub async fn apply_discovered_nics(
    pool: &DbPool,
    machine: &mut Machine,
    nics: &[NicInfo],
) -> Result<bool, AppError> {
    let mut wrote_primary = false;
    if machine.mac_address.trim().is_empty() {
        if let Some(primary) = pick_primary_mac(nics) {
            machine.mac_address = primary;
            machine.updated_at = Utc::now();
            crate::db::repos::machine::update(pool, machine).await?;
            wrote_primary = true;
        }
    }

    let id = machine.id.to_string();
    let now = Utc::now().to_rfc3339();
    for nic in nics {
        let mac = nic.mac.clone();
        if mac.is_empty() {
            continue;
        }
        // Do not steal a MAC already assigned as another machine's primary.
        if let Ok(Some(other)) = crate::db::repos::machine::get_by_mac_primary(pool, &mac).await {
            if other.id != machine.id {
                tracing::warn!(
                    mac = %mac,
                    owner = %other.id,
                    "skipping BMC MAC already owned by another machine"
                );
                continue;
            }
        }
        let kind = match nic.kind {
            NicKind::Host => "host",
            NicKind::Bmc => "bmc",
        };
        let _ = pool
            .execute("DELETE FROM machine_macs WHERE mac = ?", &[SqlVal::text(&mac)])
            .await;
        let _ = pool
            .execute(
                "INSERT INTO machine_macs (mac, machine_id, kind, name, created_at) VALUES (?, ?, ?, ?, ?)",
                &[
                    SqlVal::text(&mac),
                    SqlVal::text(&id),
                    SqlVal::text(kind),
                    SqlVal::text(&nic.name),
                    SqlVal::text(&now),
                ],
            )
            .await;
    }
    Ok(wrote_primary)
}
