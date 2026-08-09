use axum::body::Body;
use axum::extract::Request;
use axum::http::StatusCode;

use crate::auth::jwt::verify_jwt;
use crate::auth::rbac::{Action, Resource};

pub struct RbacClaims {
    pub sub: String,
    pub role: String,
}

pub fn extract_claims_from_request(request: &Request) -> Option<RbacClaims> {
    let headers = request.headers();

    let auth_header = headers.get(axum::http::header::AUTHORIZATION)?;
    let auth_str = auth_header.to_str().ok()?;
    let token = auth_str.strip_prefix("Bearer ")?;

    let token_data = verify_jwt(token).ok()?;
    let claims = token_data.claims;

    Some(RbacClaims {
        sub: claims.sub,
        role: claims.role,
    })
}

pub fn map_route_to_resource(uri: &str) -> Option<Resource> {
    if uri.starts_with("/api/clusters") {
        Some(Resource::Cluster)
    } else if uri.starts_with("/api/machines") {
        Some(Resource::Machine)
    } else if uri.starts_with("/api/machine-classes") {
        Some(Resource::MachineSet)
    } else if uri.starts_with("/api/machine-sets") || uri.starts_with("/api/machinesets") {
        Some(Resource::MachineSet)
    } else if uri.starts_with("/api/settings") {
        Some(Resource::Config)
    } else if uri.starts_with("/api/branding") {
        Some(Resource::Branding)
    } else if uri.starts_with("/api/users") {
        Some(Resource::User)
    } else if uri.starts_with("/api/auth/me") || uri.starts_with("/api/auth/password") {
        Some(Resource::User)
    } else {
        None
    }
}

pub fn map_method_to_action(method: &axum::http::Method) -> Action {
    match *method {
        axum::http::Method::GET => Action::Read,
        axum::http::Method::POST => Action::Create,
        axum::http::Method::PUT | axum::http::Method::PATCH => Action::Update,
        axum::http::Method::DELETE => Action::Delete,
        _ => Action::Read,
    }
}

pub async fn rbac_middleware(request: Request, next: axum::middleware::Next) -> axum::response::Response {
    let uri = request.uri().path().to_string();
    let method = request.method().clone();

    let claims = match extract_claims_from_request(&request) {
        Some(c) => c,
        None => {
            return axum::response::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":"Invalid or expired token"}"#))
                .unwrap();
        }
    };

    if let Some(resource) = map_route_to_resource(&uri) {
        let action = map_method_to_action(&method);

        if !crate::auth::rbac::check_permission_by_role(&claims.role, &resource, &action) {
            tracing::warn!(
                user = %claims.sub,
                role = %claims.role,
                resource = ?resource,
                action = ?action,
                uri = %uri,
                "Permission denied"
            );

            return axum::response::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"error":"Forbidden: insufficient permissions"}"#))
                .unwrap();
        }
    }

    next.run(request).await
}
