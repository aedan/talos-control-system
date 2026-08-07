use std::net::SocketAddr;
use std::sync::Arc;

use tokio::signal;
use tracing::{info, error};

use talos_control_system::config::Config;
use talos_control_system::db::{init_pool, run_migrations};
use talos_control_system::api::rest::create_rest_router;
use talos_control_system::branding::BrandingManager;
use talos_control_system::runtime::event::EventBus;
use talos_control_system::runtime::cache::AppCache;
use talos_control_system::utils::logging::init_tracing;
use talos_control_system::AppState;
use talos_control_system::utils::version::VERSION_INFO;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let config = Config::load()?;

    info!(version = VERSION_INFO.version, commit = VERSION_INFO.commit,
          build_time = VERSION_INFO.build_time, "Starting Talos Control System");

    let db_pool = init_pool(&config.database).await?;
    info!(backend = ?config.database.backend, "Database pool initialized");

    run_migrations(&db_pool).await?;
    info!("Database migrations applied successfully");

    let branding_manager = Arc::new(
        BrandingManager::new(&config.branding, &db_pool).await?
    );

    let event_bus = Arc::new(EventBus::new());
    let app_cache = AppCache::new();

    let state = talos_control_system::AppState {
        config: Arc::new(config.clone()),
        db_pool,
        branding: branding_manager,
        event_bus,
        cache: app_cache,
    };

    let rest_app = create_rest_router(state.clone(), &config.branding);
    let rest_addr: SocketAddr = format!("{}:{}", config.server.bind_addr, config.server.http_port)
        .parse()
        .expect("Invalid HTTP bind address");

    let rest_handle = tokio::spawn(async move {
        info!(addr = %rest_addr, "Starting REST server");
        if let Err(e) = axum::serve(
            tokio::net::TcpListener::bind(rest_addr).await.unwrap(),
            rest_app,
        )
        .await
        {
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


