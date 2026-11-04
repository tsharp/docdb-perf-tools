use hdrhistogram::Histogram;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::Mutex;

use crate::cli::Args;
use crate::metrics::Instruments;
use crate::report::*;

const MICROSECONDS_PER_MILLISECOND: f64 = 1_000.0;

fn micros_to_millis(microseconds: u64) -> f64 {
    microseconds as f64 / MICROSECONDS_PER_MILLISECOND
}

/// Shared counters and histogram for periodic snapshots.
pub struct Stats {
    op_count: AtomicU64,
    doc_count: AtomicU64,
    failures: AtomicU64,
    start_time: Mutex<Instant>,
    /// Set when recording stops so `elapsed()` reflects only the recorded
    /// window, even if workers keep running afterwards (e.g. during cooldown).
    stop_time: Mutex<Option<Instant>>,
    recording: AtomicBool,
    /// Shared histogram for collecting samples during recording.
    pub snapshot_hist: Mutex<Histogram<u64>>,
    /// Previous window counts for calculating throughput
    last_window_ops: AtomicU64,
    last_window_docs: AtomicU64,
    /// PerfLab metrics instruments (no-op when reporting is disabled).
    instruments: Instruments,
}

impl Stats {
    pub fn new() -> Self {
        Self {
            op_count: AtomicU64::new(0),
            doc_count: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            start_time: Mutex::new(Instant::now()),
            stop_time: Mutex::new(None),
            recording: AtomicBool::new(false),
            snapshot_hist: Mutex::new(Histogram::<u64>::new(3).unwrap()),
            last_window_ops: AtomicU64::new(0),
            last_window_docs: AtomicU64::new(0),
            instruments: Instruments::from_global(),
        }
    }

    pub async fn start_recording(&self) {
        self.op_count.store(0, Ordering::Relaxed);
        self.doc_count.store(0, Ordering::Relaxed);
        self.failures.store(0, Ordering::Relaxed);
        self.last_window_ops.store(0, Ordering::Relaxed);
        self.last_window_docs.store(0, Ordering::Relaxed);
        *self.start_time.lock().await = Instant::now();
        *self.snapshot_hist.lock().await = Histogram::<u64>::new(3).unwrap();
        *self.stop_time.lock().await = None;
        self.recording.store(true, Ordering::Release);
    }

    /// Stop recording: freeze the elapsed window and disable metric emission.
    /// Workers may keep running afterwards (e.g. during cooldown) without their
    /// operations being counted locally or exported to PerfLab.
    pub async fn stop_recording(&self) {
        let now = Instant::now();
        let mut stop = self.stop_time.lock().await;
        if stop.is_none() {
            *stop = Some(now);
        }
        self.recording.store(false, Ordering::Release);
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Acquire)
    }

    pub async fn elapsed(&self) -> f64 {
        let start = *self.start_time.lock().await;
        let end = self.stop_time.lock().await.unwrap_or_else(Instant::now);
        end.saturating_duration_since(start).as_secs_f64()
    }

    pub fn record_op(&self, docs: u64) {
        if !self.is_recording() {
            return;
        }
        self.op_count.fetch_add(1, Ordering::Relaxed);
        self.doc_count.fetch_add(docs, Ordering::Relaxed);
        self.instruments.record_op(docs);
    }

    pub fn record_failure(&self) {
        if !self.is_recording() {
            return;
        }
        self.failures.fetch_add(1, Ordering::Relaxed);
        self.instruments.record_failure();
    }

    /// Record the latency of a single operation (in microseconds).
    pub fn record_latency(&self, latency_us: u64) {
        if !self.is_recording() {
            return;
        }
        self.instruments.record_latency(latency_us);
    }

    /// Get current operation and document counts.
    pub fn get_current_counts(&self) -> (u64, u64, u64) {
        let ops = self.op_count.load(Ordering::Relaxed);
        let docs = self.doc_count.load(Ordering::Relaxed);
        let fails = self.failures.load(Ordering::Relaxed);
        (ops, docs, fails)
    }

    /// Print a snapshot of current throughput (last 5s window) and latencies.
    pub async fn print_snapshot(&self) {
        let elapsed = self.elapsed().await;
        if elapsed <= 0.0 {
            return;
        }

        let (ops, docs, _fails) = self.get_current_counts();

        // Calculate window throughput (ops/docs in last 5 seconds)
        let last_ops = self.last_window_ops.load(Ordering::Relaxed);
        let last_docs = self.last_window_docs.load(Ordering::Relaxed);

        let window_ops = ops.saturating_sub(last_ops);
        let window_docs = docs.saturating_sub(last_docs);

        // Update window counters for next snapshot
        self.last_window_ops.store(ops, Ordering::Relaxed);
        self.last_window_docs.store(docs, Ordering::Relaxed);

        let docs_per_sec = window_docs as f64 / 5.0;
        let ops_per_sec = window_ops as f64 / 5.0;

        print!(
            "[{:.1}s] Throughput: {:.1} docs/sec, {:.1} ops/sec",
            elapsed, docs_per_sec, ops_per_sec
        );

        let mut hist_lock = self.snapshot_hist.lock().await;
        if hist_lock.len() > 0 {
            let p50 = micros_to_millis(hist_lock.value_at_quantile(0.50));
            let p95 = micros_to_millis(hist_lock.value_at_quantile(0.95));
            let p99 = micros_to_millis(hist_lock.value_at_quantile(0.99));
            print!(" | Latency p50={:.3} p95={:.3} p99={:.3} ms", p50, p95, p99);

            // Reset histogram for next window
            *hist_lock = Histogram::<u64>::new(3).unwrap();
        }
        println!();
    }

    fn hist_to_latency_stats(hist: &Histogram<u64>) -> LatencyStats {
        if hist.len() > 0 {
            LatencyStats {
                avg: hist.mean() / MICROSECONDS_PER_MILLISECOND,
                min: micros_to_millis(hist.min()),
                max: micros_to_millis(hist.max()),
                p50: micros_to_millis(hist.value_at_quantile(0.50)),
                p75: micros_to_millis(hist.value_at_quantile(0.75)),
                p90: micros_to_millis(hist.value_at_quantile(0.90)),
                p95: micros_to_millis(hist.value_at_quantile(0.95)),
                p99: micros_to_millis(hist.value_at_quantile(0.99)),
                p999: micros_to_millis(hist.value_at_quantile(0.999)),
                p100: micros_to_millis(hist.max()),
            }
        } else {
            LatencyStats::default()
        }
    }

    pub async fn generate_report(
        &self,
        hist: &Histogram<u64>,
        args: &Args,
        start_time_str: &str,
        end_time_str: &str,
    ) -> Report {
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

        println!(
            "\n================================================================================"
        );
        println!("BENCHLY - BENCHMARK SUMMARY");
        println!(
            "================================================================================"
        );
        println!("  Duration:         {:.1}s", elapsed);
        println!("  Operations:       {}", ops);
        println!("  Documents:        {}", docs);
        println!("  Failures:         {}", fails);
        println!("  Ops/sec:          {:.1}", ops as f64 / elapsed);
        println!("  Docs/sec:         {:.1}", docs as f64 / elapsed);

        if hist.len() > 0 {
            println!("\nOperation Latency:");
            println!("  Min:              {:.3} ms", micros_to_millis(hist.min()));
            println!(
                "  Mean:             {:.3} ms",
                hist.mean() / MICROSECONDS_PER_MILLISECOND
            );
            println!(
                "  P50:              {:.3} ms",
                micros_to_millis(hist.value_at_quantile(0.50))
            );
            println!(
                "  P75:              {:.3} ms",
                micros_to_millis(hist.value_at_quantile(0.75))
            );
            println!(
                "  P90:              {:.3} ms",
                micros_to_millis(hist.value_at_quantile(0.90))
            );
            println!(
                "  P95:              {:.3} ms",
                micros_to_millis(hist.value_at_quantile(0.95))
            );
            println!(
                "  P99:              {:.3} ms",
                micros_to_millis(hist.value_at_quantile(0.99))
            );
            println!("  Max:              {:.3} ms", micros_to_millis(hist.max()));
        }

        println!(
            "================================================================================\n"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_stats_emit_fractional_milliseconds() {
        let mut histogram = Histogram::<u64>::new(3).unwrap();
        histogram.record(250).unwrap();
        histogram.record(750).unwrap();

        let stats = Stats::hist_to_latency_stats(&histogram);
        let emitted = serde_json::to_value(stats).unwrap();

        assert_eq!(emitted["min"], 0.25);
        assert_eq!(emitted["max"], 0.75);
        assert_eq!(emitted["p100"], 0.75);
        assert_eq!(emitted["avg"], 0.5);
    }
}
