use prometheus::{
    HistogramVec, Counter, Gauge,
};
use std::sync::LazyLock;

static HTTP_REQUEST_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    HistogramVec::new(
        prometheus::HistogramOpts::new(
            "tcs_http_request_duration_seconds",
            "HTTP request duration",
        ).buckets(prometheus::exponential_buckets(0.005, 2.0, 10).unwrap()),
        &["method", "path", "status"],
    ).unwrap()
});

static HTTP_REQUEST_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    Counter::new("tcs_http_requests_total", "Total HTTP requests").unwrap()
});

static GRPC_REQUEST_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    HistogramVec::new(
        prometheus::HistogramOpts::new(
            "tcs_grpc_request_duration_seconds",
            "gRPC request duration",
        ).buckets(prometheus::exponential_buckets(0.005, 2.0, 10).unwrap()),
        &["method", "status"],
    ).unwrap()
});

static ACTIVE_CONNECTIONS: LazyLock<Gauge> = LazyLock::new(|| {
    Gauge::new("tcs_active_connections", "Number of active connections").unwrap()
});

static CLUSTER_COUNT: LazyLock<Gauge> = LazyLock::new(|| {
    Gauge::new("tcs_clusters_total", "Total number of clusters").unwrap()
});

static MACHINE_COUNT: LazyLock<Gauge> = LazyLock::new(|| {
    Gauge::new("tcs_machines_total", "Total number of machines").unwrap()
});

static RECONCILIATION_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    HistogramVec::new(
        prometheus::HistogramOpts::new(
            "tcs_reconciliation_duration_seconds",
            "Reconciliation loop duration",
        ).buckets(prometheus::exponential_buckets(0.01, 2.0, 10).unwrap()),
        &["controller"],
    ).unwrap()
});

static CACHE_HITS: LazyLock<Counter> = LazyLock::new(|| {
    Counter::new("tcs_cache_hits_total", "Total cache hits").unwrap()
});

static CACHE_MISSES: LazyLock<Counter> = LazyLock::new(|| {
    Counter::new("tcs_cache_misses_total", "Total cache misses").unwrap()
});

pub fn register_metrics() {
    let _ = &*HTTP_REQUEST_DURATION;
    let _ = &*HTTP_REQUEST_TOTAL;
    let _ = &*GRPC_REQUEST_DURATION;
    let _ = &*ACTIVE_CONNECTIONS;
    let _ = &*CLUSTER_COUNT;
    let _ = &*MACHINE_COUNT;
    let _ = &*RECONCILIATION_DURATION;
    let _ = &*CACHE_HITS;
    let _ = &*CACHE_MISSES;
}
