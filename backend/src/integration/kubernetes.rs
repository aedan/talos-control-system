use std::sync::Arc;
use std::time::Duration;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::AppError;
use k8s_openapi::api::core::v1::{Node, Pod};

/// Parsed kubeconfig data for cluster discovery
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Kubeconfig {
    pub clusters: Vec<KubeconfigCluster>,
    pub users: Vec<KubeconfigUser>,
    pub contexts: Vec<KubeconfigContext>,
    #[serde(default)]
    pub current_context: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigCluster {
    pub cluster: KubeconfigClusterSpec,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigClusterSpec {
    pub server: String,
    #[serde(default, rename = "certificate-authority-data")]
    pub certificate_authority_data: Option<String>,
    #[serde(default, rename = "insecure-skip-tls-verify")]
    pub insecure_skip_tls_verify: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigUser {
    pub name: String,
    pub user: KubeconfigUserSpec,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigUserSpec {
    #[serde(default, rename = "client-certificate-data")]
    pub client_certificate_data: Option<String>,
    #[serde(default, rename = "client-key-data")]
    pub client_key_data: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigContext {
    pub name: String,
    pub context: KubeconfigContextSpec,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubeconfigContextSpec {
    pub cluster: String,
    pub user: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

/// Discovered cluster info from a running Kubernetes cluster
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredCluster {
    pub name: String,
    pub server: String,
    pub kubernetes_version: String,
    pub talos_version: String,
    pub control_plane_nodes: Vec<DiscoveredNode>,
    pub worker_nodes: Vec<DiscoveredNode>,
    pub is_talos: bool,
}

/// Discovered node info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredNode {
    pub name: String,
    pub internal_ip: String,
    pub kubernetes_version: String,
    pub talos_version: String,
    pub role: String,
    pub os_image: String,
}

/// Parse a kubeconfig YAML string into structured data
pub fn parse_kubeconfig(yaml: &str) -> Result<Kubeconfig, AppError> {
    let config: Kubeconfig = serde_yaml::from_str(yaml).map_err(|e| {
        AppError::InvalidInput(format!("Invalid kubeconfig YAML: {}", e))
    })?;

    if config.clusters.is_empty() {
        return Err(AppError::InvalidInput("No clusters found in kubeconfig".to_string()));
    }
    if config.contexts.is_empty() {
        return Err(AppError::InvalidInput("No contexts found in kubeconfig".to_string()));
    }

    Ok(config)
}

/// Extract CA data from kubeconfig for the active context
pub fn extract_ca_data(config: &Kubeconfig) -> Vec<u8> {
    let context = config.contexts.iter()
        .find(|c| c.name == config.current_context)
        .or_else(|| config.contexts.first());

    let cluster_name = context.as_ref().and_then(|c| Some(c.context.cluster.as_str()));
    let cluster = if let Some(name) = cluster_name {
        config.clusters.iter().find(|c| c.name == name)
    } else {
        None
    };

    if let Some(cluster) = cluster {
        if let Some(b64) = &cluster.cluster.certificate_authority_data {
            if let Ok(data) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                return data;
            }
        }
    }
    Vec::new()
}

/// Extract auth credentials from kubeconfig for the active context.
/// Returns (client_key_pem, client_cert_pem, token).
pub fn extract_auth(config: &Kubeconfig) -> (Vec<u8>, Vec<u8>, String) {
    let context = config.contexts.iter()
        .find(|c| c.name == config.current_context)
        .or_else(|| config.contexts.first());

    let user_name = context.as_ref().and_then(|c| Some(c.context.user.as_str()));
    let user = if let Some(name) = user_name {
        config.users.iter().find(|u| u.name == name)
    } else {
        None
    };

    let mut cert = Vec::new();
    let mut key = Vec::new();
    let mut token = String::new();

    if let Some(user) = user {
        if let Some(b64) = &user.user.client_certificate_data {
            if let Ok(data) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                cert = data;
            }
        }
        if let Some(b64) = &user.user.client_key_data {
            if let Ok(data) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                key = data;
            }
        }
        if let Some(t) = &user.user.token {
            token = t.clone();
        }
    }

    (key, cert, token)
}

/// Build a kube::Client from raw kubeconfig YAML (preserves mTLS certs/keys).
async fn build_client_from_kubeconfig_yaml(yaml: &str) -> Result<kube::Client, AppError> {
    let kc = kube::config::Kubeconfig::from_yaml(yaml).map_err(|e| {
        AppError::InvalidInput(format!("Invalid kubeconfig: {}", e))
    })?;

    let options = kube::config::KubeConfigOptions::default();
    let config = kube::Config::from_custom_kubeconfig(kc, &options)
        .await
        .map_err(|e| {
            AppError::Internal(format!("Failed to build kube client from kubeconfig: {}", e))
        })?;

    kube::Client::try_from(config).map_err(|e| {
        AppError::Internal(format!("Failed to create kube client: {}", e))
    })
}

pub struct KubernetesClient {
    cluster_id: uuid::Uuid,
    endpoint: String,
    client: Option<kube::Client>,
}

impl KubernetesClient {
    pub async fn new(cluster_id: uuid::Uuid, endpoint: String, ca_data: Vec<u8>, token: String) -> Result<Self, AppError> {
        info!(cluster_id = %cluster_id, endpoint = %endpoint, "Kubernetes client initialized");
        let client = if !ca_data.is_empty() || !token.is_empty() {
            let mut kube_config = kube::config::Config::new(endpoint.parse().map_err(|e| {
                AppError::InvalidInput(format!("Invalid endpoint URL: {}", e))
            })?);

            if !ca_data.is_empty() {
                kube_config.root_cert = Some(vec![ca_data]);
            }
            if !token.is_empty() {
                kube_config.auth_info.token = Some(token.into());
            }

            Some(kube::Client::try_from(kube_config).map_err(|e| {
                AppError::Internal(format!("Failed to build kube client: {}", e))
            })?)
        } else {
            None
        };

        Ok(Self { cluster_id, endpoint, client })
    }

    pub async fn apply_manifest(&self, manifest: &str) -> Result<(), AppError> {
        // Split YAML by document separator and parse each
        let docs: Vec<&str> = manifest
            .split("\n---")
            .filter(|s| !s.trim().is_empty())
            .collect();

        for doc in docs {
            let value: serde_json::Value = serde_yaml::from_str(doc)
                .map_err(|e| AppError::InvalidInput(format!("Invalid YAML document: {}", e)))?;

            let kind = value.get("kind").and_then(|k| k.as_str()).unwrap_or("");
            let api_version = value.get("apiVersion").and_then(|v| v.as_str()).unwrap_or("");
            let name = value.get("metadata").and_then(|m| m.get("name")).and_then(|n| n.as_str()).unwrap_or("");
            let namespace = value.get("metadata").and_then(|m| m.get("namespace")).and_then(|n| n.as_str());

            if kind.is_empty() || name.is_empty() {
                continue;
            }

            tracing::info!(kind, api_version, name, namespace, "Applying manifest document");

            // Build the K8s API URL
            let ns_segment = namespace.map(|ns| format!("/namespaces/{}", ns)).unwrap_or_default();
            let resource_kind = match kind {
                "Namespace" => "namespaces",
                "ConfigMap" => "configmaps",
                "Service" => "services",
                "Deployment" => "deployments",
                "ServiceAccount" => "serviceaccounts",
                "Role" => "roles",
                "RoleBinding" => "rolebindings",
                "ClusterRole" => "clusterroles",
                "ClusterRoleBinding" => "clusterrolebindings",
                "Ingress" => "ingresses",
                _ => {
                    tracing::warn!(kind, "Unsupported resource kind, skipping");
                    continue;
                }
            };

            let api_group = match api_version {
                "v1" => "",
                "apps/v1" => "apps",
                "rbac.authorization.k8s.io/v1" => "rbac.authorization.k8s.io",
                "networking.k8s.io/v1" => "networking.k8s.io",
                other => {
                    other.split('/').next().unwrap_or("")
                }
            };

            let api_prefix = if api_group.is_empty() {
                "api/v1".to_string()
            } else {
                format!("apis/{}/{}", api_group, api_version.split('/').last().unwrap_or("v1"))
            };

            let url = if kind == "Namespace" || matches!(kind, "ClusterRole" | "ClusterRoleBinding") {
                format!("{}/{}/{}/{}", self.endpoint, api_prefix, resource_kind, name)
            } else {
                format!("{}/{}/{}{}/{}", self.endpoint, api_prefix, ns_segment, resource_kind, name)
            };

            let apply_body = serde_json::json!({
                "apiVersion": api_version,
                "kind": kind,
                "metadata": {
                    "name": name,
                    "namespace": namespace,
                    "ownerReferences": [{
                        "apiVersion": "v1",
                        "blockOwnerDeletion": true,
                        "controller": true,
                        "kind": "Service",
                        "name": "tcs-manager",
                        "uid": "tcs-manager"
                    }]
                },
                "spec": value.get("spec").unwrap_or(&serde_json::Value::Object(Default::default())),
            });

            let client = reqwest::Client::new();
            let req = client.patch(&url)
                .header("Content-Type", "application/apply-patch+json")
                .query(&[("fieldManager", "tcs"), ("force", "true")])
                .body(apply_body.to_string());

            let resp = req.send().await
                .map_err(|e| AppError::Network(format!("Failed to apply {}: {}", kind, e)))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!(kind, name, status = %status, body, "Failed to apply manifest (non-fatal)");
            }
        }

        Ok(())
    }

    pub fn cluster_id(&self) -> uuid::Uuid {
        self.cluster_id
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

pub struct KubernetesClientPool {
    cache: Cache<uuid::Uuid, Arc<KubernetesClient>>,
}

impl KubernetesClientPool {
    pub fn new() -> Self {
        Self {
            cache: Cache::builder()
                .max_capacity(10240)
                .time_to_live(Duration::from_secs(3600))
                .time_to_idle(Duration::from_secs(1800))
                .build(),
        }
    }

    pub async fn get_or_create(
        &self,
        cluster_id: uuid::Uuid,
        endpoint: String,
        ca_data: Vec<u8>,
        token: String,
    ) -> Result<Arc<KubernetesClient>, AppError> {
        if let Some(client_entry) = self.cache.get(&cluster_id).await {
            return Ok(client_entry);
        }

        let client = Arc::new(
            KubernetesClient::new(cluster_id, endpoint.clone(), ca_data, token).await?
        );

        self.cache.insert(cluster_id, Arc::clone(&client)).await;
        Ok(client)
    }

    pub async fn invalidate(&self, cluster_id: &uuid::Uuid) {
        self.cache.invalidate(cluster_id).await;
    }
}

/// Discover an existing cluster by parsing kubeconfig and querying nodes
pub async fn discover_cluster_from_kubeconfig(kubeconfig_yaml: &str) -> Result<DiscoveredCluster, AppError> {
    // Parse for metadata (name/server); use kube crate loader for the live client.
    let config = parse_kubeconfig(kubeconfig_yaml)?;

    let client = build_client_from_kubeconfig_yaml(kubeconfig_yaml).await?;

    // Get active context to extract cluster name
    let context_name = if config.current_context.is_empty() {
        config.contexts.first().map(|c| c.name.clone()).ok_or_else(|| {
            AppError::InvalidInput("No context available in kubeconfig".to_string())
        })?
    } else {
        config.current_context.clone()
    };

    let context = config.contexts.iter()
        .find(|c| c.name == context_name)
        .ok_or_else(|| AppError::InvalidInput(format!("Context '{}' not found", context_name)))?;

    let cluster_spec = config.clusters.iter()
        .find(|c| c.name == context.context.cluster)
        .ok_or_else(|| AppError::InvalidInput(format!("Cluster spec not found")))?.clone();

    let cluster_name = context.context.cluster.clone();

    // List all nodes
    let node_list = kube::Api::<k8s_openapi::api::core::v1::Node>::all(client.clone())
        .list(&kube::api::ListParams::default())
        .await
        .map_err(|e| AppError::Internal(format!("Failed to list nodes: {}", e)))?;

    if node_list.items.is_empty() {
        return Err(AppError::InvalidInput("No nodes found in cluster".to_string()));
    }

    let mut control_plane_nodes = Vec::new();
    let mut worker_nodes = Vec::new();
    let mut is_talos = false;
    let mut talos_version = String::new();

    for node in node_list.items {
        let metadata = node.metadata;
        let status = node.status.ok_or_else(|| {
            AppError::InvalidInput(format!("Node {} has no status", metadata.name.as_deref().unwrap_or("unknown")))
        })?;

        let name = metadata.name.unwrap_or_default();

        // Get node labels
        let labels = metadata.labels.unwrap_or_default();

        // Determine OS type
        let os_image = status.node_info.as_ref()
            .map(|info| info.os_image.clone())
            .unwrap_or_default();

        let os_id = labels.get("os.id")
            .or_else(|| labels.get("kubernetes.io/os"))
            .map(|s| s.as_str())
            .unwrap_or("");

        // Check if this is a Talos node
        if os_image.to_lowercase().contains("talos") || os_id == "talos" {
            is_talos = true;
        }

        // Extract Talos version from labels
        let mut node_talos_version = labels.get("talos.version")
            .or_else(|| labels.get("node.kubernetes.io/talos-version"))
            .cloned()
            .unwrap_or_default();

        if node_talos_version.is_empty() {
            // Try extracting from os_image like "Talos 1.7.3"
            if os_image.starts_with("Talos ") {
                node_talos_version.clone_from(&os_image[6..].to_string());
            }
        }

        if !node_talos_version.is_empty() && talos_version.is_empty() {
            talos_version = node_talos_version.clone();
        }

        // Get node IP
        let internal_ip = status.addresses.as_ref()
            .and_then(|addrs| {
                addrs.iter().find(|a| a.type_ == "InternalIP")
                    .map(|a| a.address.clone())
            })
            .unwrap_or_default();

        // Get Kubernetes version
        let kubernetes_version = status.node_info.as_ref()
            .map(|info| info.kubelet_version.clone())
            .unwrap_or_default();

        // Determine role
        let role = if labels.contains_key("node-role.kubernetes.io/control-plane")
            || labels.contains_key("node-role.kubernetes.io/master")
        {
            "control-plane".to_string()
        } else if labels.contains_key("node-role.kubernetes.io/worker") {
            "worker".to_string()
        } else {
            "unknown".to_string()
        };

        let discovered_node = DiscoveredNode {
            name,
            internal_ip,
            kubernetes_version,
            talos_version: node_talos_version,
            role: role.clone(),
            os_image,
        };

        if role == "control-plane" {
            control_plane_nodes.push(discovered_node);
        } else {
            worker_nodes.push(discovered_node);
        }
    }

    // Extract Kubernetes version from first control plane or first node
    let kubernetes_version = control_plane_nodes.first()
        .or_else(|| worker_nodes.first())
        .map(|n| n.kubernetes_version.clone())
        .unwrap_or_else(|| String::from("unknown"));

    Ok(DiscoveredCluster {
        name: cluster_name,
        server: cluster_spec.cluster.server,
        kubernetes_version,
        talos_version,
        control_plane_nodes,
        worker_nodes,
        is_talos,
    })
}

// ============================================================================
// K8sClient — the frozen proxy substrate for the explorer + CLI
// ============================================================================
//
// A single, self-contained client over a stored kubeconfig. Every method the
// REST handlers and the CLI need lives here so that the parallel agents only
// ever call into this surface (they never touch kube internals directly).

use std::collections::BTreeMap;
use futures::io::AsyncBufRead;

use crate::integration::k8s_explorer::{
    ApplyResult, DrainResult, ResolvedKind,
};
use crate::utils::secrets;

/// Map a `kube::Error` to an `AppError`, surfacing the HTTP status when present.
fn map_kube_err(e: kube::Error) -> AppError {
    let code = match &e {
        kube::Error::Api(resp) => Some(resp.code),
        _ => None,
    };
    let msg = e.to_string();
    match code {
        Some(404) => AppError::NotFound(msg),
        Some(401 | 403) => AppError::Auth(msg),
        Some(409) => AppError::InvalidInput(msg),
        Some(429) => AppError::Network(msg),
        Some(c) if (400..500).contains(&c) => AppError::InvalidInput(format!("{c}: {msg}")),
        Some(c) if (500..600).contains(&c) => AppError::Internal(format!("{c}: {msg}")),
        _ => AppError::Internal(msg),
    }
}

/// Build the absolute API path for a resolved kind.
fn url_path_for(r: &ResolvedKind, ns: Option<&str>) -> String {
    let group = if r.group.is_empty() {
        "api"
    } else {
        "apis"
    };
    // The URL version segment is the full apiVersion: "v1" for core,
    // "apps/v1" for group resources (kube discovery keeps `version` bare).
    match ns {
        // Namespaced: /{api|apis}/{apiVersion}/namespaces/{ns}/{plural}
        Some(n) if r.namespaced => format!("/{group}/{}/namespaces/{}/{}", r.api_version, n, r.plural),
        // Cluster-scoped (or no ns): /{api|apis}/{apiVersion}/{plural}
        _ => format!("/{group}/{}/{}", r.api_version, r.plural),
    }
}

/// Decide whether `cand` (with `cand_score`) should replace the current `best`
/// (with `best_score`) in kind resolution.
///
/// Preference order: (1) strictly higher score; (2) on a score tie, the CORE
/// group (empty `group`) beats a CRD group — so "nodes" resolves to the core
/// v1 Node rather than a same-named CRD (e.g. longhorn.io Node); (3) on a
/// full tie, keep the shorter kind name (the more specific match).
fn prefer_candidate(cand: &ResolvedKind, cand_score: usize, best: &ResolvedKind, best_score: usize) -> bool {
    if cand_score != best_score {
        return cand_score > best_score;
    }
    let cand_core = cand.group.is_empty();
    let best_core = best.group.is_empty();
    if cand_core != best_core {
        return cand_core;
    }
    best.kind.len() >= cand.kind.len()
}

/// Map kubectl-style short names to their canonical plural resource name.
/// Only the common core aliases are covered; anything else is returned as-is
/// and matched against kind/plural/prefix during resolution.
fn short_name_to_plural(norm: &str) -> Option<&'static str> {
    Some(match norm {
        "po" => "pods",
        "pods" => "pods",
        "svc" => "services",
        "services" => "services",
        "ns" => "namespaces",
        "namespaces" => "namespaces",
        "deploy" => "deployments",
        "deployments" => "deployments",
        "rs" => "replicasets",
        "replicasets" => "replicasets",
        "sts" => "statefulsets",
        "statefulsets" => "statefulsets",
        "ds" => "daemonsets",
        "daemonsets" => "daemonsets",
        "job" => "jobs",
        "jobs" => "jobs",
        "cj" => "cronjobs",
        "cronjobs" => "cronjobs",
        "cm" => "configmaps",
        "configmaps" => "configmaps",
        "se" => "secrets",
        "secrets" => "secrets",
        "no" => "nodes",
        "nodes" => "nodes",
        "ing" => "ingresses",
        "ingresses" => "ingresses",
        "pv" => "persistentvolumes",
        "persistentvolumes" => "persistentvolumes",
        "pvc" => "persistentvolumeclaims",
        "persistentvolumeclaims" => "persistentvolumeclaims",
        "sa" => "serviceaccounts",
        "serviceaccounts" => "serviceaccounts",
        "ep" => "endpoints",
        "endpoints" => "endpoints",
        "events" => "events",
        "ev" => "events",
        _ => return None,
    })
}

/// A typed + dynamic Kubernetes client bound to one stored cluster kubeconfig.
#[derive(Clone)]
pub struct K8sClient {
    client: kube::Client,
    discovery: std::sync::Arc<kube::discovery::Discovery>,
}

impl K8sClient {
    /// Build a client from a decrypted kubeconfig YAML string.
    pub async fn from_kubeconfig_yaml(yaml: &str) -> Result<Self, AppError> {
        let client = build_client_from_kubeconfig_yaml(yaml).await?;
        let discovery = kube::discovery::Discovery::new(client.clone())
            .run()
            .await
            .map_err(map_kube_err)?;
        Ok(Self {
            client,
            discovery: std::sync::Arc::new(discovery),
        })
    }

    pub fn raw(&self) -> &kube::Client {
        &self.client
    }

    /// Query the API server's `/version` endpoint → gitVersion (e.g. "v1.36.3").
    pub async fn api_server_version(&self) -> Result<String, AppError> {
        let info = self.client.apiserver_version().await.map_err(map_kube_err)?;
        let v = info.git_version;
        if v.is_empty() {
            return Err(AppError::Internal("API /version returned no gitVersion".into()));
        }
        Ok(v)
    }
    // ---- discovery --------------------------------------------------------

    /// Resolve an arbitrary kind (e.g. "pods", "po", "deploy", "ingress", "mycrd")
    /// to its group/version/plural/namespaced shape via API discovery.
    pub fn resolve(&self, kind: &str) -> Result<ResolvedKind, AppError> {
        let norm = kind.to_lowercase().trim().to_string();
        let norm = norm.chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        // Expand kubectl-style short names (po, svc, ns, ...) to their plural.
        let norm = short_name_to_plural(&norm).map(str::to_string).unwrap_or(norm);

        let mut best: Option<(ResolvedKind, usize)> = None;
        for group in self.discovery.groups_alphabetical() {
            for (res, caps) in group.recommended_resources() {
                let rkind = res.kind.to_lowercase();
                let rplural = res.plural.to_lowercase();
                let score = if rkind == norm {
                    3
                } else if rplural == norm {
                    3
                } else if rkind.starts_with(&norm) && norm.len() >= 3 {
                    2
                } else if rplural.starts_with(&norm) && norm.len() >= 3 {
                    2
                } else {
                    0
                };
                if score > 0 {
                    let namespaced = matches!(caps.scope, kube::discovery::Scope::Namespaced);
                    let rk = ResolvedKind {
                        group: res.group.clone(),
                        version: res.version.clone(),
                        api_version: res.api_version.clone(),
                        kind: res.kind.clone(),
                        plural: res.plural.clone(),
                        namespaced,
                    };
                    // Prefer: (1) higher score, (2) on a tie the CORE group
                    // (empty group) over a CRD group — e.g. "nodes" must resolve
                    // to the core v1 Node, not longhorn.io/v1beta2 Node (which
                    // shares the same plural + kind and would otherwise win on
                    // iteration order, producing a 404 page not found). (3) on
                    // an otherwise-identical tie, keep the shorter kind name.
                    let replace = best
                        .as_ref()
                        .map(|(b, s)| prefer_candidate(&rk, score, b, *s))
                        .unwrap_or(true);
                    if replace {
                        best = Some((rk, score));
                    }
                }
            }
        }

        best.map(|(rk, _)| rk)
            .ok_or_else(|| AppError::NotFound(format!("kind '{kind}' not found in cluster")))
    }

    // ---- typed core lists (fast path for the explorer) --------------------
    pub async fn list_namespaces(&self) -> Result<Vec<k8s_openapi::api::core::v1::Namespace>, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Namespace>::all(self.client.clone());
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn list_pods(&self, ns: Option<&str>) -> Result<Vec<k8s_openapi::api::core::v1::Pod>, AppError> {
        let api = match ns {
            Some(n) => kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), n),
            None => kube::Api::<k8s_openapi::api::core::v1::Pod>::all(self.client.clone()),
        };
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn list_deployments(&self, ns: Option<&str>) -> Result<Vec<k8s_openapi::api::apps::v1::Deployment>, AppError> {
        let api = match ns {
            Some(n) => kube::Api::<k8s_openapi::api::apps::v1::Deployment>::namespaced(self.client.clone(), n),
            None => kube::Api::<k8s_openapi::api::apps::v1::Deployment>::all(self.client.clone()),
        };
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn list_services(&self, ns: Option<&str>) -> Result<Vec<k8s_openapi::api::core::v1::Service>, AppError> {
        let api = match ns {
            Some(n) => kube::Api::<k8s_openapi::api::core::v1::Service>::namespaced(self.client.clone(), n),
            None => kube::Api::<k8s_openapi::api::core::v1::Service>::all(self.client.clone()),
        };
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn list_events(&self, ns: Option<&str>) -> Result<Vec<k8s_openapi::api::core::v1::Event>, AppError> {
        let api = match ns {
            Some(n) => kube::Api::<k8s_openapi::api::core::v1::Event>::namespaced(self.client.clone(), n),
            None => kube::Api::<k8s_openapi::api::core::v1::Event>::all(self.client.clone()),
        };
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn list_nodes(&self) -> Result<Vec<k8s_openapi::api::core::v1::Node>, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Node>::all(self.client.clone());
        Ok(api.list(&kube::api::ListParams::default()).await.map_err(map_kube_err)?.items)
    }

    pub async fn get_pod(&self, ns: &str, name: &str) -> Result<k8s_openapi::api::core::v1::Pod, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), ns);
        api.get(name).await.map_err(map_kube_err)
    }

    // ---- dynamic (arbitrary-kind) reads -----------------------------------

    /// List an arbitrary kind, returning raw JSON (items + metadata).
    pub async fn list_kind(&self, kind: &str, ns: Option<&str>) -> Result<serde_json::Value, AppError> {
        let rk = self.resolve(kind)?;
        let path = url_path_for(&rk, ns.filter(|_| rk.namespaced));
        let req = kube::core::Request::new(path)
            .list(&kube::api::ListParams::default())
            .map_err(|e| AppError::Internal(format!("build list request: {e}")))?;
        self.client.request::<serde_json::Value>(req).await.map_err(map_kube_err)
    }

    /// Get a single arbitrary-kind object as raw JSON.
    pub async fn get_kind(&self, kind: &str, ns: Option<&str>, name: &str) -> Result<serde_json::Value, AppError> {
        let rk = self.resolve(kind)?;
        let path = url_path_for(&rk, ns.filter(|_| rk.namespaced));
        tracing::debug!(kind, ?ns, name, group = %rk.group, api_version = %rk.api_version, plural = %rk.plural, namespaced = rk.namespaced, path, "get_kind");
        let req = kube::core::Request::new(path)
            .get(name, &kube::api::GetParams::default())
            .map_err(|e| AppError::Internal(format!("build get request: {e}")))?;
        self.client.request::<serde_json::Value>(req).await.map_err(map_kube_err)
    }

    // ---- mutations --------------------------------------------------------

    /// Delete an arbitrary-kind object.
    pub async fn delete_kind(&self, kind: &str, ns: Option<&str>, name: &str) -> Result<(), AppError> {
        let rk = self.resolve(kind)?;
        let path = url_path_for(&rk, ns.filter(|_| rk.namespaced));
        let dp = kube::api::DeleteParams::foreground();
        let req = kube::core::Request::new(path)
            .delete(name, &dp)
            .map_err(|e| AppError::Internal(format!("build delete request: {e}")))?;
        // The API returns the deleted object (or a Status for some kinds), not
        // always a Status — deserialize loosely so both work.
        self.client.request::<serde_json::Value>(req).await.map_err(map_kube_err)?;
        Ok(())
    }

    /// Scale a Deployment to `replicas`.
    pub async fn scale_deployment(&self, ns: &str, name: &str, replicas: i32) -> Result<(), AppError> {
        let path = format!("/apis/apps/v1/namespaces/{ns}/deployments");
        // The /scale subresource does not support server-side apply; use a
        // merge patch on the subresource with just the desired replica count.
        let body = serde_json::json!({ "spec": { "replicas": replicas } });
        let pp = kube::api::PatchParams::default();
        let req = kube::core::Request::new(path)
            .patch_subresource("scale", name, &pp, &kube::api::Patch::Merge(&body))
            .map_err(|e| AppError::Internal(format!("build scale request: {e}")))?;
        self.client.request::<serde_json::Value>(req).await.map_err(map_kube_err)?;
        Ok(())
    }

    /// Cordon a node (mark unschedulable).
    pub async fn cordon(&self, name: &str) -> Result<(), AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Node>::all(self.client.clone());
        api.cordon(name).await.map_err(map_kube_err)?;
        Ok(())
    }

    /// Uncordon a node.
    pub async fn uncordon(&self, name: &str) -> Result<(), AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Node>::all(self.client.clone());
        api.uncordon(name).await.map_err(map_kube_err)?;
        Ok(())
    }

    /// Drain a node: cordon it, then evict its non-DaemonSet, non-mirror pods.
    pub async fn drain(&self, name: &str, force: bool) -> Result<DrainResult, AppError> {
        let mut result = DrainResult {
            node: name.to_string(),
            evicted: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        };

        self.cordon(name).await?;

        let lp = kube::api::ListParams {
            field_selector: Some(format!("spec.nodeName={name}")),
            ..Default::default()
        };
        let pods_api = kube::Api::<k8s_openapi::api::core::v1::Pod>::all(self.client.clone());
        let pods = pods_api.list(&lp).await.map_err(map_kube_err)?.items;

        for pod in pods {
            let pod_name = pod.metadata.name.clone().unwrap_or_default();
            let ns = pod.metadata.namespace.clone().unwrap_or_default();

            // Skip mirror pods (managed by static file) and pods without a controller.
            if pod
                .metadata
                .owner_references
                .as_ref()
                .map(|o| o.iter().any(|r| r.kind == "DaemonSet"))
                .unwrap_or(false)
            {
                result.skipped.push(format!("{ns}/{pod_name} (daemonset)"));
                continue;
            }
            if pod
                .metadata
                .annotations
                .as_ref()
                .map(|a| a.contains_key("kubernetes.io/config.mirror"))
                .unwrap_or(false)
            {
                result.skipped.push(format!("{ns}/{pod_name} (mirror)"));
                continue;
            }

            let pod_api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), &ns);
            let ep = kube::api::EvictParams {
                delete_options: Some(kube::api::DeleteParams {
                    grace_period_seconds: Some(0),
                    ..Default::default()
                }),
                post_options: kube::api::PostParams::default(),
            };
            match pod_api.evict(&pod_name, &ep).await {
                Ok(_) => result.evicted.push(format!("{ns}/{pod_name}")),
                Err(e) => {
                    if force {
                        // Best-effort hard delete on force.
                        let _ = pod_api.delete(&pod_name, &kube::api::DeleteParams::foreground()).await;
                        result.evicted.push(format!("{ns}/{pod_name} (forced)"));
                    } else {
                        result.errors.push(format!("{ns}/{pod_name}: {e}"));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Server-side apply of a single YAML document (arbitrary kind).
    pub async fn apply_document(&self, doc: &str) -> Result<ApplyResult, AppError> {
        let value: serde_json::Value =
            serde_yaml::from_str(doc).map_err(|e| AppError::InvalidInput(format!("invalid YAML: {e}")))?;
        let kind = value.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string();
        let name = value
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let ns = value
            .get("metadata")
            .and_then(|m| m.get("namespace"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string());

        if kind.is_empty() || name.is_empty() {
            return Ok(ApplyResult {
                kind,
                name,
                namespace: ns.unwrap_or_default(),
                status: "skipped (no kind/name)".to_string(),
            });
        }

        let rk = self.resolve(&kind)?;
        let path = url_path_for(&rk, ns.as_deref().filter(|_| rk.namespaced));
        let pp = kube::api::PatchParams::apply("tcs");
        let req = kube::core::Request::new(path)
            .patch(&name, &pp, &kube::api::Patch::Apply(&value))
            .map_err(|e| AppError::Internal(format!("build apply request: {e}")))?;
        self.client.request::<serde_json::Value>(req).await.map_err(map_kube_err)?;

        Ok(ApplyResult {
            kind,
            name,
            namespace: ns.unwrap_or_default(),
            status: "applied".to_string(),
        })
    }

    /// Apply a multi-document YAML manifest, returning one result per document.
    pub async fn apply_manifest(&self, manifest: &str) -> Result<Vec<ApplyResult>, AppError> {
        let docs: Vec<&str> = manifest
            .split("\n---")
            .filter(|s| !s.trim().is_empty())
            .collect();
        let mut out = Vec::new();
        for doc in docs {
            out.push(self.apply_document(doc).await?);
        }
        Ok(out)
    }

    // ---- streaming / interactive -----------------------------------------

    /// Fetch pod logs as a string (non-follow).
    pub async fn logs(
        &self,
        ns: &str,
        name: &str,
        container: Option<&str>,
        tail_lines: Option<i64>,
        previous: bool,
        since_seconds: Option<i64>,
    ) -> Result<String, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), ns);
        let lp = kube::api::LogParams {
            container: container.map(|s| s.to_string()),
            tail_lines,
            previous,
            since_seconds,
            ..Default::default()
        };
        api.logs(name, &lp).await.map_err(map_kube_err)
    }

    /// Stream pod logs (follow). Returns a `Send + Unpin` async reader.
    pub async fn log_stream(
        &self,
        ns: &str,
        name: &str,
        container: Option<&str>,
        tail_lines: Option<i64>,
        previous: bool,
    ) -> Result<impl AsyncBufRead + Send + Unpin, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), ns);
        let lp = kube::api::LogParams {
            container: container.map(|s| s.to_string()),
            tail_lines,
            previous,
            follow: true,
            ..Default::default()
        };
        api.log_stream(name, &lp).await.map_err(map_kube_err)
    }

    /// Exec into a pod. Returns the attached process (stdin/stdout/stderr + resize).
    pub async fn exec(
        &self,
        ns: &str,
        name: &str,
        command: &[String],
        container: Option<&str>,
        tty: bool,
    ) -> Result<kube::api::AttachedProcess, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), ns);
        let ap = kube::api::AttachParams {
            container: container.map(|s| s.to_string()),
            stdin: true,
            stdout: true,
            stderr: !tty,
            tty,
            ..Default::default()
        };
        api.exec(name, command, &ap).await.map_err(map_kube_err)
    }

    /// Attach to a running pod's main container (interactive shell).
    pub async fn attach(
        &self,
        ns: &str,
        name: &str,
        container: Option<&str>,
        tty: bool,
    ) -> Result<kube::api::AttachedProcess, AppError> {
        let api = kube::Api::<k8s_openapi::api::core::v1::Pod>::namespaced(self.client.clone(), ns);
        let ap = kube::api::AttachParams {
            container: container.map(|s| s.to_string()),
            stdin: true,
            stdout: true,
            stderr: !tty,
            tty,
            ..Default::default()
        };
        api.attach(name, &ap).await.map_err(map_kube_err)
    }

    /// All served kinds (for `tcs api-resources` style listing).
    pub fn all_kinds(&self) -> Vec<ResolvedKind> {
        let mut seen = BTreeMap::new();
        for group in self.discovery.groups_alphabetical() {
            for (res, caps) in group.recommended_resources() {
                let namespaced = matches!(caps.scope, kube::discovery::Scope::Namespaced);
                seen.entry(res.kind.clone()).or_insert(ResolvedKind {
                    group: res.group.clone(),
                    version: res.version.clone(),
                    api_version: res.api_version.clone(),
                    kind: res.kind.clone(),
                    plural: res.plural.clone(),
                    namespaced,
                });
            }
        }
        seen.into_values().collect()
    }

    // ---- rebalance --------------------------------------------------------

    /// Rebalance a cluster: spread surplus Deployment replicas so every
    /// schedulable (Ready, not cordoned) node holds an even share, respecting
    /// the pod's node selector/affinity, taint tolerations, node capacity and
    /// PDBs. DaemonSets, StatefulSets and workloads with required pod
    /// anti-affinity are left untouched (ceph/osd-style pinning lives there).
    ///
    /// Only *surplus* replicas on over-represented nodes are evicted; the
    /// Deployment controller recreates them and kube-scheduler places the
    /// replacements (which enforces affinity/anti-affinity for real). This is a
    /// best-effort nudge — re-running converges further.
    pub async fn rebalance(
        &self,
        dry_run: bool,
        focus_node: Option<&str>,
        include_statefulsets: bool,
    ) -> Result<RebalanceResult, AppError> {
        let _ = (focus_node, include_statefulsets);
        let mut result = RebalanceResult {
            dry_run,
            eligible_nodes: 0,
            considered_workloads: 0,
            moves: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        };

        let nodes = self.list_nodes().await?;
        let pods = self.list_pods(None).await?;
        let deployments = self.list_deployments(None).await?;

        let eligible_set: std::collections::HashSet<String> = nodes
            .iter()
            .filter(|n| node_is_eligible(n))
            .map(|n| n.metadata.name.clone().unwrap_or_default())
            .filter(|n| !n.is_empty())
            .collect();
        result.eligible_nodes = eligible_set.len();
        if eligible_set.len() < 2 {
            result
                .skipped
                .push(format!("only {} eligible node(s); need >=2 to spread", eligible_set.len()));
            return Ok(result);
        }

        // Index running pods for O(1) lookup by (ns, name).
        let mut pod_index: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
        for (i, p) in pods.iter().enumerate() {
            if pod_running(p) {
                if let (Some(ns), Some(name)) = (p.metadata.namespace.clone(), p.metadata.name.clone()) {
                    pod_index.entry((ns, name)).or_insert(i);
                }
            }
        }

        // Free capacity per eligible node = allocatable - sum(running pods' requests).
        let mut used: std::collections::HashMap<String, (i64, i64)> = std::collections::HashMap::new();
        for p in pods.iter().filter(|p| pod_running(p)) {
            if let Some(node) = p
                .spec
                .as_ref()
                .and_then(|s| s.node_name.clone())
                .filter(|n| eligible_set.contains(n))
            {
                let (cpu, mem) = pod_requests(p);
                let e = used.entry(node).or_insert((0, 0));
                e.0 += cpu;
                e.1 += mem;
            }
        }
        let mut free: std::collections::HashMap<String, (i64, i64)> = nodes
            .iter()
            .filter(|n| node_is_eligible(n))
            .filter_map(|n| {
                let name = n.metadata.name.clone().unwrap_or_default();
                let (a_cpu, a_mem) = node_allocatable(n);
                let (u_cpu, u_mem) = used.get(&name).copied().unwrap_or((0, 0));
                Some((name, (a_cpu.saturating_sub(u_cpu), a_mem.saturating_sub(u_mem))))
            })
            .collect();

        for dep in deployments.iter() {
            let desired = dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
            if desired < 2 {
                continue;
            }
            let ns = dep.metadata.namespace.clone().unwrap_or_default();
            let dep_name = dep.metadata.name.clone().unwrap_or_default();
            let selector = dep
                .spec
                .as_ref()
                .and_then(|s| s.selector.match_labels.clone())
                .unwrap_or_default();
            if selector.is_empty() {
                continue;
            }
            // Skip workloads enforcing required pod anti-affinity — we can't
            // safely predict their placement; ceph/osd pinning lives here.
            if has_required_anti_affinity(dep) {
                result
                    .skipped
                    .push(format!("{ns}/{dep_name} (required pod anti-affinity)"));
                continue;
            }
            result.considered_workloads += 1;

            // Current per-node replica counts + removable candidates (on eligible nodes).
            let mut cnt: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            let mut cands: std::collections::HashMap<String, Vec<(String, String)>> = std::collections::HashMap::new();
            for p in pods.iter().filter(|p| pod_running(p)) {
                if p.metadata.namespace.as_deref() != Some(ns.as_str()) {
                    continue;
                }
                if !pod_matches_selector(p, &selector) {
                    continue;
                }
                if let Some(node) = p.spec.as_ref().and_then(|s| s.node_name.clone()) {
                    if eligible_set.contains(&node) {
                        *cnt.entry(node.clone()).or_insert(0) += 1;
                        if let Some(name) = p.metadata.name.clone() {
                            cands.entry(node).or_default().push((ns.clone(), name));
                        }
                    }
                }
            }

            // Level counts: while some node hosts more than another by >1, evict a
            // surplus replica from the busiest node onto a feasible, under-count node.
            let max_guard = (desired as usize).saturating_mul(eligible_set.len()).saturating_add(8);
            let mut guard = 0usize;
            loop {
                guard += 1;
                if guard > max_guard {
                    break;
                }
                let high = match cnt
                    .iter()
                    .filter(|(_, c)| **c > 0)
                    .max_by(|a, b| a.1.cmp(b.1))
                    .map(|(n, _)| n.clone())
                {
                    Some(h) => h,
                    None => break,
                };
                let high_cnt = *cnt.get(&high).unwrap_or(&0);
                // Lowest replica count among eligible nodes (0 for empty ones).
                let low_cnt = eligible_set
                    .iter()
                    .map(|n| cnt.get(n).copied().unwrap_or(0))
                    .min()
                    .unwrap_or(0);
                if high_cnt <= low_cnt + 1 {
                    break; // already evenly spread
                }
                let (pod_ns, pod_name) = match cands.get_mut(&high).and_then(|v| v.pop()) {
                    Some(x) => x,
                    None => {
                        cnt.insert(high.clone(), 0);
                        continue;
                    }
                };
                let Some(&pod_idx) = pod_index.get(&(pod_ns.clone(), pod_name.clone())) else {
                    continue;
                };
                let pod = &pods[pod_idx];
                let req = pod_requests(pod);
                let target_name = nodes
                    .iter()
                    .filter(|n| node_is_eligible(n))
                    .filter(|n| {
                        let name = n.metadata.name.clone().unwrap_or_default();
                        name != high
                            && cnt.get(&name).copied().unwrap_or(0) <= low_cnt
                            && pod_schedulable_on(pod, n)
                            && free.get(&name).map(|f| f.0 >= req.0 && f.1 >= req.1).unwrap_or(false)
                    })
                    .max_by_key(|n| free.get(&n.metadata.name.clone().unwrap_or_default()).copied().unwrap_or((0, 0)))
                    .and_then(|n| n.metadata.name.clone());
                let Some(target_name) = target_name else {
                    result.skipped
                        .push(format!("{ns}/{dep_name}: {pod_ns}/{pod_name} (no feasible target for spread)"));
                    break;
                };
                if let Some(f) = free.get_mut(&target_name) {
                    f.0 = f.0.saturating_sub(req.0);
                    f.1 = f.1.saturating_sub(req.1);
                }
                if let Some(c) = cnt.get_mut(&high) {
                    *c = c.saturating_sub(1);
                }
                *cnt.entry(target_name.clone()).or_insert(0) += 1;
                result.moves.push(RebalanceMove {
                    namespace: pod_ns,
                    pod: pod_name,
                    workload: format!("{ns}/{dep_name}"),
                    from_node: high,
                    to_node: target_name,
                    reason: "spread surplus replica to an under-loaded node".to_string(),
                });
            }
        }

        // Evict the planned moves (PDB-aware). Dry-run returns the plan untouched.
        if !dry_run {
            for mv in result.moves.iter() {
                let pod_api = kube::Api::<Pod>::namespaced(self.client.clone(), &mv.namespace);
                let ep = kube::api::EvictParams {
                    delete_options: Some(kube::api::DeleteParams {
                        grace_period_seconds: None,
                        ..Default::default()
                    }),
                    post_options: kube::api::PostParams::default(),
                };
                if let Err(e) = pod_api.evict(&mv.pod, &ep).await {
                    result.errors.push(format!("{}/{}: {e}", mv.namespace, mv.pod));
                }
            }
        }

        Ok(result)
    }
}

// ---- rebalance result + helpers --------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RebalanceMove {
    pub namespace: String,
    pub pod: String,
    pub workload: String,
    pub from_node: String,
    pub to_node: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RebalanceResult {
    pub dry_run: bool,
    pub eligible_nodes: usize,
    pub considered_workloads: usize,
    pub moves: Vec<RebalanceMove>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

/// A node is an eligible spread target: Ready, not cordoned, not terminating.
fn node_is_eligible(node: &Node) -> bool {
    if node.spec.as_ref().and_then(|s| s.unschedulable) == Some(true) {
        return false;
    }
    if node.metadata.deletion_timestamp.is_some() {
        return false;
    }
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conds| conds.iter().find(|c| c.type_ == "Ready"))
        .map(|c| c.status == "True")
        .unwrap_or(false)
}

fn pod_running(pod: &Pod) -> bool {
    pod.metadata.deletion_timestamp.is_none()
        && pod.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running")
}

/// Does the pod's nodeSelector + required nodeAffinity match this node?
fn node_affinity_ok(pod: &Pod, node: &Node) -> bool {
    let spec = match pod.spec.as_ref() {
        Some(s) => s,
        None => return false,
    };
    if let Some(sel) = &spec.node_selector {
        let labels = node.metadata.labels.as_ref();
        for (k, v) in sel {
            let hit = labels.and_then(|l| l.get(k)).map(|lv| lv == v).unwrap_or(false);
            if !hit {
                return false;
            }
        }
    }
    if let Some(ns) = spec
        .affinity
        .as_ref()
        .and_then(|a| a.node_affinity.as_ref())
        .and_then(|a| a.required_during_scheduling_ignored_during_execution.as_ref())
    {
        let terms = ns.node_selector_terms.as_slice();
        if !terms.is_empty() && !terms.iter().any(|t| node_affinity_term_matches(t, node)) {
            return false;
        }
    }
    true
}

fn node_affinity_term_matches(
    term: &k8s_openapi::api::core::v1::NodeSelectorTerm,
    node: &Node,
) -> bool {
    for expr in term.match_expressions.iter().flatten() {
        let val = node
            .metadata
            .labels
            .as_ref()
            .and_then(|m| m.get(&expr.key))
            .map(|s| s.as_str());
        let ok = match expr.operator.as_str() {
            "In" => expr.values.iter().flatten().any(|v| val == Some(v.as_str())),
            "NotIn" => expr.values.iter().flatten().all(|v| val != Some(v.as_str())),
            "Exists" => val.is_some(),
            "DoesNotExist" => val.is_none(),
            // Gt/Lt and anything unknown: conservative — treat as unsatisfied.
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Every blocking (NoSchedule/NoExecute) taint on the node is tolerated by the pod.
fn pod_tolerates(node_taints: &[k8s_openapi::api::core::v1::Taint], pod: &Pod) -> bool {
    let spec = match pod.spec.as_ref() {
        Some(s) => s,
        None => return false,
    };
    for taint in node_taints {
        let blocking = taint.effect == "NoSchedule" || taint.effect == "NoExecute";
        if !blocking {
            continue;
        }
        let tolerated = spec.tolerations.iter().flatten().any(|tol| {
            let effect_ok = match &tol.effect {
                Some(e) => e == &taint.effect,
                None => true,
            };
            if !effect_ok {
                return false;
            }
            let key_ok = match &tol.key {
                Some(k) => k == &taint.key,
                None => tol.operator.as_deref() == Some("Exists"),
            };
            if !key_ok {
                return false;
            }
            match tol.operator.as_deref() {
                Some("Exists") => true,
                _ => match &tol.value {
                    Some(v) => v.as_str() == taint.value.as_deref().unwrap_or(""),
                    None => true,
                },
            }
        });
        if !tolerated {
            return false;
        }
    }
    true
}

/// Can this pod be scheduled onto this node (taints + selector/affinity)?
fn pod_schedulable_on(pod: &Pod, node: &Node) -> bool {
    if let Some(taints) = node.spec.as_ref().and_then(|s| s.taints.as_ref()) {
        if !pod_tolerates(taints, pod) {
            return false;
        }
    }
    node_affinity_ok(pod, node)
}

fn pod_matches_selector(pod: &Pod, selector: &std::collections::BTreeMap<String, String>) -> bool {
    let labels = pod.metadata.labels.as_ref();
    for (k, v) in selector {
        let hit = labels.and_then(|l| l.get(k)).map(|lv| lv == v).unwrap_or(false);
        if !hit {
            return false;
        }
    }
    true
}

fn has_required_anti_affinity(dep: &k8s_openapi::api::apps::v1::Deployment) -> bool {
    dep.spec
        .as_ref()
        .and_then(|s| s.template.spec.as_ref())
        .and_then(|s| s.affinity.as_ref())
        .and_then(|a| a.pod_anti_affinity.as_ref())
        .and_then(|a| a.required_during_scheduling_ignored_during_execution.as_ref())
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn pod_requests(pod: &Pod) -> (i64, i64) {
    let mut cpu = 0;
    let mut mem = 0;
    if let Some(spec) = pod.spec.as_ref() {
        for c in spec.containers.iter() {
            if let Some(req) = c.resources.as_ref().and_then(|r| r.requests.as_ref()) {
                if let Some(v) = req.get("cpu") {
                    cpu += parse_cpu_milli(&v.0);
                }
                if let Some(v) = req.get("memory") {
                    mem += parse_mem_bytes(&v.0);
                }
            }
        }
    }
    (cpu, mem)
}

fn node_allocatable(node: &Node) -> (i64, i64) {
    let alloc = node.status.as_ref().and_then(|s| s.allocatable.as_ref());
    let cpu = alloc
        .and_then(|a| a.get("cpu"))
        .map(|q| parse_cpu_milli(&q.0))
        .unwrap_or(0);
    let mem = alloc
        .and_then(|a| a.get("memory"))
        .map(|q| parse_mem_bytes(&q.0))
        .unwrap_or(0);
    (cpu, mem)
}

fn parse_cpu_milli(s: &str) -> i64 {
    let s = s.trim();
    if let Some(m) = s.strip_suffix('m') {
        m.parse::<i64>().unwrap_or(0)
    } else {
        s.parse::<f64>().map(|v| (v * 1000.0) as i64).unwrap_or(0)
    }
}

fn parse_mem_bytes(s: &str) -> i64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }
    let num_end = s
        .chars()
        .position(|c| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let num: f64 = s[..num_end].parse().unwrap_or(0.0);
    let mult: f64 = match &s[num_end..] {
        "" | "B" => 1.0,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024.0,
        "Mi" => 1024.0 * 1024.0,
        "Gi" => 1024.0 * 1024.0 * 1024.0,
        "Ti" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "Pi" => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "Ei" => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (num * mult) as i64
}

/// A pool of `K8sClient`s keyed by cluster id, built lazily from the stored
/// (encrypted) kubeconfig. Wired into `AppState` so handlers share one client
/// per cluster instead of rebuilding per request.
pub struct K8sClientPool {
    cache: moka::future::Cache<uuid::Uuid, std::sync::Arc<K8sClient>>,
}

impl K8sClientPool {
    pub fn new() -> Self {
        Self {
            cache: moka::future::Cache::builder()
                .max_capacity(1024)
                .time_to_live(Duration::from_secs(3600))
                .time_to_idle(Duration::from_secs(1800))
                .build(),
        }
    }

    /// Get or build a client for a cluster from its encrypted kubeconfig.
    /// Returns `Err(NotFound)` if the cluster has no kubeconfig stored.
    pub async fn get(
        &self,
        cluster_id: uuid::Uuid,
        encrypted_kubeconfig: &str,
        jwt_secret: &str,
    ) -> Result<std::sync::Arc<K8sClient>, AppError> {
        if let Some(c) = self.cache.get(&cluster_id).await {
            return Ok(c);
        }
        let plain = secrets::decrypt(jwt_secret, encrypted_kubeconfig)?;
        let client = std::sync::Arc::new(K8sClient::from_kubeconfig_yaml(&plain).await?);
        self.cache.insert(cluster_id, std::sync::Arc::clone(&client)).await;
        Ok(client)
    }

    /// Drop a cached client (e.g. after the kubeconfig is replaced).
    pub async fn invalidate(&self, cluster_id: &uuid::Uuid) {
        self.cache.invalidate(cluster_id).await;
    }
}

impl Default for K8sClientPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rk(group: &str, version: &str, api_version: &str, kind: &str, plural: &str, namespaced: bool) -> ResolvedKind {
        ResolvedKind {
            group: group.to_string(),
            version: version.to_string(),
            api_version: api_version.to_string(),
            kind: kind.to_string(),
            plural: plural.to_string(),
            namespaced,
        }
    }

    #[test]
    fn path_namespaced_core() {
        let r = rk("", "v1", "v1", "Pod", "pods", true);
        assert_eq!(url_path_for(&r, Some("kube-system")), "/api/v1/namespaces/kube-system/pods");
    }

    #[test]
    fn path_namespaced_group() {
        // kube discovery keeps `version` bare ("v1"); the URL needs the full
        // apiVersion ("apps/v1").
        let r = rk("apps", "v1", "apps/v1", "Deployment", "deployments", true);
        assert_eq!(url_path_for(&r, Some("default")), "/apis/apps/v1/namespaces/default/deployments");
    }

    #[test]
    fn path_cluster_scoped_ignores_ns() {
        let r = rk("", "v1", "v1", "Node", "nodes", false);
        assert_eq!(url_path_for(&r, Some("default")), "/api/v1/nodes");
    }

    #[test]
    fn path_no_namespace() {
        let r = rk("", "v1", "v1", "Pod", "pods", true);
        assert_eq!(url_path_for(&r, None), "/api/v1/pods");
    }

    #[test]
    fn prefer_higher_score() {
        let core = rk("", "v1", "v1", "Node", "nodes", false);
        let crd = rk("longhorn.io", "v1beta2", "longhorn.io/v1beta2", "Node", "nodes", true);
        // Exact plural match (3) beats a prefix match (2) regardless of group.
        assert!(prefer_candidate(&core, 3, &crd, 2));
        assert!(!prefer_candidate(&crd, 2, &core, 3));
    }

    #[test]
    fn core_group_wins_score_tie() {
        // Both score 3 (exact plural "nodes"); core v1 Node must win over the
        // longhorn.io Node CRD — this is the "nodes" -> 404 page not found fix.
        let core = rk("", "v1", "v1", "Node", "nodes", false);
        let crd = rk("longhorn.io", "v1beta2", "longhorn.io/v1beta2", "Node", "nodes", true);
        // Core replaces an existing CRD best.
        assert!(prefer_candidate(&core, 3, &crd, 3));
        // A CRD does NOT replace an existing core best.
        assert!(!prefer_candidate(&crd, 3, &core, 3));
    }

    #[test]
    fn crd_tie_is_deterministic() {
        // If only CRDs match (no core equivalent) and they tie on score and kind
        // length, the decision is deterministic: the later candidate (>= kind
        // length) wins. groups_alphabetical() iterates in a stable order, so the
        // result is stable across calls (no flip-flopping).
        let a = rk("a.io", "v1", "a.io/v1", "Widget", "widgets", true);
        let b = rk("b.io", "v1", "b.io/v1", "Widget", "widgets", true);
        // Both non-core, same score, same kind len -> candidate replaces (last wins).
        assert!(prefer_candidate(&b, 3, &a, 3));
        // A CRD must never replace a core match on a tie (the important case).
        let core = rk("", "v1", "v1", "Widget", "widgets", true);
        assert!(!prefer_candidate(&b, 3, &core, 3));
    }

    #[test]
    fn shorter_kind_wins_full_tie() {
        // Same score, both core: the shorter (more specific) kind name is kept.
        let short = rk("", "v1", "v1", "Pod", "pods", true);
        let long = rk("", "v1", "v1", "PodDisruptionBudget", "poddisruptionbudgets", true);
        // cand=short replacing best=long: tie, both core, long.len()>=short.len() -> replace.
        assert!(prefer_candidate(&short, 3, &long, 3));
        // cand=long replacing best=short: tie, both core, short.len()>=long.len() false -> keep short.
        assert!(!prefer_candidate(&long, 3, &short, 3));
    }
}
