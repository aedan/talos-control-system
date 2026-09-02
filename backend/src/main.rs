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
use talos_control_system::config::Config;
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
            print_help();
            return Ok(());
        }
        // Tool passthrough: `tcs [global flags] kubectl|helm|talosctl <raw args...>`.
        // Scanned past the global flags (which consume a value) so the tool verb
        // can appear anywhere before the tool's own args.
        if let Some((tool_name, tool_pos)) = find_tool_verb(&argv) {
            return run_tool_passthrough(&tool_name, tool_pos, &argv).await;
        }
    }

    // The binary is a CLI by default. The control plane is only started when
    // invoked explicitly with `tcs serve` (the systemd unit does this).
    //
    //   * parse Ok + a subcommand present  -> run that command (serve boots the server)
    //   * parse Ok + no subcommand         -> print help (CLI is the default)
    //   * parse Err (help/version/typo)    -> clap already printed the message; exit
    match talos_control_system::cli::Cli::try_parse_from(&argv) {
        Ok(cli) => {
            if let Some(ref command) = cli.command {
                if matches!(command, talos_control_system::cli::Command::Serve) {
                    return run_server().await;
                }
                if let Err(e) = talos_control_system::cli::run_cli(cli).await {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
                return Ok(());
            }
            print_help();
            return Ok(());
        }
        Err(e) => {
            // `try_parse_from` does not print the message itself (unlike `parse_from`);
            // `exit()` renders help/errors to the right stream and sets the exit code.
            e.exit();
        }
    }
}

/// Tool passthrough: `tcs [global flags] kubectl|helm|talosctl <raw args...>`.
///
/// Long-form TCS global flags (`--server`, `--token`, `--cluster`) are pulled
/// out wherever they appear; everything else is passed verbatim to the real
/// binary. Short flags belong to the tool itself (e.g. `kubectl -c`), so they
/// are never consumed.
async fn run_tool_passthrough(
    tool_name: &str,
    tool_pos: usize,
    argv: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use talos_control_system::cli::client::Client;
    use talos_control_system::cli::commands::{require_cluster, tool};

    let mut server: Option<String> = None;
    let mut token: Option<String> = None;
    let mut cluster: Option<String> = None;
    let mut tool_args: Vec<String> = Vec::new();
    let mut i = 1usize;
    while i < argv.len() {
        if i == tool_pos {
            // Everything from here on (after the tool verb) is the tool's argv.
            tool_args = argv[i + 1..].to_vec();
            break;
        }
        match argv[i].as_str() {
            "--server" | "-s" => {
                i += 1;
                server = argv.get(i).cloned();
            }
            "--token" | "-t" => {
                i += 1;
                token = argv.get(i).cloned();
            }
            "--cluster" | "-c" => {
                i += 1;
                cluster = argv.get(i).cloned();
            }
            _ => {}
        }
        i += 1;
    }

    let client = Client::new(server.as_deref(), token.as_deref())?;
    let cluster_id = require_cluster(&client, cluster.as_deref()).await?;
    tool::run(&client, &cluster_id, tool_name, &tool_args).await?;
    Ok(())
}

/// Locate the tool verb (kubectl/helm/talosctl) in argv, skipping the TCS
/// global flags (which consume a value). Returns the verb and its index.
fn find_tool_verb(argv: &[String]) -> Option<(String, usize)> {
    let mut i = 1usize;
    while i < argv.len() {
        match argv[i].as_str() {
            "kubectl" | "helm" | "talosctl" => return Some((argv[i].clone(), i)),
            // Global flags that take a following value: skip the value too.
            "--server" | "-s" | "--token" | "-t" | "--cluster" | "-c" => i += 2,
            _ => i += 1,
        }
    }
    None
}

fn print_help() {
    eprintln!(
        "tcs — Talos Control System\n\n\
         Server:\n\
           tcs serve                       Start the control plane (systemd unit)\n\
           tcs migrate-sqlite-to-postgres  Copy SQLite data into Postgres\n\
                                           env: TCS_SQLITE_PATH, TCS_POSTGRES_URL\n\
           tcs help                        Show this message\n\n\
         CLI (kubectl-like, thin client over the TCS API):\n\
           tcs login [email] [password]   (or --email/--password; prompts if omitted)\n\
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
            tcs apply -f <file|->\n\
             tcs kubectl <args...>        (real kubectl, run server-side)\n\
             tcs helm <args...>           (real helm, run server-side)\n\
             tcs talosctl <args...>       (real talosctl, run server-side)\n\
             tcs kubeconfig               Print the cluster's stored kubeconfig\n\
             tcs talosconfig              Print the cluster's stored talosconfig\n\
             (fetched over the API with your token; auto re-login if expired)\n\n\
         Global flags: --server URL --token JWT --cluster ID\n\
         Auth: `tcs login`, TCS_TOKEN, or ~/.tcs/config\n"
    );
}

/// Boot the TCS control-plane server (invoked via `tcs serve`).
async fn run_server() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

    // Re-register known Siderolink peers to the freshly-created tcs-sl0. The
    // kernel WG device is wiped on restart, so peers live only until a node
    // re-provisions; nodes that keep their cached provisionData and just retry
    // the existing handshake would otherwise find no peer. (Best-effort: peers
    // table may be empty on first boot.)
    {
        use talos_control_system::db::repos::siderolink as sl_repos;
        match sl_repos::list_peers(&db_pool).await {
            Ok(peers) if !peers.is_empty() => {
                let mapped: Vec<(String, String)> = peers
                    .iter()
                    .map(|p| (p.public_key.clone(), p.assigned_ip.clone()))
                    .collect();
                siderolink_wg.reapply_peers(&mapped);
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "Siderolink: could not list peers for boot re-apply"),
        }
    }

    let event_bus = Arc::new(EventBus::new());
    let app_cache = AppCache::new();

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

    let _status_sched = talos_control_system::runtime::spawn_status_reconciler(
        db_pool.clone(),
        config.auth.jwt_secret.clone(),
    );

    run_server_all(config, state, acme_store).await
}

/// Always-on server: binds :443 (HTTPS, via a live-reloadable cert) and :80
/// (HTTP: ACME challenges + redirect-to-HTTPS). TCS only ever listens on 80 and
/// 443 — there is no separate `http_port` listener (alpha: no backward-compat
/// escape hatch needed).
///
/// A `TlsRuntime` always exists so enabling/switching certs from the UI works
/// live from any starting mode (including "disabled", which falls back to a
/// generated self-signed cert so :443 is reachable immediately).
async fn run_server_all(
    config: Config,
    mut state: AppState,
    acme_store: AcmeChallengeStore,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind = config.server.bind_addr.clone();
    let data_dir = std::env::var("TCS_DATA_DIR").unwrap_or_else(|_| "/var/lib/tcs".into());

    // Resolve the initial certificate. Let's Encrypt is best-effort (needs
    // :80 reachable from the internet); on failure fall back to self-signed so
    // the HTTPS listener is always up. "disabled" also falls back to
    // self-signed.
    let (cert_pem, key_pem, effective_mode, effective_domains, initial_note) =
        resolve_initial_certificate(&config, &acme_store).await?;
    info!(%initial_note, "Initial TLS certificate ready");

    // Reflect the EFFECTIVE cert in the live runtime so the Certificates UI
    // shows what's actually serving :443 (e.g. "self-signed" even when the
    // config says "disabled").
    let mut effective_tls = config.tls.clone();
    effective_tls.mode = effective_mode.clone();
    if matches!(effective_tls.mode, talos_control_system::config::TlsMode::SelfSigned) {
        let domains = if effective_domains.is_empty() {
            vec!["localhost".to_string()]
        } else {
            effective_domains.clone()
        };
        effective_tls.self_signed = Some(talos_control_system::config::SelfSignedConfig { domains });
    }

    let tls_runtime = Arc::new(
        talos_control_system::cert::TlsRuntime::new(
            cert_pem.clone(),
            key_pem.clone(),
            acme_store.clone(),
            effective_tls,
            data_dir.clone(),
        )
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?,
    );
    state.tls_runtime = Some(tls_runtime.clone());

    // The app we serve (same router as before; branding applied).
    let rest_app = create_rest_router(state.clone(), &config.branding);

    // Listener ports. Default :443 (HTTPS) + :80 (HTTP). Overridable for
    // non-root dev (`TCS_HTTPS_PORT=8443 TCS_HTTP_PORT=8081 cargo run`); set
    // either to 0 to skip that listener entirely.
    let https_port: u16 = std::env::var("TCS_HTTPS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(443);
    let http_port: u16 = std::env::var("TCS_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);

    // HTTPS with live-reloadable cert.
    let rustls_config = tls_runtime.rustls_config();
    let https_app = rest_app.clone();
    let https_handle = if https_port == 0 {
        info!("TCS_HTTPS_PORT=0 — skipping HTTPS listener");
        None
    } else {
        let https_addr: SocketAddr = format!("{bind}:{https_port}").parse()?;
        Some(tokio::spawn(async move {
            info!(addr = %https_addr, "Starting HTTPS server (live cert reload)");
            if let Err(e) = axum_server::bind_rustls(https_addr, rustls_config)
                .serve(https_app.into_make_service())
                .await
            {
                error!(error = %e, "HTTPS server error");
            }
        }))
    };

    // HTTP listener. When HTTPS is the primary path it serves ACME + a
    // redirect-to-HTTPS (browsers follow the redirect). When HTTPS is disabled
    // (non-root dev: `TCS_HTTPS_PORT=0`) it serves the real app directly so the
    // UI/API work on a plain high port.
    let serve_app_on_http = https_port == 0;
    let http_app = if serve_app_on_http {
        rest_app.clone()
    } else {
        build_http_redirect_router(acme_store.clone())
    };
    let http_handle = if http_port == 0 {
        info!("TCS_HTTP_PORT=0 — skipping HTTP listener");
        None
    } else {
        let http_addr: SocketAddr = format!("{bind}:{http_port}").parse()?;
        Some(tokio::spawn(async move {
            info!(addr = %http_addr, "Starting HTTP server (ACME + redirect, or app in dev)");
            match tokio::net::TcpListener::bind(http_addr).await {
                Ok(listener) => {
                    if let Err(e) = axum::serve(listener, http_app).await {
                        error!(error = %e, "HTTP server error");
                    }
                }
                Err(e) => {
                    // :80 may be held by another process. Not fatal: ACME HTTP-01
                    // won't work, but :443 does. Self-signed/provided/LE-DNS certs
                    // don't need :80.
                    error!(error = %e, "Could not bind HTTP port — continuing without it (ACME HTTP-01 unavailable)");
                }
            }
        }))
    };

    // Certificate renewal task (LE).
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

    let signals = run_signal_handlers();

    type ListenerFut = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;
    // Real Talos nodes dial this (plaintext gRPC on bind_port) to join the
    // SideroLink overlay; TCS then adds them as WireGuard peers on tcs-sl0.
    // The WireGuard *data* port is listen_port. Runs as a separate task; a
    // bind failure is non-fatal (direct-LAN management still works).
    let sl_grpc_port: u16 = std::env::var("TCS_SIDEROLINK_API_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(config.siderolink.bind_port);
    let sl_endpoint_host = std::env::var("TCS_SIDEROLINK_ENDPOINT_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("TCS_PUBLIC_HOST").ok().filter(|s| !s.is_empty()))
        .or_else(|| {
            // Derive the advertised host (same logic used to bake configs).
            config
                .server
                .advertised_url
                .trim()
                .split("//")
                .nth(1)
                .and_then(|h| h.split('/').next())
                .and_then(|h| h.split(':').next())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| config.server.bind_addr.clone());
    let sl_installation_id =
        std::env::var("TCS_SIDEROLINK_INSTALLATION_ID").unwrap_or_else(|_| "tcs".into());
    let sl_router = talos_control_system::siderolink::server::build_router(
        Arc::new(talos_control_system::siderolink::server::SiderolinkServer {
            cfg: config.siderolink.clone(),
            wg: state.siderolink_wg.clone(),
            pool: state.db_pool.clone(),
            installation_id: sl_installation_id.clone(),
            wg_endpoint_host: sl_endpoint_host.clone(),
            enabled: Arc::new(tokio::sync::RwLock::new(true)),
        }),
        talos_control_system::siderolink::server::WgGrpcNotSupported::default(),
    );
    // Bind the SideroLink gRPC API on ALL interfaces (0.0.0.0) so nodes reach
    // it via whichever host IP is routable to them (the management VLAN), not
    // only the configured HTTP bind_addr. The advertised apiUrl host is set
    // separately via TCS_SIDEROLINK_ENDPOINT_HOST / TCS_PUBLIC_HOST so nodes
    // dial the IP they can actually reach.
    let sl_grpc_addr: std::net::SocketAddr =
        format!("0.0.0.0:{sl_grpc_port}").parse().unwrap_or_else(|_| "0.0.0.0:8082".parse().unwrap());
    let sl_endpoint_host_log = sl_endpoint_host.clone();
    let sl_installation_id_log = sl_installation_id.clone();
    let sl_grpc_handle = tokio::spawn(async move {
        info!(
            addr = %sl_grpc_addr,
            endpoint = %sl_endpoint_host_log,
            installation_id = %sl_installation_id_log,
            "Starting SideroLink gRPC API (plaintext); WG data port = listen_port"
        );
        if let Err(e) = sl_router.serve(sl_grpc_addr).await {
            error!(error = %e, "SideroLink gRPC server error");
        }
    });
    let mut sl_grpc_done: ListenerFut = Box::pin(async move {
        let _ = sl_grpc_handle.await;
    });

    // A skipped (None) listener must not trip the select! — make it a future
    // that only resolves on shutdown, not instantly.
    let mut https_done: ListenerFut = if let Some(h) = https_handle {
        Box::pin(async move { let _ = h.await; })
    } else {
        Box::pin(std::future::pending())
    };
    let mut http_done: ListenerFut = if let Some(h) = http_handle {
        Box::pin(async move { let _ = h.await; })
    } else {
        Box::pin(std::future::pending())
    };
    tokio::select! {
        res = signals => {
            info!("Shutdown signal received: {}", res);
        }
        _ = &mut https_done => {
            info!("HTTPS server exited");
        }
        _ = &mut http_done => {
            info!("HTTP server exited");
        }
        _ = &mut sl_grpc_done => {
            info!("SideroLink gRPC server exited");
        }
        _ = renewal_handle => {
            info!("Certificate renewal task exited");
        }
    }

    info!("Shutting down gracefully...");
    Ok(())
}

/// Resolve the initial cert to load into the always-on HTTPS listener.
/// Returns (cert_pem, key_pem, effective_mode, effective_domains, note).
/// `effective_mode` is what we actually serve — when the config is "disabled"
/// or LE falls back, it is `SelfSigned`, so the live cert status reflects the
/// real cert instead of the stale config.
async fn resolve_initial_certificate(
    config: &Config,
    acme_store: &AcmeChallengeStore,
) -> Result<
    (String, String, talos_control_system::config::TlsMode, Vec<String>, String),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use talos_control_system::config::TlsMode;
    let fallback_domain = || {
        // Prefer a configured domain; else the advertised host; else localhost.
        config
            .tls
            .letsencrypt
            .as_ref()
            .and_then(|l| l.domains.first().cloned())
            .or_else(|| config.tls.self_signed.as_ref().and_then(|s| s.domains.first().cloned()))
            .or_else(|| advertised_host(config))
            .unwrap_or_else(|| "localhost".to_string())
    };

    let tls_enabled = config.tls.enabled && config.tls.mode != TlsMode::Disabled;
    if !tls_enabled {
        let domain = fallback_domain();
        info!(%domain, "TLS disabled in config — generating self-signed so :443 is reachable");
        let (c, k) =
            talos_control_system::cert::self_signed::generate_self_signed(std::slice::from_ref(&domain)).await?;
        return Ok((
            c,
            k,
            TlsMode::SelfSigned,
            vec![domain.clone()],
            format!("self-signed ({domain}) — TLS disabled in config, auto-enabled"),
        ));
    }

    match config.tls.mode {
        TlsMode::LetsEncrypt => {
            if let Some(le) = &config.tls.letsencrypt {
                if !le.domains.is_empty() {
                    let client = talos_control_system::cert::acme::AcmeClient::new(
                        &le.email,
                        le.dns_provider.clone(),
                        le.challenge_type.clone(),
                    )
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    match client
                        .obtain_certificate(&le.domains, Some(acme_store))
                        .await
                    {
                        Ok((c, k)) => {
                            return Ok((c, k, TlsMode::LetsEncrypt, le.domains.clone(), "Let's Encrypt issued".to_string()))
                        }
                        Err(e) => {
                            warn!(error = %e, "Let's Encrypt issuance failed at boot; using self-signed fallback");
                        }
                    }
                }
                let domain = fallback_domain();
                let (c, k) =
                    talos_control_system::cert::self_signed::generate_self_signed(std::slice::from_ref(&domain)).await?;
                return Ok((
                    c,
                    k,
                    TlsMode::SelfSigned,
                    vec![domain.clone()],
                    format!("self-signed ({domain}) — LE not ready (no domains or :80 unreachable)"),
                ));
            }
            let domain = fallback_domain();
            let (c, k) =
                talos_control_system::cert::self_signed::generate_self_signed(std::slice::from_ref(&domain)).await?;
            Ok((c, k, TlsMode::SelfSigned, vec![domain.clone()], format!("self-signed ({domain}) — LE mode not fully configured")))
        }
        TlsMode::SelfSigned => {
            let domains = config
                .tls
                .self_signed
                .as_ref()
                .map(|s| s.domains.clone())
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| vec![fallback_domain()]);
            let (c, k) = talos_control_system::cert::self_signed::generate_self_signed(&domains).await?;
            Ok((c, k, TlsMode::SelfSigned, domains.clone(), "self-signed (configured)".to_string()))
        }
        TlsMode::Provided => {
            let provided = config
                .tls
                .provided
                .as_ref()
                .ok_or("Provided TLS mode requires cert_path and key_path")?;
            let (c, k) =
                talos_control_system::cert::provided::load_provided_certs(
                    &provided.cert_path,
                    &provided.key_path,
                )
                .await?;
            Ok((c, k, TlsMode::Provided, vec![], "provided (uploaded)".to_string()))
        }
        TlsMode::Disabled => {
            let domain = fallback_domain();
            let (c, k) =
                talos_control_system::cert::self_signed::generate_self_signed(std::slice::from_ref(&domain)).await?;
            Ok((c, k, TlsMode::SelfSigned, vec![domain.clone()], format!("self-signed ({domain}) — mode disabled, auto-enabled")))
        }
    }
}

/// Best-effort public host for the self-signed SAN: the advertised_url host, if
/// parseable, else the bind address.
fn advertised_host(config: &Config) -> Option<String> {
    let url = config.server.advertised_url.trim();
    if url.is_empty() {
        return None;
    }
    url.split("//").nth(1)?
        .split('/').next()?
        .split(':').next()?.to_string().into()
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
