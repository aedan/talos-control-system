use axum::Router;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::static_server;
use crate::AppState;
use crate::config::BrandingConfig;

pub mod handlers;
pub mod middleware;

pub fn create_rest_router(state: AppState, _branding: &BrandingConfig) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request());

    // Public: health, auth entrypoints, branding for login page
    let public_routes = Router::new()
        .route("/health", get(handlers::health_check))
        .route("/auth/login", post(handlers::login))
        .route("/auth/logout", post(handlers::logout))
        .route("/auth/token", post(handlers::refresh_token))
        .route("/auth/providers", get(handlers::get_auth_providers))
        .route("/auth/oidc", get(handlers::oidc_authorize))
        .route("/auth/oidc/callback", get(handlers::oidc_callback))
        .route("/auth/saml/metadata", get(handlers::saml_metadata))
        .route("/auth/saml/login", get(handlers::saml_login))
        .route("/auth/saml/acs", post(handlers::saml_acs))
        .route("/branding", get(handlers::get_branding))
        .route("/branding/css", get(handlers::get_branding_css))
        .route("/branding/logo", get(handlers::get_logo))
        .route("/branding/favicon", get(handlers::get_favicon))
        .route(
            "/branding/tenants/:tenant_id",
            get(handlers::get_tenant_branding),
        )
        .route("/siderolink/register", post(handlers::siderolink_register));

    // Metrics intentionally not on public API; use /metrics on metrics_port when exposed separately.
    // Kept behind auth for alpha simplicity:
    let protected_routes = Router::new()
        .route("/metrics", get(handlers::get_metrics))
        .route("/auth/me", get(handlers::get_user_info))
        .route("/auth/password", post(handlers::change_password))
        .route("/users", get(handlers::list_users))
        .route("/users", post(handlers::create_user))
        .route("/users/:id", get(handlers::get_user))
        .route("/users/:id", put(handlers::update_user))
        .route("/users/:id", delete(handlers::delete_user))
        .route("/settings/certificates/status", get(handlers::get_cert_status))
        .route("/settings/certificates/config", put(handlers::update_cert_config))
        .route("/settings/certificates/renew", post(handlers::renew_certificate))
        .route("/settings/auth/config", get(handlers::get_auth_config))
        .route("/settings/auth/config", put(handlers::update_auth_config))
        .route("/branding", put(handlers::update_branding))
        .route(
            "/branding/tenants/:tenant_id",
            put(handlers::put_tenant_branding),
        )
        .route("/clusters", get(handlers::list_clusters))
        .route("/clusters", post(handlers::create_cluster))
        .route("/clusters/import", post(handlers::import_cluster))
        .route("/clusters/import/preview", post(handlers::preview_import))
        .route(
            "/clusters/generate-config",
            post(handlers::generate_cluster_config),
        )
        .route(
            "/provision/artifacts",
            get(handlers::list_provision_artifacts),
        )
        .route(
            "/provision/artifacts/:id",
            get(handlers::get_provision_artifact),
        )
        .route("/fleets/upgrades", post(handlers::start_fleet_upgrade))
        .route("/upgrade-jobs", get(handlers::list_upgrade_jobs))
        .route("/upgrade-jobs/:id", get(handlers::get_upgrade_job))
        .route(
            "/upgrade-jobs/:id/cancel",
            post(handlers::cancel_upgrade_job),
        )
        .route("/siderolink/peers", get(handlers::siderolink_peers))
        .route(
            "/siderolink/tokens",
            get(handlers::list_siderolink_tokens).post(handlers::create_siderolink_token),
        )
        .route("/clusters/:id", get(handlers::get_cluster))
        .route("/clusters/:id", put(handlers::update_cluster))
        .route("/clusters/:id", delete(handlers::delete_cluster))
        .route("/clusters/:id/talosconfig", put(handlers::set_cluster_talosconfig))
        .route("/clusters/:id/refresh", post(handlers::refresh_cluster))
        .route("/clusters/:id/talos/test", post(handlers::test_cluster_talos))
        .route(
            "/clusters/:id/talos/versions",
            post(handlers::probe_cluster_versions),
        )
        .route(
            "/clusters/:id/upgrade",
            post(handlers::start_cluster_upgrade),
        )
        .route(
            "/clusters/:id/access",
            get(handlers::list_cluster_access).put(handlers::upsert_cluster_access),
        )
        .route(
            "/clusters/:id/access/:user_id",
            delete(handlers::delete_cluster_access),
        )
        .route("/machines", get(handlers::list_machines))
        .route("/machines/:id", get(handlers::get_machine))
        .route("/machines/:id", put(handlers::update_machine))
        .route("/machines/:id", delete(handlers::delete_machine))
        .route("/machines/:id/reboot", post(handlers::reboot_machine))
        .route("/machines/:id/upgrade", post(handlers::upgrade_machine))
        .route("/machines/:id/version", get(handlers::get_machine_version))
        .route("/machines/:id/services", get(handlers::get_machine_services))
        .route("/machines/:id/hostname", get(handlers::get_machine_hostname))
        // Cluster sub-routes
        .route("/clusters/:id/nodes", get(handlers::get_cluster_nodes))
        .route("/clusters/:id/machines", get(handlers::get_cluster_machines))
        .route("/clusters/:id/config", get(handlers::list_config_patches))
        .route("/clusters/:id/config", post(handlers::create_config_patch))
        .route("/clusters/:id/config/apply", post(handlers::apply_cluster_config))
        .route("/clusters/:id/config/:patch_id", delete(handlers::delete_config_patch))
        .route(
            "/clusters/:id/backups/schedule",
            put(handlers::set_backup_schedule),
        )
        .route("/clusters/:id/backups", get(handlers::list_cluster_backups))
        .route("/clusters/:id/backups", post(handlers::create_cluster_backup))
        .route("/clusters/:id/backups/:backup_id", get(handlers::download_cluster_backup))
        .route(
            "/clusters/:id/backups/:backup_id/download",
            get(handlers::download_cluster_backup),
        )
        .route(
            "/clusters/:id/backups/:backup_id/restore",
            post(handlers::restore_cluster_backup),
        )
        .route("/clusters/:id/backups/:backup_id", delete(handlers::delete_cluster_backup))
        // Machine classes
        .route("/machine-classes", get(handlers::list_machine_classes))
        .route("/machine-classes", post(handlers::create_machine_class))
        .route("/machine-classes/:id", get(handlers::get_machine_class))
        .route("/machine-classes/:id", put(handlers::update_machine_class))
        .route("/machine-classes/:id", delete(handlers::delete_machine_class))
        // Settings
        .route("/settings/audit-logs", get(handlers::get_audit_logs))
        .route("/settings/audit-logs", delete(handlers::clear_audit_logs))
        .route("/settings/system/info", get(handlers::get_system_info))
        .layer(from_fn_with_state(
            state.clone(),
            middleware::rbac_middleware,
        ));

    let api_routes = Router::new()
        .merge(public_routes)
        .merge(protected_routes);

    Router::new()
        .nest("/api", api_routes)
        .fallback(static_server::serve_static)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
