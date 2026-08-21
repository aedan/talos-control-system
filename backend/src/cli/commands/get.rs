//! `tcs get <kind> [name]` — list or fetch K8s objects (arbitrary kinds).
//!
//! The server returns raw K8s JSON (via the generic `/resource` endpoint). This
//! command renders it: `json`/`yaml` pass through; `table`/`wide` build a
//! client-side table from the standard object fields.

use super::client::Client;
use super::output::{render, Format};
use clap::Args;
use serde_json::Value;

#[derive(Args, Debug, Clone)]
pub struct GetArgs {
    /// Kind to get (e.g. pod, deployment, svc, node, or any CRD kind).
    pub kind: String,
    /// Object name (omit to list).
    #[arg(value_name = "NAME")]
    pub name: Option<String>,
    /// Namespace (defaults to all for namespaced kinds).
    #[arg(short = 'n', long = "namespace", alias = "ns")]
    pub namespace: Option<String>,
    /// List across all namespaces (default for list; kept for kubectl parity).
    #[arg(short = 'A', long = "all-namespaces")]
    pub all_namespaces: bool,
    /// Output format: table, wide, json, yaml.
    #[arg(short = 'o', long, default_value = "table")]
    pub output: String,
}

pub async fn run(client: &Client, cluster: &str, args: &GetArgs) -> super::super::client::CliResult<()> {
    let format = Format::parse(&args.output);
    let base = format!("/api/clusters/{cluster}/k8s");

    let raw = match &args.name {
        Some(name) => {
            let ns = args
                .namespace
                .as_deref()
                .map(|n| format!("&ns={n}"))
                .unwrap_or_default();
            client.get_json(&format!("{base}/resource/{name}?kind={}&{ns}", args.kind)).await?
        }
        None => {
            let ns = args
                .namespace
                .as_deref()
                .map(|n| format!("&ns={n}"))
                .unwrap_or_default();
            client.get_json(&format!("{base}/resource?kind={}&{ns}", args.kind)).await?
        }
    };

    let value = match format {
        Format::Json | Format::Yaml => raw,
        Format::Table | Format::Wide => to_table(&raw, format == Format::Wide),
    };
    println!("{}", render(&value, format));
    Ok(())
}

/// Build a `{"columns": [...], "rows": [[...]]}` value from a raw K8s list/get.
fn to_table(raw: &Value, wide: bool) -> Value {
    // K8s lists come back as typed objects (`NodeList`, `PodList`, …) or the
    // generic `List`; both carry an `items` array. Anything else is a single object.
    let is_list = raw
        .get("kind")
        .and_then(|k| k.as_str())
        .map(|k| k == "List" || k.ends_with("List"))
        .unwrap_or(false)
        || raw.get("items").map(|i| i.is_array()).unwrap_or(false);

    let items: Vec<&Value> = if is_list {
        raw.get("items")
            .and_then(|i| i.as_array())
            .map(|a| a.iter().collect())
            .unwrap_or_default()
    } else {
        vec![raw]
    };

    let mut columns = vec!["NAME".to_string()];
    if items.iter().any(|i| i.pointer("/metadata/namespace").and_then(|n| n.as_str()).is_some()) {
        columns.push("NAMESPACE".to_string());
    }
    if wide {
        columns.push("STATUS".to_string());
        columns.push("AGE".to_string());
    }

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|i| {
            let name = i.pointer("/metadata/name").and_then(|n| n.as_str()).unwrap_or("-");
            let ns = i.pointer("/metadata/namespace").and_then(|n| n.as_str()).unwrap_or("");
            let mut row = vec![name.to_string()];
            if columns.contains(&"NAMESPACE".to_string()) {
                row.push(ns.to_string());
            }
            if wide {
                row.push(status_str(i));
                row.push(age_str(i));
            }
            row
        })
        .collect();

    serde_json::json!({ "columns": columns, "rows": rows })
}

fn status_str(obj: &Value) -> String {
    // Pod phase
    if let Some(p) = obj.pointer("/status/phase").and_then(|p| p.as_str()) {
        return p.to_string();
    }
    // Deployment: ready/replicas
    if let (Some(r), Some(d)) = (
        obj.pointer("/status/readyReplicas").and_then(|v| v.as_u64()),
        obj.pointer("/spec/replicas").and_then(|v| v.as_u64()),
    ) {
        return format!("{r}/{d}");
    }
    // Node Ready
    if let Some(conds) = obj.pointer("/status/conditions").and_then(|c| c.as_array()) {
        if let Some(c) = conds.iter().find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Ready")) {
            return c.get("status").and_then(|s| s.as_str()).unwrap_or("-").to_string();
        }
    }
    "-".to_string()
}

fn age_str(obj: &Value) -> String {
    let created = obj.pointer("/metadata/creationTimestamp").and_then(|t| t.as_str());
    match created {
        Some(ts) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                let secs = (chrono::Utc::now() - dt.with_timezone(&chrono::Utc)).num_seconds().max(0);
                human_age(secs)
            } else {
                ts.to_string()
            }
        }
        None => "-".to_string(),
    }
}

fn human_age(secs: i64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names(v: &Value) -> Vec<String> {
        v.get("rows")
            .and_then(|r| r.as_array())
            .map(|r| {
                r.iter()
                    .filter_map(|row| row.get(0).and_then(|c| c.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn table_typed_list() {
        let raw = json!({"kind": "NodeList", "items": [
            {"metadata": {"name": "n1"}},
            {"metadata": {"name": "n2"}}
        ]});
        let v = to_table(&raw, false);
        assert_eq!(names(&v), vec!["n1", "n2"]);
    }

    #[test]
    fn table_generic_list() {
        let raw = json!({"kind": "List", "items": [
            {"metadata": {"name": "p1", "namespace": "default"}}
        ]});
        let v = to_table(&raw, false);
        assert_eq!(names(&v), vec!["p1"]);
        assert!(v["columns"].as_array().unwrap().iter().any(|c| c == "NAMESPACE"));
    }

    #[test]
    fn table_single_object() {
        let raw = json!({"kind": "Pod", "metadata": {"name": "solo", "namespace": "ns"}});
        let v = to_table(&raw, false);
        assert_eq!(names(&v), vec!["solo"]);
    }

    #[test]
    fn table_empty_list() {
        let raw = json!({"kind": "PodList", "items": []});
        let v = to_table(&raw, false);
        assert!(v["rows"].as_array().unwrap().is_empty());
    }
}
