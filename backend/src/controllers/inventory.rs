//! Bulk machine inventory import (YAML / CSV).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::models::machine::Machine;
use crate::db::pool::DbPool;
use crate::db::repos::{self, machine::normalize_mac};
use crate::utils::secrets;
use crate::AppError;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryMachine {
    pub hostname: Option<String>,
    pub role: Option<String>,
    #[serde(alias = "machineType")]
    pub machine_type: Option<String>,
    pub mac: Option<String>,
    #[serde(alias = "macAddress")]
    pub mac_address: Option<String>,
    pub address: Option<String>,
    #[serde(alias = "installDisk")]
    pub install_disk: Option<String>,
    pub system_uuid: Option<String>,
    pub bmc: Option<InventoryBmc>,
    // flat CSV-style fields
    pub bmc_address: Option<String>,
    pub bmc_username: Option<String>,
    pub bmc_password: Option<String>,
    pub bmc_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryBmc {
    pub address: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryDocument {
    pub cluster: Option<InventoryCluster>,
    pub machines: Vec<InventoryMachine>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCluster {
    pub name: Option<String>,
    pub endpoint: Option<String>,
    pub talos_version: Option<String>,
    pub kubernetes_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryRowError {
    pub index: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryPreview {
    pub machines: Vec<InventoryMachineView>,
    pub errors: Vec<InventoryRowError>,
    pub cluster_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryMachineView {
    pub index: usize,
    pub hostname: String,
    pub role: String,
    pub mac: String,
    pub address: String,
    pub install_disk: String,
    pub bmc_address: String,
    pub has_bmc_password: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryImportResult {
    pub created: usize,
    pub updated: usize,
    pub errors: Vec<InventoryRowError>,
    pub machine_ids: Vec<Uuid>,
    pub cluster_id: Option<Uuid>,
}

pub fn parse_inventory(format: &str, content: &str) -> Result<InventoryDocument, AppError> {
    let format = format.trim().to_ascii_lowercase();
    match format.as_str() {
        "yaml" | "yml" => parse_yaml(content),
        "csv" => parse_csv(content),
        _ => Err(AppError::InvalidInput(
            "format must be yaml or csv".into(),
        )),
    }
}

fn parse_yaml(content: &str) -> Result<InventoryDocument, AppError> {
    // Accept either full document or bare machine list
    if let Ok(doc) = serde_yaml::from_str::<InventoryDocument>(content) {
        if !doc.machines.is_empty() {
            return Ok(doc);
        }
    }
    if let Ok(machines) = serde_yaml::from_str::<Vec<InventoryMachine>>(content) {
        return Ok(InventoryDocument {
            cluster: None,
            machines,
        });
    }
    serde_yaml::from_str(content).map_err(|e| AppError::InvalidInput(format!("YAML parse: {e}")))
}

fn parse_csv(content: &str) -> Result<InventoryDocument, AppError> {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| AppError::InvalidInput("CSV is empty".into()))?;
    let cols: Vec<String> = split_csv_line(header)
        .into_iter()
        .map(|c| c.trim().to_ascii_lowercase().replace('-', "_"))
        .collect();
    let mut machines = Vec::new();
    for line in lines {
        let cells = split_csv_line(line);
        let get = |name: &str| -> Option<String> {
            cols.iter()
                .position(|c| c == name)
                .and_then(|i| cells.get(i).cloned())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        machines.push(InventoryMachine {
            hostname: get("hostname"),
            role: get("role").or_else(|| get("machine_type")).or_else(|| get("machinetype")),
            machine_type: get("machine_type").or_else(|| get("machinetype")),
            mac: get("mac").or_else(|| get("mac_address")).or_else(|| get("macaddress")),
            mac_address: get("mac_address").or_else(|| get("macaddress")),
            address: get("address").or_else(|| get("ip")),
            install_disk: get("install_disk").or_else(|| get("installdisk")),
            system_uuid: get("system_uuid").or_else(|| get("systemuuid")),
            bmc: None,
            bmc_address: get("bmc_address").or_else(|| get("bmcaddress")),
            bmc_username: get("bmc_username").or_else(|| get("bmcusername")),
            bmc_password: get("bmc_password").or_else(|| get("bmcpassword")),
            bmc_type: get("bmc_type").or_else(|| get("bmctype")),
        });
    }
    Ok(InventoryDocument {
        cluster: None,
        machines,
    })
}

fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for ch in line.chars() {
        match ch {
            '"' => in_q = !in_q,
            ',' if !in_q => {
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn resolve_role(m: &InventoryMachine) -> String {
    let r = m
        .role
        .as_deref()
        .or(m.machine_type.as_deref())
        .unwrap_or("worker")
        .trim()
        .to_ascii_lowercase();
    match r.as_str() {
        "cp" | "controlplane" | "control-plane" | "control_plane" => "controlplane".into(),
        "worker" | "node" => "worker".into(),
        other => other.to_string(),
    }
}

fn resolve_mac(m: &InventoryMachine) -> String {
    let raw = m
        .mac
        .as_deref()
        .or(m.mac_address.as_deref())
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        String::new()
    } else {
        normalize_mac(raw)
    }
}

fn resolve_bmc(m: &InventoryMachine) -> (String, String, Option<String>, String) {
    if let Some(b) = &m.bmc {
        (
            b.address.clone().unwrap_or_default(),
            b.username.clone().unwrap_or_default(),
            b.password.clone().filter(|p| !p.is_empty()),
            b.r#type.clone().unwrap_or_else(|| "auto".into()),
        )
    } else {
        (
            m.bmc_address.clone().unwrap_or_default(),
            m.bmc_username.clone().unwrap_or_default(),
            m.bmc_password.clone().filter(|p| !p.is_empty()),
            m.bmc_type.clone().unwrap_or_else(|| "auto".into()),
        )
    }
}

pub fn preview_inventory(doc: &InventoryDocument) -> InventoryPreview {
    let mut errors = Vec::new();
    let mut views = Vec::new();
    let mut seen_mac = std::collections::HashSet::new();

    for (index, m) in doc.machines.iter().enumerate() {
        let mac = resolve_mac(m);
        let role = resolve_role(m);
        let hostname = m.hostname.clone().unwrap_or_default();
        let (bmc_address, _, bmc_pw, _) = resolve_bmc(m);

        if mac.is_empty() && m.address.as_ref().map(|a| a.is_empty()).unwrap_or(true) {
            errors.push(InventoryRowError {
                index,
                message: "row needs mac and/or address".into(),
            });
        }
        if !mac.is_empty() {
            if !seen_mac.insert(mac.clone()) {
                errors.push(InventoryRowError {
                    index,
                    message: format!("duplicate MAC {mac} in file"),
                });
            }
            let hex: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() != 12 {
                errors.push(InventoryRowError {
                    index,
                    message: format!("invalid MAC {mac}"),
                });
            }
        }

        views.push(InventoryMachineView {
            index,
            hostname,
            role,
            mac,
            address: m.address.clone().unwrap_or_default(),
            install_disk: m.install_disk.clone().unwrap_or_default(),
            bmc_address,
            has_bmc_password: bmc_pw.is_some(),
        });
    }

    InventoryPreview {
        machines: views,
        errors,
        cluster_name: doc
            .cluster
            .as_ref()
            .and_then(|c| c.name.clone()),
    }
}

pub async fn apply_inventory(
    pool: &DbPool,
    jwt_secret: &str,
    doc: &InventoryDocument,
    cluster_id: Option<Uuid>,
    upsert_by_mac: bool,
    create_cluster_name: Option<&str>,
    proxy_id: Option<&str>,
) -> Result<InventoryImportResult, AppError> {
    let preview = preview_inventory(doc);
    if preview.errors.iter().any(|e| e.message.contains("invalid") || e.message.contains("duplicate")) {
        // still allow apply with soft errors except invalid MAC
    }

    let mut cluster_id = cluster_id;
    if cluster_id.is_none() {
        if let Some(name) = create_cluster_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                doc.cluster
                    .as_ref()
                    .and_then(|c| c.name.as_deref())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
        {
            let mut c = crate::db::models::cluster::Cluster::new(
                name.to_string(),
                doc.cluster
                    .as_ref()
                    .and_then(|x| x.kubernetes_version.clone())
                    .unwrap_or_else(|| "v1.36.3".into()),
                doc.cluster
                    .as_ref()
                    .and_then(|x| x.talos_version.clone())
                    .unwrap_or_else(|| "v1.13.7".into()),
            );
            c.status = "pending".into();
            let created = repos::cluster::create(pool, &c).await?;
            cluster_id = Some(created.id);
        }
    }

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut errors = Vec::new();
    let mut machine_ids = Vec::new();
    let proxy_id = proxy_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    for (index, row) in doc.machines.iter().enumerate() {
        let mac = resolve_mac(row);
        let role = resolve_role(row);
        let hostname = row.hostname.clone().unwrap_or_default();
        let address = row.address.clone().unwrap_or_default();
        let install_disk = row.install_disk.clone().unwrap_or_default();
        let (bmc_address, bmc_username, bmc_password, bmc_type) = resolve_bmc(row);

        if mac.is_empty() && address.is_empty() {
            errors.push(InventoryRowError {
                index,
                message: "skipped: needs mac and/or address".into(),
            });
            continue;
        }

        let existing = if upsert_by_mac && !mac.is_empty() {
            repos::machine::get_by_mac(pool, &mac).await?
        } else {
            None
        };

        if let Some(mut m) = existing {
            m.hostname = hostname;
            m.machine_type = role;
            if !address.is_empty() {
                m.address = address;
            }
            if !install_disk.is_empty() {
                m.install_disk = install_disk;
            }
            m.mac_address = mac;
            m.bmc_address = bmc_address;
            m.bmc_username = bmc_username;
            m.bmc_type = bmc_type;
            if let Some(cid) = cluster_id {
                m.cluster_id = Some(cid);
            }
            if let Some(pw) = bmc_password {
                m.bmc_password_enc = Some(secrets::encrypt(jwt_secret, &pw)?);
            }
            if let Some(pid) = &proxy_id {
                m.proxy_id = Some(pid.clone());
            }
            m.updated_at = chrono::Utc::now();
            let m = repos::machine::update(pool, &m).await?;
            machine_ids.push(m.id);
            updated += 1;
        } else {
            let system_uuid = row
                .system_uuid
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    if !mac.is_empty() {
                        format!("mac-{mac}")
                    } else {
                        format!("baremetal-{}", Uuid::new_v4())
                    }
                });
            let mut m = Machine::new(system_uuid, role);
            m.hostname = hostname;
            m.address = address;
            m.install_disk = install_disk;
            m.mac_address = mac;
            m.bmc_address = bmc_address;
            m.bmc_username = bmc_username;
            m.bmc_type = bmc_type;
            m.cluster_id = cluster_id;
            m.proxy_id = proxy_id.clone();
            if let Some(pw) = bmc_password {
                m.bmc_password_enc = Some(secrets::encrypt(jwt_secret, &pw)?);
            }
            match repos::machine::create(pool, &m).await {
                Ok(m) => {
                    machine_ids.push(m.id);
                    created += 1;
                }
                Err(e) => {
                    // MAC-based system_uuid collision — try unique
                    if e.to_string().contains("already exists") || e.to_string().contains("UNIQUE") {
                        m.system_uuid = format!("baremetal-{}", Uuid::new_v4());
                        m.id = Uuid::new_v4();
                        match repos::machine::create(pool, &m).await {
                            Ok(m) => {
                                machine_ids.push(m.id);
                                created += 1;
                            }
                            Err(e2) => errors.push(InventoryRowError {
                                index,
                                message: e2.to_string(),
                            }),
                        }
                    } else {
                        errors.push(InventoryRowError {
                            index,
                            message: e.to_string(),
                        });
                    }
                }
            }
        }
    }

    Ok(InventoryImportResult {
        created,
        updated,
        errors,
        machine_ids,
        cluster_id,
    })
}
