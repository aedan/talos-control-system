use std::net::SocketAddr;
use std::sync::Arc;


use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
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
use clap::Parser as _;

type AcmeChallengeStore = Arc<DashMap<String, String>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let argv: Vec<String> = std::env::args().collect();

    // One-shot tools before server boot
    if let Some(cmd) = argv.get(1) {
        if cmd == "migrate-sqlite-to-postgres" {
            init_tracing();
            let sqlite = std::env::var("TCS_SQLITE_PATH")
                .or_else(|_| std::env::var("TCS_DATABASE_SQLITE_PATH"))
                .unwrap_or_else(|_| "/var/lib/tcs/data.db".into());
            let pg = std::env::var("TCS_POSTGRES_URL")
                .or_else(|_| std::env::var("TCS_DATABASE_POSTGRES_URL"))
                .map_err(|_| "TCS_POSTGRES_URL is required for migrate-sqlite-to-postgres")?;
            info!(%sqlite, "Starting SQLite → Postgres migration");
            talos_control_system::db::migrate_sqlite_to_postgres::run(&sqlite, &pg).await?;
            info!("Migration complete");
            return Ok(());
        }
        if cmd == "--help" || cmd == "-h" || cmd == "help" {
            eprintln!(
                "tcs — Talos Control System\n\n\
                 Commands:\n\
                   (default)                   Start the control plane\n\
                   migrate-sqlite-to-postgres  Copy SQLite data into Postgres\n\
                                               env: TCS_SQLITE_PATH, TCS_POSTGRES_URL\n\
                   help                        Show this message\n\n\
                 CLI (kubectl-like, thin client over the TCS API):\n\
                   tcs login <email> <password>\n\
                   tcs clusters\n\
                   tcs get <kind> [name] [--ns] [-o table|wide|json|yaml]\n\
                   tcs describe <kind> <name> [--ns]\n\
                   tcs logs <pod> [--ns] [-f] [--tail N] [-c CONTAINER]\n\
                   tcs exec <pod> [--ns] [-c CONTAINER] [-t] -- <command...>\n\
                   tcs attach <pod> [--ns] [-c CONTAINER] [-t]\n\
                   tcs delete <kind> <name> [--ns] [-f]\n\
                   tcs scale <deployment> [--ns] --replicas N\n\
                   tcs cordon <node> | uncordon <node>\n\
                   tcs drain <node> [-f]\n\
                   tcs apply -f <file|->\n\n\
                 Global flags: --server URL --token JWT --cluster ID\n\
                 Auth: `tcs login`, TCS_TOKEN, or ~/.tcs/config\n"
            );
            return Ok(());
        }
    }

    // CLI verbs (tcs get / logs / exec / ...) vs. the server (bare `tcs`).
    //
    // The control plane is always started with no subcommand, so:
    //   * parse Ok + a subcommand present  -> run the CLI and exit
    //   * parse Ok + no subcommand         -> fall through and start the server
    //   * parse Err (help/version/typo)    -> clap already printed the message; exit
    match talos_control_system::cli::Cli::try_parse_from(&argv) {
        Ok(cli) => {
            if cli.command.is_some() {
                if let Err(e) = talos_control_system::cli::run_cli(cli).await {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
                return Ok(());
            }
        }
        Err(e) => {
            // `try_parse_from` does not print the message itself (unlike `parse_from`);
            // `exit()` renders help/errors to the right stream and sets the exit code.
            e.exit();
        }
    }

    use talos_control_system::auth::jwt::set_jwt_secret;
    init_tracing();
    talos_control_system::api::rest::handlers::record_start_time();

    // Install ring as the rustls CryptoProvider before any TLS operations.
    // Required because multiple crypto backends (ring, aws-lc-rs) are in the dep tree.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = Config::load()?;

    let default_secret = "talos-control-system-default-secret-change-in-production";
    if config.auth.jwt_secret == default_secret {
        let allow = std::env::var("TCS_ALLOW_INSECURE").ok().as_deref() == Some("1");
        if !allow {
            error!(
                "Refusing to start with default JWT secret. Set TCS_AUTH_JWT_SECRET (or auth.jwt_secret)                  or TCS_ALLOW_INSECURE=1 for local lab use only."
            );
            return Err("Insecure default JWT secret".into());
        }
        warn!("TCS_ALLOW_INSECURE=1: running with default JWT secret (lab only)");
    }

    set_jwt_secret(&config.auth.jwt_secret);

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
    use talos_control_system::db::SqlVal;
    let count = db_pool
        .fetch_scalar_i64("SELECT COUNT(*) FROM users", &[])
        .await?;
    if count == 0 {
        let default_password = std::env::var("TCS_DEFAULT_ADMIN_PASSWORD")
            .unwrap_or_else(|_| {
                let chars: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#";
                let mut rng = fastrand::Rng::new();
                let pw: String = (0..24).map(|_| chars[rng.u32(0..chars.len() as u32) as usize] as char).collect();
                warn!("No TCS_DEFAULT_ADMIN_PASSWORD set. Generated random admin password: {}", pw);
                pw
            });

        let password_hash = talos_control_system::auth::local::hash_password(&default_password)
            .expect("Failed to hash default admin password");

        let now = chrono::Utc::now();
        let admin_id = uuid::Uuid::new_v4();

        db_pool
            .execute(
                "INSERT INTO users (id, email, display_name, role, is_active, password_hash, auth_provider, ldap_dn, password_needs_change, last_login, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    SqlVal::Uuid(admin_id),
                    SqlVal::text("admin@tcs.local"),
                    SqlVal::text("TCS Administrator"),
                    SqlVal::text("admin"),
                    SqlVal::Bool(true),
                    SqlVal::text(password_hash),
                    SqlVal::text("local"),
                    SqlVal::OptText(None),
                    SqlVal::Bool(true),
                    SqlVal::OptDateTime(None),
                    SqlVal::DateTime(now),
                    SqlVal::DateTime(now),
                ],
            )
            .await?;

        info!(
            "Created default admin user: admin@tcs.local"
        );
        info!("Default admin password: {}", default_password);
        info!("IMPORTANT: Change the default admin password on first login");
    }

    let branding_manager = Arc::new(BrandingManager::new(&config.branding, &db_pool).await?);

    let data_dir = std::path::Path::new(&config.database.sqlite_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/var/lib/tcs".to_string());
    let siderolink_wg = talos_control_system::network::SiderolinkWg::init(
        &config.siderolink,
        &data_dir,
    );
    info!(
        instance_id = %talos_control_system::runtime::ha::instance_id(),
        wg_enabled = siderolink_wg.enabled(),
        "HA instance identity"
    );

    let event_bus = Arc::new(EventBus::new());
    let app_cache = AppCache::new();

    let tls_enabled = config.tls.enabled && config.tls.mode != TlsMode::Disabled;
    let acme_store: AcmeChallengeStore = Arc::new(DashMap::new());

    // Metal runtime: merge /var/lib/tcs/metal.toml overlay, own DHCP/PXE tasks
    let metal_merged = talos_control_system::network::MetalRuntime::load_merged(
        &config.metal,
        std::path::Path::new(&data_dir),
    );
    let metal_runtime = talos_control_system::network::MetalRuntime::start(
        db_pool.clone(),
        config.metal.clone(),
        &data_dir,
    );

    // Placeholder runtime; filled in run_with_tls before serving
    let state = AppState {
        config: Arc::new(config.clone()),
        db_pool: db_pool.clone(),
        branding: branding_manager,
        event_bus,
        cache: app_cache,
        siderolink_wg,
        tls_runtime: None,
        metal_runtime: Some(metal_runtime),
        k8s_pool: Arc::new(talos_control_system::integration::K8sClientPool::new()),
    };

    let _backup_sched = talos_control_system::runtime::spawn_backup_scheduler(
        db_pool.clone(),
        config.database.sqlite_path.clone(),
        config.auth.jwt_secret.clone(),
    );
    let _upgrade_sched = talos_control_system::runtime::spawn_upgrade_scheduler(
        db_pool.clone(),
        config.database.sqlite_path.clone(),
        config.auth.jwt_secret.clone(),
    );
    let _metal_sched = talos_control_system::runtime::spawn_metal_scheduler(
        db_pool.clone(),
        config.database.sqlite_path.clone(),
        config.auth.jwt_secret.clone(),
        metal_merged,
    );
    let _status_sched = talos_control_system::runtime::spawn_status_reconciler(
        db_pool.clone(),
        config.auth.jwt_secret.clone(),
    );

    if tls_enabled {
        run_with_tls(config, state, acme_store).await
    } else {
        run_without_tls(config, state).await
    }
}

async fn run_with_tls(
    config: Config,
    mut state: AppState,
    acme_store: AcmeChallengeStore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Start HTTP server FIRST (required for ACME HTTP-01 challenge validation)
    let http_addr: SocketAddr = format!("{}:80", config.server.bind_addr).parse()?;
    let http_app = build_http_redirect_router(acme_store.clone());
    let http_listener = tokio::net::TcpListener::bind(http_addr).await.unwrap();
    info!(
        addr = %http_addr,
        "Starting HTTP server (ACME challenges + HTTPS redirect)"
    );
    let http_handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(http_listener, http_app).await {
            error!(error = %e, "HTTP server error");
        }
    });

    // Now obtain certificate (HTTP challenge endpoint is live)
    let (cert_pem, key_pem) = if config.tls.mode == TlsMode::LetsEncrypt {
        if let Some(le) = &config.tls.letsencrypt {
            match talos_control_system::cert::acme::obtain_http01_certificate(
                &le.domains,
                &le.email,
                &acme_store,
            )
            .await
            {
                Ok((cert, key)) => {
                    info!("Let's Encrypt certificate obtained successfully");
                    (cert, key)
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Let's Encrypt issuance failed, falling back to self-signed certificate"
                    );
                    talos_control_system::cert::self_signed::generate_self_signed(&le.domains).await?
                }
            }
        } else {
            warn!("Let's Encrypt mode selected but not configured, falling back to self-signed");
            talos_control_system::cert::self_signed::generate_self_signed(&["localhost".to_string()]).await?
        }
    } else {
        load_certificates_from_config(&config.tls).await?
    };

    let data_dir = std::env::var("TCS_DATA_DIR").unwrap_or_else(|_| "/var/lib/tcs".into());

    let tls_runtime = Arc::new(
        talos_control_system::cert::TlsRuntime::new(
            cert_pem.clone(),
            key_pem.clone(),
            acme_store.clone(),
            config.tls.clone(),
            data_dir.clone(),
        )
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?,
    );
    state.tls_runtime = Some(tls_runtime.clone());

    let rest_app = create_rest_router(state.clone(), &config.branding);

    let https_addr: SocketAddr = format!("{}:443", config.server.bind_addr).parse()?;

    let renewal_config = config.clone();
    let renewal_acme = acme_store.clone();
    let renewal_handle = tokio::spawn(async move {
        if let Err(e) =
            talos_control_system::cert::start_cert_renewal_task(renewal_config, Some(renewal_acme))
                .await
        {
            error!(error = %e, "Certificate renewal task error");
        }
    });

    let rustls_config = tls_runtime.rustls_config();

    let https_handle = tokio::spawn(async move {
        info!(addr = %https_addr, "Starting HTTPS server (TLS, live reload enabled)");
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

async fn load_certificates_from_config(
    tls_config: &talos_control_system::config::tls::TlsConfig,
) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    match tls_config.mode {
        TlsMode::SelfSigned => {
            let domains = tls_config
                .self_signed
                .as_ref()
                .map(|c| c.domains.clone())
                .unwrap_or_else(|| vec!["localhost".to_string()]);
            Ok(talos_control_system::cert::self_signed::generate_self_signed(&domains).await?)
        }
        TlsMode::Provided => {
            let provided = tls_config.provided.as_ref().ok_or(
                "Provided TLS mode requires cert_path and key_path in config",
            )?;
            Ok(talos_control_system::cert::provided::load_provided_certs(
                &provided.cert_path,
                &provided.key_path,
            )
            .await?)
        }
        TlsMode::Disabled => Err("TLS mode is Disabled".into()),
        TlsMode::LetsEncrypt => {
            Err("Let's Encrypt handled separately in run_with_tls".into())
        }
    }
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
            term.recv().await;
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
