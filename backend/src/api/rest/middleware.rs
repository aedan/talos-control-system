use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::auth::jwt::verify_jwt;
use crate::auth::rbac::{
    check_permission_by_role, min_role_for_action, role_at_least, Action, Resource,
};
use crate::db::repos::{self, cluster_access};
use crate::AppState;

pub struct RbacClaims {
    pub sub: String,
    pub role: String,
}

pub fn extract_claims_from_request(request: &Request) -> Option<RbacClaims> {
    // Prefer the Authorization header; fall back to a `?token=` query param for
    // WebSocket/SSE clients that cannot set custom headers.
    let token = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
        .or_else(|| {
            let query = request.uri().query()?;
            query
                .split('&')
                .find_map(|pair| pair.strip_prefix("token="))
                .map(|s| s.to_string())
        })?;

    let token_data = verify_jwt(&token).ok()?;
    let claims = token_data.claims;

    Some(RbacClaims {
        sub: claims.sub,
        role: claims.role,
    })
}

pub fn map_route_to_resource(uri: &str) -> Option<Resource> {
    // The iLO console asset-proxy and KVM-WebSocket routes are hit by the
    // iLO's own <script>/WS (no Authorization header). They are gated by the
    // unguessable `ilo_…` session id instead, so exempt them from RBAC. The
    // console `session` mint and `sol` WS keep normal machine auth.
    if uri.contains("/console/") && console_segment_is_session(uri) {
        return None;
    }
    if uri.starts_with("/api/clusters") {
        Some(Resource::Cluster)
    } else if uri.starts_with("/api/machines") {
        Some(Resource::Machine)
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

/// True if the path segment after `/console/` is an `ilo_…` session id (asset
/// proxy or KVM WS), i.e. a route that must bypass RBAC auth.
fn console_segment_is_session(uri: &str) -> bool {
    match uri.rsplit_once("/console/") {
        Some((_, rest)) => rest.split('/').next().unwrap_or("").starts_with("ilo_"),
        None => false,
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

/// Extract cluster UUID from `/api/clusters/{uuid}/...` paths.
pub fn extract_cluster_id_from_path(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/api/clusters/")?;
    let id_str = rest.split('/').next()?;
    if id_str.is_empty() || id_str == "import" {
        return None;
    }
    Uuid::parse_str(id_str).ok()
}

/// Extract machine UUID from `/api/machines/{uuid}/...`.
pub fn extract_machine_id_from_path(path: &str) -> Option<Uuid> {
    let rest = path.strip_prefix("/api/machines/")?;
    let id_str = rest.split('/').next()?;
    if id_str.is_empty() {
        return None;
    }
    Uuid::parse_str(id_str).ok()
}

fn forbidden(msg: &str) -> axum::response::Response {
    axum::response::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"error":"{}"}}"#, msg)))
        .unwrap()
}

pub async fn rbac_middleware(
    State(state): State<AppState>,
    request: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let uri = request.uri().path().to_string();
    let method = request.method().clone();

    // Session-gated console routes (iLO asset proxy / KVM WS) bypass RBAC entirely:
    // they carry no Authorization header (iLO's own JS makes the calls) and are
    // authenticated by the unguessable `ilo_…` session id inside the path.
    if console_segment_is_session(&uri) {
        return next.run(request).await;
    }

    // The iDRAC auto-login redeem endpoint is called cross-origin by the TCS
    // browser extension (no TCS auth header); it self-authenticates via the
    // single-use autologin JWT in the request body. Bypass RBAC here.
    if uri.contains("/console/idrac-autologin/redeem") {
        return next.run(request).await;
    }

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

        // Global capability check first (role can touch this resource type at all).
        if !check_permission_by_role(&claims.role, &resource, &action) {
            tracing::warn!(
                user = %claims.sub,
                role = %claims.role,
                resource = ?resource,
                action = ?action,
                uri = %uri,
                "Permission denied (global role)"
            );
            return forbidden("Forbidden: insufficient permissions");
        }

        // Per-cluster scope for cluster- and machine-scoped routes.
        if matches!(resource, Resource::Cluster | Resource::Machine) {
            if let Some(resp) =
                enforce_cluster_scope(&state, &claims, &uri, &resource, &action).await
            {
                return resp;
            }
        }
    }

    next.run(request).await
}

async fn enforce_cluster_scope(
    state: &AppState,
    claims: &RbacClaims,
    uri: &str,
    resource: &Resource,
    action: &Action,
) -> Option<axum::response::Response> {
    // Global admins skip membership checks.
    if claims.role == "admin" {
        return None;
    }

    let user = match repos::user::get_by_email(&state.db_pool, &claims.sub).await {
        Ok(Some(u)) => u,
        Ok(None) => return Some(forbidden("User not found")),
        Err(_) => {
            return Some(
                axum::response::Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"error":"Failed to load user"}"#))
                    .unwrap(),
            );
        }
    };

    let cluster_id = if let Some(cid) = extract_cluster_id_from_path(uri) {
        Some(cid)
    } else if matches!(resource, Resource::Machine) {
        if let Some(mid) = extract_machine_id_from_path(uri) {
            match repos::machine::get(&state.db_pool, mid).await {
                Ok(Some(m)) => m.cluster_id,
                Ok(None) => None,
                Err(_) => {
                    return Some(
                        axum::response::Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .header(axum::http::header::CONTENT_TYPE, "application/json")
                            .body(Body::from(r#"{"error":"Failed to load machine"}"#))
                            .unwrap(),
                    );
                }
            }
        } else {
            // GET /api/machines list — filter in handler if needed; allow if global role ok.
            return None;
        }
    } else {
        // GET/POST /api/clusters list/create — no specific cluster.
        return None;
    };

    let Some(cluster_id) = cluster_id else {
        // Machine without cluster: allow if global role already passed.
        return None;
    };

    match cluster_access::effective_cluster_role(
        &state.db_pool,
        user.id,
        &claims.role,
        cluster_id,
    )
    .await
    {
        Ok(Some(eff)) => {
            let need = min_role_for_action(action);
            if !role_at_least(&eff, need) {
                tracing::warn!(
                    user = %claims.sub,
                    effective = %eff,
                    need = %need,
                    cluster = %cluster_id,
                    uri = %uri,
                    "Permission denied (cluster membership)"
                );
                return Some(forbidden(
                    "Forbidden: insufficient role on this cluster",
                ));
            }
            None
        }
        Ok(None) => {
            tracing::warn!(
                user = %claims.sub,
                cluster = %cluster_id,
                uri = %uri,
                "Permission denied (no cluster access)"
            );
            Some(forbidden("Forbidden: no access to this cluster"))
        }
        Err(e) => Some(
            axum::response::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"error":"Access check failed: {}"}}"#,
                    e
                )))
                .unwrap(),
        ),
    }
}
