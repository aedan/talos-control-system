use tracing_subscriber::EnvFilter;

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("talos_control_system=info,tower_http=debug,talos_rust_client=info"));

    let use_json = std::env::var("TCS_LOG_JSON").is_ok();

    if use_json {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .try_init()
            .ok();
    } else {
        tracing_subscriber::fmt()
            .with_target(true)
            .with_thread_ids(true)
            .with_thread_names(true)
            .with_env_filter(filter)
            .try_init()
            .ok();
    }
}
