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
    #[serde(default)]
    pub client_certificate_data: Option<String>,
    #[serde(default)]
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

/// Extract auth token or credentials from kubeconfig for the active context
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
    let mut token = String::new();

    if let Some(user) = user {
        if let Some(b64) = &user.user.client_certificate_data {
            if let Ok(data) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64) {
                cert = data;
            }
        }
        if let Some(t) = &user.user.token {
            token = t.clone();
        }
    }

    (Vec::new(), cert, token)
}

/// Build a kube::Client from parsed kubeconfig
async fn build_client_from_kubeconfig(config: &Kubeconfig) -> Result<kube::Client, AppError> {
    let context_name = if config.current_context.is_empty() {
        config.contexts.first().map(|c| c.name.clone()).ok_or_else(|| {
            AppError::InvalidInput("No context available in kubeconfig".to_string())
        })?
    } else {
        config.current_context.clone()
    };

    let context = config.contexts.iter()
        .find(|c| c.name == context_name)
        .ok_or_else(|| {
            AppError::InvalidInput(format!("Context '{}' not found", context_name))
        })?;

    let cluster_name = context.context.cluster.clone();
    let user_name = context.context.user.clone();

    let cluster = config.clusters.iter()
        .find(|c| c.name == cluster_name)
        .ok_or_else(|| {
            AppError::InvalidInput(format!("Cluster '{}' not found", cluster_name))
        })?;

    let server = cluster.cluster.server.clone();

    // Build kube::config from the parsed data
    let ca_data = extract_ca_data(config);
    let (_client_ca, client_cert, token) = extract_auth(config);

    let kubeconfig_yaml = serde_yaml::to_string(config).map_err(|e| {
        AppError::InvalidInput(format!("Failed to serialize kubeconfig: {}", e))
    })?;

    let ca_data = extract_ca_data(config);
    let (_client_ca, client_cert, token) = extract_auth(config);

    let mut kube_config = kube::config::Config::new(
        cluster.cluster.server.parse().unwrap()
    );

    if !ca_data.is_empty() {
        kube_config.root_cert = Some(vec![ca_data]);
    }

    if !client_cert.is_empty() {
        kube_config.auth_info.client_certificate_data = Some(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &client_cert));
    }

    if !token.is_empty() {
        kube_config.auth_info.token = Some(token.into());
    }

    kube::Client::try_from(kube_config).map_err(|e| {
        AppError::Internal(format!("Failed to build kube client: {}", e))
    })
}

pub struct KubernetesClient {
    cluster_id: uuid::Uuid,
    endpoint: String,
}

impl KubernetesClient {
    pub async fn new(cluster_id: uuid::Uuid, endpoint: String, _ca_data: Vec<u8>, _token: String) -> Result<Self, AppError> {
        info!(cluster_id = %cluster_id, endpoint = %endpoint, "Kubernetes client initialized");
        Ok(Self { cluster_id, endpoint })
    }

    pub async fn apply_manifest(&self, manifest: &str) -> Result<(), AppError> {
        tracing::debug!(manifest_len = manifest.len(), "Applying manifest");
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

        self.cache.insert(cluster_id, Arc::clone(&client));
        Ok(client)
    }

    pub fn invalidate(&self, cluster_id: &uuid::Uuid) {
        self.cache.invalidate(cluster_id);
    }
}

/// Discover an existing cluster by parsing kubeconfig and querying nodes
pub async fn discover_cluster_from_kubeconfig(kubeconfig_yaml: &str) -> Result<DiscoveredCluster, AppError> {
    // Parse kubeconfig
    let config = parse_kubeconfig(kubeconfig_yaml)?;

    // Build kube client
    let client = build_client_from_kubeconfig(&config).await?;

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
