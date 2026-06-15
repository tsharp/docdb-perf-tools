use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Clone, Default)]
pub struct LatencyStats {
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub p50: u64,
    pub p75: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub p999: u64,
    pub p100: u64,
}

#[derive(Serialize)]
pub struct ReportMetadata {
    pub benchmark_name: String,
    pub run_label: String,
    pub database_engine: String,
    pub driver: String,
    pub driver_version: String,
    pub driver_language: String,
    pub properties: BTreeMap<String, String>,
    pub database: String,
    pub collection: String,
    pub workers: usize,
    pub run_time: String,
    pub warmup: String,
    pub workload_params: WorkloadParams,
    pub start_time: String,
    pub end_time: String,
    pub duration_seconds: f64,
}

#[derive(Serialize)]
pub struct WorkloadParams {
    pub document_size: usize,
    pub batch_size: usize,
    pub max_writes_per_sec: i64,
    pub drop_on_start: bool,
}

#[derive(Serialize)]
pub struct WriteStats {
    pub total_operations: u64,
    pub total_documents: u64,
    pub total_failures: u64,
    pub operations_per_sec: f64,
    pub documents_per_sec: f64,
    pub failures_per_sec: f64,
    pub latency_ms: LatencyStats,
}

#[derive(Serialize)]
pub struct Report {
    pub metadata: ReportMetadata,
    pub writes: WriteStats,
}
