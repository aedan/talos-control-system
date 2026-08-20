//! Pure data mappers for the K8s explorer / CLI proxy.
//!
//! These turn `k8s-openapi` typed objects (or raw `serde_json::Value` for
//! arbitrary kinds) into compact, camelCase summary structs that the REST
//! handlers serialize directly. Keeping the mapping here (not in handlers)
//! means the list/stream/action handlers and the CLI share one contract.

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Event, Namespace, Node, Pod, Service};
use serde::Serialize;

/// Human-readable age from a creation timestamp, e.g. "2d4h", "3h12m", "45s".
pub fn age(created: Option<DateTime<Utc>>) -> String {
    let Some(ts) = created else {
        return "unknown".to_string();
    };
    let secs = (Utc::now() - ts).num_seconds().max(0);
    let (d, h, m, s) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m}m")
    } else if m > 0 {
        format!("{m}m{s}s")
    } else {
        format!("{s}s")
    }
}

fn ts(v: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time) -> DateTime<Utc> {
    v.0
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceSummary {
    pub name: String,
    pub status: String,
    pub age: String,
}

pub fn namespace_summary(ns: &Namespace) -> NamespaceSummary {
    NamespaceSummary {
        name: ns.metadata.name.clone().unwrap_or_default(),
        status: ns.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_default(),
        age: age(ns.metadata.creation_timestamp.as_ref().map(|t| ts(t))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodSummary {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub ready: String,
    pub restarts: i64,
    pub node: String,
    pub ip: String,
    pub age: String,
    pub containers: Vec<String>,
}

pub fn pod_summary(pod: &Pod) -> PodSummary {
    let status = pod.status.clone().unwrap_or_default();
    let ready = status
        .container_statuses
        .as_ref()
        .map(|cs| {
            let ready = cs.iter().filter(|c| c.ready).count();
            format!("{}/{}", ready, cs.len())
        })
        .unwrap_or_default();
    let restarts = status
        .container_statuses
        .as_ref()
        .map(|cs| cs.iter().map(|c| c.restart_count as i64).sum::<i64>())
        .unwrap_or(0);
    PodSummary {
        name: pod.metadata.name.clone().unwrap_or_default(),
        namespace: pod.metadata.namespace.clone().unwrap_or_default(),
        phase: status.phase.clone().unwrap_or_default(),
        ready,
        restarts,
        node: pod.spec.as_ref().and_then(|s| s.node_name.clone()).unwrap_or_default(),
        ip: status.pod_ip.clone().unwrap_or_default(),
        age: age(pod.metadata.creation_timestamp.as_ref().map(|t| ts(t))),
        containers: pod
            .spec
            .as_ref()
            .map(|s| s.containers.iter().map(|c| c.name.clone()).collect())
            .unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentSummary {
    pub name: String,
    pub namespace: String,
    pub ready: String,
    pub replicas: i32,
    pub available: i32,
    pub age: String,
}

pub fn deployment_summary(dep: &Deployment) -> DeploymentSummary {
    let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
    let status = dep.status.clone().unwrap_or_default();
    let available = status.available_replicas.unwrap_or(0);
    DeploymentSummary {
        name: dep.metadata.name.clone().unwrap_or_default(),
        namespace: dep.metadata.namespace.clone().unwrap_or_default(),
        ready: format!("{}/{}", available, desired),
        replicas: desired,
        available,
        age: age(dep.metadata.creation_timestamp.as_ref().map(|t| ts(t))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSummary {
    pub name: String,
    pub namespace: String,
    pub kind: String,
    pub cluster_ip: String,
    pub ports: String,
    pub age: String,
}

pub fn service_summary(svc: &Service) -> ServiceSummary {
    let kind = svc
        .spec
        .as_ref()
        .and_then(|s| s.type_.clone())
        .unwrap_or_else(|| "ClusterIP".to_string());
    let cluster_ip = svc.spec.as_ref().and_then(|s| s.cluster_ip.clone()).unwrap_or_default();
    let ports = svc
        .spec
        .as_ref()
        .and_then(|s| s.ports.clone())
        .map(|ps| {
            ps.iter()
                .map(|p| {
                    let proto = p.protocol.clone().unwrap_or_default();
                    match p.node_port {
                        Some(np) => format!("{proto}/{}:{}", p.port, np),
                        None => format!("{proto}/{}", p.port),
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();
    ServiceSummary {
        name: svc.metadata.name.clone().unwrap_or_default(),
        namespace: svc.metadata.namespace.clone().unwrap_or_default(),
        kind,
        cluster_ip,
        ports,
        age: age(svc.metadata.creation_timestamp.as_ref().map(|t| ts(t))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSummary {
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub reason: String,
    pub message: String,
    pub object: String,
    pub count: i32,
    pub last_seen: String,
}

pub fn event_summary(ev: &Event) -> EventSummary {
    let object = format!(
        "{}/{}",
        ev.involved_object.kind.clone().unwrap_or_default(),
        ev.involved_object.name.clone().unwrap_or_default()
    );
    let last_seen = ev
        .last_timestamp
        .as_ref()
        .map(|t| ts(t).format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .or_else(|| ev.event_time.as_ref().map(|t| t.0.format("%Y-%m-%dT%H:%M:%SZ").to_string()))
        .unwrap_or_default();
    EventSummary {
        namespace: ev.metadata.namespace.clone().unwrap_or_default(),
        name: ev.metadata.name.clone().unwrap_or_default(),
        kind: "Event".to_string(),
        reason: ev.reason.clone().unwrap_or_default(),
        message: ev.message.clone().unwrap_or_default(),
        object,
        count: ev.count.unwrap_or(1),
        last_seen,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    pub name: String,
    pub status: String,
    pub role: String,
    pub kubernetes_version: String,
    pub os_image: String,
    pub internal_ip: String,
    pub age: String,
}

pub fn node_summary(node: &Node) -> NodeSummary {
    let status = node.status.clone().unwrap_or_default();
    let labels = node.metadata.labels.clone().unwrap_or_default();
    let role = if labels.contains_key("node-role.kubernetes.io/control-plane")
        || labels.contains_key("node-role.kubernetes.io/master")
    {
        "control-plane"
    } else if labels.contains_key("node-role.kubernetes.io/worker") {
        "worker"
    } else {
        "unknown"
    };
    let ready = status
        .conditions
        .as_ref()
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| if c.status == "True" { "Ready".to_string() } else { "NotReady".to_string() })
        .unwrap_or_else(|| "Unknown".to_string());
    let internal_ip = status
        .addresses
        .as_ref()
        .and_then(|addrs| addrs.iter().find(|a| a.type_ == "InternalIP").map(|a| a.address.clone()))
        .unwrap_or_default();
    NodeSummary {
        name: node.metadata.name.clone().unwrap_or_default(),
        status: ready,
        role: role.to_string(),
        kubernetes_version: status.node_info.as_ref().map(|i| i.kubelet_version.clone()).unwrap_or_default(),
        os_image: status.node_info.as_ref().map(|i| i.os_image.clone()).unwrap_or_default(),
        internal_ip,
        age: age(node.metadata.creation_timestamp.as_ref().map(|t| ts(t))),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDetail {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub restarts: i64,
    pub state: String,
    pub started_at: String,
    pub last_state: String,
    /// The container spec's `command` (overrides the image entrypoint). Empty
    /// means the image's own entrypoint runs.
    pub command: Vec<String>,
    /// The container spec's `args` (passed to the command/entrypoint).
    pub args: Vec<String>,
}

fn state_str(state: &k8s_openapi::api::core::v1::ContainerState) -> String {
    if state.waiting.is_some() {
        format!("waiting: {}", state.waiting.as_ref().and_then(|w| w.reason.clone()).unwrap_or_default())
    } else if state.terminated.is_some() {
        format!("exited({})", state.terminated.as_ref().map(|t| t.exit_code).unwrap_or(-1))
    } else if state.running.is_some() {
        "running".to_string()
    } else {
        "unknown".to_string()
    }
}

pub fn container_detail(
    cs: &k8s_openapi::api::core::v1::ContainerStatus,
    spec: Option<&k8s_openapi::api::core::v1::Container>,
) -> ContainerDetail {
    ContainerDetail {
        name: cs.name.clone(),
        image: cs.image.clone(),
        ready: cs.ready,
        restarts: cs.restart_count as i64,
        state: cs.state.as_ref().map(state_str).unwrap_or_default(),
        started_at: cs
            .state
            .as_ref()
            .and_then(|s| s.running.as_ref())
            .and_then(|r| r.started_at.as_ref())
            .map(|t| ts(t).format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_default(),
        last_state: cs.last_state.as_ref().map(state_str).unwrap_or_default(),
        command: spec.and_then(|c| c.command.clone()).unwrap_or_default(),
        args: spec.and_then(|c| c.args.clone()).unwrap_or_default(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodDetail {
    pub summary: PodSummary,
    pub containers: Vec<ContainerDetail>,
    pub labels: std::collections::BTreeMap<String, String>,
    pub yaml: String,
}

pub fn pod_detail(pod: &Pod) -> PodDetail {
    let spec_containers = pod.spec.as_ref().map(|s| s.containers.clone()).unwrap_or_default();
    let containers = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.clone())
        .map(|cs| {
            cs.iter()
                .map(|c| {
                    let spec = spec_containers.iter().find(|sc| sc.name == c.name);
                    container_detail(c, spec)
                })
                .collect()
        })
        .unwrap_or_default();
    let labels = pod.metadata.labels.clone().unwrap_or_default();
    let yaml = serde_yaml::to_string(pod).unwrap_or_default();
    PodDetail {
        summary: pod_summary(pod),
        containers,
        labels,
        yaml,
    }
}

/// A kind resolved through API discovery (for arbitrary-kind reads/apply).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedKind {
    pub group: String,
    pub version: String,
    pub api_version: String,
    pub kind: String,
    pub plural: String,
    pub namespaced: bool,
}

impl From<&kube::discovery::ApiResource> for ResolvedKind {
    fn from(r: &kube::discovery::ApiResource) -> Self {
        ResolvedKind {
            group: r.group.clone(),
            version: r.version.clone(),
            api_version: r.api_version.clone(),
            kind: r.kind.clone(),
            plural: r.plural.clone(),
            namespaced: false,
        }
    }
}

/// Result of applying one YAML document.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub status: String,
}

/// Result of draining a node (evicting its non-DaemonSet pods).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrainResult {
    pub node: String,
    pub evicted: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateRunning, ContainerStatus, NodeAddress,
        NodeCondition, NodeStatus, NodeSystemInfo, ObjectReference, PodSpec, PodStatus,
        ServicePort, ServiceSpec,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use std::collections::BTreeMap;

    #[test]
    fn test_age_none() {
        assert_eq!(age(None), "unknown");
    }

    #[test]
    fn test_age_days_hours() {
        let ts = Utc::now() - Duration::hours(52);
        assert_eq!(age(Some(ts)), "2d4h");
    }

    #[test]
    fn test_age_hours_minutes() {
        let ts = Utc::now() - Duration::hours(3) - Duration::minutes(12);
        assert_eq!(age(Some(ts)), "3h12m");
    }

    #[test]
    fn test_age_seconds() {
        let ts = Utc::now() - Duration::seconds(45);
        let out = age(Some(ts));
        assert!(out.starts_with('4') && out.ends_with('s'), "unexpected age: {out}");
    }

    #[test]
    fn test_namespace_summary() {
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some("kube-system".into()),
                creation_timestamp: Some(Time(Utc::now() - Duration::hours(52))),
                ..Default::default()
            },
            status: Some(k8s_openapi::api::core::v1::NamespaceStatus {
                phase: Some("Active".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = namespace_summary(&ns);
        assert_eq!(s.name, "kube-system");
        assert_eq!(s.status, "Active");
        assert_eq!(s.age, "2d4h");
    }

    fn sample_pod() -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some("p1".into()),
                namespace: Some("ns1".into()),
                creation_timestamp: Some(Time(Utc::now() - Duration::hours(1))),
                ..Default::default()
            },
            spec: Some(PodSpec {
                node_name: Some("node-a".into()),
                containers: vec![Container {
                    name: "app".into(),
                    image: Some("img:1".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            status: Some(PodStatus {
                phase: Some("Running".into()),
                pod_ip: Some("10.0.0.5".into()),
                container_statuses: Some(vec![ContainerStatus {
                    name: "app".into(),
                    ready: true,
                    restart_count: 2,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_pod_summary() {
        let pod = sample_pod();
        let s = pod_summary(&pod);
        assert_eq!(s.name, "p1");
        assert_eq!(s.namespace, "ns1");
        assert_eq!(s.phase, "Running");
        assert_eq!(s.ready, "1/1");
        assert_eq!(s.restarts, 2);
        assert_eq!(s.node, "node-a");
        assert_eq!(s.ip, "10.0.0.5");
        assert_eq!(s.containers, vec!["app".to_string()]);
    }

    #[test]
    fn test_deployment_summary() {
        let dep = Deployment {
            metadata: ObjectMeta {
                name: Some("d1".into()),
                namespace: Some("ns1".into()),
                ..Default::default()
            },
            spec: Some(k8s_openapi::api::apps::v1::DeploymentSpec {
                replicas: Some(3),
                ..Default::default()
            }),
            status: Some(k8s_openapi::api::apps::v1::DeploymentStatus {
                available_replicas: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = deployment_summary(&dep);
        assert_eq!(s.name, "d1");
        assert_eq!(s.namespace, "ns1");
        assert_eq!(s.ready, "2/3");
        assert_eq!(s.replicas, 3);
        assert_eq!(s.available, 2);
    }

    #[test]
    fn test_service_summary() {
        let svc = Service {
            metadata: ObjectMeta {
                name: Some("s1".into()),
                namespace: Some("ns1".into()),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                type_: Some("NodePort".into()),
                cluster_ip: Some("10.1.2.3".into()),
                ports: Some(vec![ServicePort {
                    port: 80,
                    node_port: Some(30080),
                    protocol: Some("TCP".into()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let s = service_summary(&svc);
        assert_eq!(s.name, "s1");
        assert_eq!(s.kind, "NodePort");
        assert_eq!(s.cluster_ip, "10.1.2.3");
        assert_eq!(s.ports, "TCP/80:30080");
    }

    fn sample_node(labels: BTreeMap<String, String>) -> Node {
        Node {
            metadata: ObjectMeta {
                name: Some("n1".into()),
                labels: Some(labels),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![NodeCondition {
                    type_: "Ready".into(),
                    status: "True".into(),
                    ..Default::default()
                }]),
                node_info: Some(NodeSystemInfo {
                    kubelet_version: "v1.36.3".into(),
                    os_image: "Talos 3.6".into(),
                    ..Default::default()
                }),
                addresses: Some(vec![NodeAddress {
                    type_: "InternalIP".into(),
                    address: "192.168.1.10".into(),
                }]),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_node_summary_control_plane() {
        let node = sample_node(BTreeMap::from([(
            "node-role.kubernetes.io/control-plane".to_string(),
            "true".to_string(),
        )]));
        let s = node_summary(&node);
        assert_eq!(s.name, "n1");
        assert_eq!(s.status, "Ready");
        assert_eq!(s.role, "control-plane");
        assert_eq!(s.kubernetes_version, "v1.36.3");
        assert_eq!(s.os_image, "Talos 3.6");
        assert_eq!(s.internal_ip, "192.168.1.10");
    }

    #[test]
    fn test_node_summary_worker() {
        let node = sample_node(BTreeMap::from([(
            "node-role.kubernetes.io/worker".to_string(),
            "true".to_string(),
        )]));
        assert_eq!(node_summary(&node).role, "worker");
    }

    #[test]
    fn test_event_summary() {
        let ev = Event {
            metadata: ObjectMeta {
                name: Some("e1".into()),
                namespace: Some("ns1".into()),
                ..Default::default()
            },
            involved_object: ObjectReference {
                kind: Some("Pod".into()),
                name: Some("p1".into()),
                ..Default::default()
            },
            reason: Some("BackOff".into()),
            message: Some("back-off restarting".into()),
            count: Some(5),
            ..Default::default()
        };
        let s = event_summary(&ev);
        assert_eq!(s.name, "e1");
        assert_eq!(s.namespace, "ns1");
        assert_eq!(s.kind, "Event");
        assert_eq!(s.reason, "BackOff");
        assert_eq!(s.message, "back-off restarting");
        assert_eq!(s.object, "Pod/p1");
        assert_eq!(s.count, 5);
    }

    #[test]
    fn test_container_detail() {
        let cs = ContainerStatus {
            name: "c1".into(),
            image: "img:1".into(),
            ready: true,
            restart_count: 0,
            state: Some(ContainerState {
                running: Some(ContainerStateRunning {
                    started_at: Some(Time(Utc::now() - Duration::seconds(10))),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let d = container_detail(&cs, None);
        assert_eq!(d.name, "c1");
        assert_eq!(d.image, "img:1");
        assert!(d.ready);
        assert_eq!(d.restarts, 0);
        assert_eq!(d.state, "running");
        assert!(!d.started_at.is_empty());
        assert!(d.started_at.ends_with('Z'));
        assert!(d.command.is_empty());
        assert!(d.args.is_empty());
    }

    #[test]
    fn test_pod_detail() {
        let pod = sample_pod();
        let d = pod_detail(&pod);
        assert_eq!(d.summary.name, "p1");
        assert_eq!(d.containers.len(), 1);
        assert!(!d.yaml.is_empty());
        assert!(d.yaml.contains("p1"));
    }
}
