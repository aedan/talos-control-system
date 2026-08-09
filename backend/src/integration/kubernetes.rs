use std::sync::Arc;
use std::time::Duration;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::AppError;

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
