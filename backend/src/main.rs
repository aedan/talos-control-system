use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use dashmap::DashMap;
use tokio::signal;
use tracing::{error, info, warn};

use talos_control_system::api::rest::create_rest_router;
use talos_control_system::branding::BrandingManager;
use talos_control_system::config::{Config, TlsMode};
use talos_control_system::db::{init_pool, run_migrations};
use talos_control_system::runtime::cache::AppCache;
use talos_control_system::runtime::event::EventBus;
use talos_control_system::utils::logging::init_tracing;
use talos_control_system::utils::version::VERSION_INFO;
use talos_control_system::AppState;

type AcmeChallengeStore = Arc<DashMap<String, String>>;
type CertStore = Arc<tokio::sync::RwLock<(String, String)>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let config = Config::load()?;

    info!(
        version = VERSION_INFO.version,
        commit = VERSION_INFO.commit,
        build_time = VERSION_INFO.build_time,
        "Starting Talos Control System"
    );

    let db_pool = init_pool(&config.database).await?;
    info!(backend = ?config.database.backend, "Database pool initialized");

    run_migrations(&db_pool).await?;
    info!("Database migrations applied successfully");

    // Create default admin user if users table is empty
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&db_pool)
        .await?;
    if count.0 == 0 {
        let password: String = (0..16)
            .map(|_| {
                let b = rand::random::<u8>();
                b as char
            })
            .filter(|c: &char| c.is_ascii_alphanumeric())
            .take(16)
            .collect();

        let password_hash = talos_control_system::auth::local::hash_password(&password)
            .expect("Failed to hash default admin password");

        let now = chrono::Utc::now();
        let admin_id = uuid::Uuid::new_v4();

        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, is_active, password_hash, auth_provider, ldap_dn, password_needs_change, last_login, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(admin_id)
        .bind("admin@tcs.local")
        .bind("TCS Administrator")
        .bind("admin")
        .bind(true)
        .bind(&password_hash)
        .bind("local")
        .bind::<Option<String>>(None)
        .bind(true)
        .bind::<Option<chrono::DateTime<chrono::Utc>>>(None)
        .bind(&now)
        .bind(&now)
        .execute(&db_pool)
        .await?;

        info!(
            "Created default admin user: admin@tcs.local with password: {}",
            password
        );
        info!("IMPORTANT: Change the default admin password on first login");
    }

    let branding_manager = Arc::new(BrandingManager::new(&config.branding, &db_pool).await?);

    let event_bus = Arc::new(EventBus::new());
    let app_cache = AppCache::new();

    let state = AppState {
        config: Arc::new(config.clone()),
        db_pool,
        branding: branding_manager,
        event_bus,
        cache: app_cache,
    };

    let tls_enabled = config.tls.enabled && config.tls.mode != TlsMode::Disabled;
    let acme_store: AcmeChallengeStore = Arc::new(DashMap::new());

    if tls_enabled {
        run_with_tls(config, state, acme_store).await
    } else {
        run_without_tls(config, state).await
    }
}

async fn run_with_tls(
    config: Config,
    state: AppState,
    acme_store: AcmeChallengeStore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cert_store: CertStore = Arc::new(tokio::sync::RwLock::new((String::new(), String::new())));
    let (cert_pem, key_pem) = load_certificates(&config, &cert_store).await?;

    let rest_app = create_rest_router(state.clone(), &config.branding);
    let http_app = build_http_redirect_router(acme_store);

    let http_addr: SocketAddr = format!("{}:80", config.server.bind_addr).parse()?;
    let https_addr: SocketAddr = format!("{}:443", config.server.bind_addr).parse()?;

    let http_handle = tokio::spawn(async move {
        info!(
            addr = %http_addr,
            "Starting HTTP server (ACME challenges + HTTPS redirect)"
        );
        let listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
        if let Err(e) = axum::serve(listener, http_app).await {
            error!(error = %e, "HTTP server error");
        }
    });

    let rustls_config = RustlsConfig::from_pem(cert_pem.clone().into_bytes(), key_pem.clone().into_bytes())
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    let renewal_handle = tokio::spawn(async move {
        if let Err(e) = talos_control_system::cert::start_cert_renewal_task(config).await {
            error!(error = %e, "Certificate renewal task error");
        }
    });

    let https_handle = tokio::spawn(async move {
        info!(addr = %https_addr, "Starting HTTPS server (TLS)");
        if let Err(e) = axum_server::bind_rustls(https_addr, rustls_config)
            .serve(rest_app.into_make_service())
            .await
        {
            error!(error = %e, "HTTPS server error");
        }
    });

    let signals = run_signal_handlers();
    tokio::select! {
        res = signals => {
            info!("Shutdown signal received: {}", res);
        }
        _ = http_handle => {
            info!("HTTP server exited");
        }
        _ = https_handle => {
            info!("HTTPS server exited");
        }
        _ = renewal_handle => {
            info!("Certificate renewal task exited");
        }
    }

    info!("Shutting down gracefully...");
    Ok(())
}

async fn run_without_tls(
    config: Config,
    state: AppState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rest_app = create_rest_router(state.clone(), &config.branding);
    let rest_addr: SocketAddr =
        format!("{}:{}", config.server.bind_addr, config.server.http_port)
            .parse()
            .expect("Invalid HTTP bind address");

    let rest_handle = tokio::spawn(async move {
        info!(addr = %rest_addr, "Starting REST server (no TLS)");
        let listener = tokio::net::TcpListener::bind(rest_addr).await.unwrap();
        if let Err(e) = axum::serve(listener, rest_app).await {
            error!(error = %e, "REST server error");
        }
    });

    let signals = run_signal_handlers();
    tokio::select! {
        res = signals => {
            info!("Shutdown signal received: {}", res);
        }
        _ = rest_handle => {
            info!("REST server exited");
        }
    }

    info!("Shutting down gracefully...");
    Ok(())
}

async fn load_certificates(
    config: &Config,
    cert_store: &CertStore,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    let (cert_pem, key_pem) = match config.tls.mode {
        TlsMode::SelfSigned => {
            let domains = config
                .tls
                .self_signed
                .as_ref()
                .map(|c| c.domains.clone())
                .unwrap_or_else(|| vec!["localhost".to_string()]);
            talos_control_system::cert::self_signed::generate_self_signed(&domains).await?
        }
        TlsMode::Provided => {
            let provided = config.tls.provided.as_ref().ok_or(
                "Provided TLS mode requires cert_path and key_path in config",
            )?;
            talos_control_system::cert::provided::load_provided_certs(
                &provided.cert_path,
                &provided.key_path,
            )
            .await?
        }
        TlsMode::LetsEncrypt => {
            warn!(
                "Let's Encrypt mode: using self-signed placeholder certificate; \
                 real ACME issuance will be handled by the cert manager"
            );
            let domains = config
                .tls
                .letsencrypt
                .as_ref()
                .map(|c| c.domains.clone())
                .unwrap_or_else(|| vec!["localhost".to_string()]);
            talos_control_system::cert::self_signed::generate_self_signed(&domains).await?
        }
        TlsMode::Disabled => {
            return Err("TLS mode is Disabled".into());
        }
    };

    *cert_store.write().await = (cert_pem.clone(), key_pem.clone());
    Ok((cert_pem, key_pem))
}

fn build_http_redirect_router(acme_store: AcmeChallengeStore) -> Router {
    Router::new()
        .route("/.well-known/acme-challenge/*token", get(acme_challenge_handler))
        .fallback(https_redirect)
        .with_state(acme_store)
}

async fn acme_challenge_handler(
    State(store): State<AcmeChallengeStore>,
    Path(token): Path<String>,
) -> (StatusCode, String) {
    if let Some(entry) = store.get(&token) {
        (StatusCode::OK, entry.value().clone())
    } else {
        (
            StatusCode::NOT_FOUND,
            "Challenge token not found".to_string(),
        )
    }
}

async fn https_redirect(request: Request) -> Redirect {
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");

    let uri = request.uri();
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let redirect_url = format!("https://{host}{}{query}", uri.path());
    Redirect::permanent(&redirect_url)
}

async fn run_signal_handlers() -> String {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        "Ctrl+C".to_string()
    };

    let terminate = async {
        #[cfg(unix)]
        {
            let mut term = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to listen for SIGTERM");
            term.recv();
        }
        #[cfg(not(unix))]
        {
            futures::future::pending::<()>().await;
        }
        "SIGTERM".to_string()
    };

    tokio::select! {
        sig = ctrl_c => sig,
        sig = terminate => sig,
    }
}
