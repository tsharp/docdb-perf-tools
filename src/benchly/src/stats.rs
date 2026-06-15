use hdrhistogram::Histogram;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;

use crate::cli::Args;
use crate::report::*;

/// Shared counters only — no mutex in the hot path.
pub struct Stats {
    op_count: AtomicU64,
    doc_count: AtomicU64,
    failures: AtomicU64,
    start_time: Mutex<Instant>,
    recording: AtomicBool,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            op_count: AtomicU64::new(0),
            doc_count: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            start_time: Mutex::new(Instant::now()),
            recording: AtomicBool::new(false),
        }
    }

    pub async fn start_recording(&self) {
        self.op_count.store(0, Ordering::Relaxed);
        self.doc_count.store(0, Ordering::Relaxed);
        self.failures.store(0, Ordering::Relaxed);
        *self.start_time.lock().await = Instant::now();
        self.recording.store(true, Ordering::Release);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Acquire)
    }

    pub async fn elapsed(&self) -> f64 {
        self.start_time.lock().await.elapsed().as_secs_f64()
    }

    pub fn record_op(&self, docs: u64) {
        if !self.is_recording() { return; }
        self.op_count.fetch_add(1, Ordering::Relaxed);
        self.doc_count.fetch_add(docs, Ordering::Relaxed);
    }

    pub fn record_failure(&self) {
        if !self.is_recording() { return; }
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    fn hist_to_latency_stats(hist: &Histogram<u64>) -> LatencyStats {
        if hist.len() > 0 {
            LatencyStats {
                avg: hist.mean(),
                min: hist.min() as f64,
                max: hist.max() as f64,
                p50: hist.value_at_quantile(0.50),
                p75: hist.value_at_quantile(0.75),
                p90: hist.value_at_quantile(0.90),
                p95: hist.value_at_quantile(0.95),
                p99: hist.value_at_quantile(0.99),
                p999: hist.value_at_quantile(0.999),
                p100: hist.max(),
            }
        } else {
            LatencyStats::default()
        }
    }

    pub async fn generate_report(&self, hist: &Histogram<u64>, args: &Args, start_time_str: &str, end_time_str: &str) -> Report {
        let ops = self.op_count.load(Ordering::Relaxed);
        let docs = self.doc_count.load(Ordering::Relaxed);
        let fails = self.failures.load(Ordering::Relaxed);
        let elapsed = self.elapsed().await;

        Report {
            metadata: ReportMetadata {
                benchmark_name: args.run_label.clone(),
                run_label: args.run_label.clone(),
                database_engine: "azure_documentdb".to_string(),
                driver: "mongodb".to_string(),
                driver_version: "3.1".to_string(),
                driver_language: "rust".to_string(),
                properties: args.parsed_metadata_properties().unwrap_or_default(),
                database: args.database.clone(),
                collection: args.collection.clone(),
                workers: args.workers,
                run_time: format!("{}s", args.duration),
                warmup: format!("{}s", args.warmup),
                workload_params: WorkloadParams {
                    document_size: args.doc_size,
                    batch_size: args.batch_size,
                    max_writes_per_sec: args.max_writes_per_sec,
                    drop_on_start: args.should_drop_collection(),
                },
                start_time: start_time_str.to_string(),
                end_time: end_time_str.to_string(),
                duration_seconds: elapsed,
            },
            writes: WriteStats {
                total_operations: ops,
                total_documents: docs,
                total_failures: fails,
                operations_per_sec: ops as f64 / elapsed,
                documents_per_sec: docs as f64 / elapsed,
                failures_per_sec: fails as f64 / elapsed,
                latency_ms: Self::hist_to_latency_stats(hist),
            },
        }
    }

    pub async fn print_summary(&self, hist: &Histogram<u64>) {
        let ops = self.op_count.load(Ordering::Relaxed);
        let docs = self.doc_count.load(Ordering::Relaxed);
        let fails = self.failures.load(Ordering::Relaxed);
        let elapsed = self.elapsed().await;

        println!("\n================================================================================");
        println!("BENCHLY - INSERT BENCHMARK SUMMARY");
        println!("================================================================================");
        println!("  Duration:         {:.1}s", elapsed);
        println!("  Operations:       {}", ops);
        println!("  Documents:        {}", docs);
        println!("  Failures:         {}", fails);
        println!("  Ops/sec:          {:.1}", ops as f64 / elapsed);
        println!("  Docs/sec:         {:.1}", docs as f64 / elapsed);

        if hist.len() > 0 {
            println!("\nWrite Latency:");
            println!("  Min:              {} ms", hist.min());
            println!("  Mean:             {:.1} ms", hist.mean());
            println!("  P50:              {} ms", hist.value_at_quantile(0.50));
            println!("  P75:              {} ms", hist.value_at_quantile(0.75));
            println!("  P90:              {} ms", hist.value_at_quantile(0.90));
            println!("  P95:              {} ms", hist.value_at_quantile(0.95));
            println!("  P99:              {} ms", hist.value_at_quantile(0.99));
            println!("  Max:              {} ms", hist.max());
        }

        println!("================================================================================\n");
    }
}
