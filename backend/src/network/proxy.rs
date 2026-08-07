use std::sync::Arc;
use axum::http::{Request, Response, Uri, HeaderValue};
use axum::body::Body;
use tracing::{debug, warn};
use tokio::sync::RwLock;

use crate::auth::jwt::Claims;

pub struct KubernetesProxy {
    control_plane_endpoints: Arc<RwLock<Vec<String>>>,
}

impl KubernetesProxy {
    pub fn new() -> Self {
        Self {
            control_plane_endpoints: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn set_control_plane_endpoint(&self, endpoint: &str) {
        let mut endpoints = self.control_plane_endpoints.write().await;
        endpoints.clear();
        endpoints.push(endpoint.to_string());
        debug!(endpoint, "Control plane endpoint updated");
    }

    pub async fn proxy_request(
        &self,
        _req: Request<Body>,
        _claims: &Claims,
    ) -> Result<Response<Body>, axum::http::StatusCode> {
        let endpoints = self.control_plane_endpoints.read().await;
        if endpoints.is_empty() {
            warn!("No control plane endpoint configured");
            return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
        }

        Ok(Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from("{\"status\":\"ok\"}"))
            .unwrap())
    }

    pub fn build_jwt_for_proxy(&self, claims: &Claims) -> Option<HeaderValue> {
        match crate::auth::jwt::create_jwt(claims) {
            Ok(token) => HeaderValue::from_str(&format!("Bearer {}", token)).ok(),
            Err(_) => None,
        }
    }
}
