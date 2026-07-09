//! PerfLab metrics integration.
//!
//! Wraps the [`perflab_metrics`] crate so benchmark workloads can stream
//! application metrics (operations, documents, failures, latency) to a PerfLab
//! run. Metrics recording goes through OpenTelemetry instruments created on the
//! global meter provider. When no session is active the global provider is a
//! no-op, so recording is effectively free and the rest of the code path is
//! unchanged.

use perflab_metrics::opentelemetry::global;
use perflab_metrics::opentelemetry::metrics::{Counter, Histogram};
use perflab_metrics::{RunConfig, RunType, Session};

use serde::Deserialize;
use std::path::PathBuf;

use crate::cli::Args;

/// OpenTelemetry instruments used to record benchmark metrics.
///
/// Built from the global meter provider. If a [`Session`] has been started the
/// provider is the real PerfLab exporter; otherwise these resolve to no-op
/// instruments.
pub struct Instruments {
    operations: Counter<u64>,
    documents: Counter<u64>,
    failures: Counter<u64>,
    latency: Histogram<f64>,
}

impl Instruments {
    /// Creates the benchmark instruments from the global meter provider.
    pub fn from_global() -> Self {
        let meter = global::meter("benchly");
        Self {
            operations: meter.u64_counter("benchly.operations").with_unit("1").build(),
            documents: meter.u64_counter("benchly.documents").with_unit("1").build(),
            failures: meter.u64_counter("benchly.failures").with_unit("1").build(),
            latency: meter
                .f64_histogram("benchly.latency")
                .with_unit("ms")
                .build(),
        }
    }

    /// Records a successful operation that returned/wrote `docs` documents.
    pub fn record_op(&self, docs: u64) {
        self.operations.add(1, &[]);
        if docs > 0 {
            self.documents.add(docs, &[]);
        }
    }

    /// Records a failed operation.
    pub fn record_failure(&self) {
        self.failures.add(1, &[]);
    }

    /// Records the latency of an operation in milliseconds.
    pub fn record_latency(&self, latency_ms: u64) {
        self.latency.record(latency_ms as f64, &[]);
    }
}

/// A PerfLab run session tied to the lifetime of a benchmark.
///
/// When PerfLab reporting is disabled (or fails to start) this holds `None` and
/// all lifecycle calls are no-ops, so benchmarks keep running normally.
pub struct MetricsSession {
    session: Option<Session>,
}

impl MetricsSession {
    /// Starts a PerfLab run when enabled.
    ///
    /// Reporting is activated by either the `--perflab` flag or an
    /// `enabled = true` value in `perflab.toml`. Configuration is layered
    /// defaults -> `perflab.toml` -> `PERFLAB_*` env vars (see the
    /// `perflab-metrics` crate). Benchly-specific run metadata is attached as
    /// run attributes. Any failure is logged and downgrades to a disabled
    /// session so the benchmark still runs.
    pub async fn start(args: &Args) -> Self {
        if !args.perflab && !config_enabled() {
            return Self { session: None };
        }

        let config = match build_config(args) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("perflab-metrics disabled: failed to load config: {}", e);
                return Self { session: None };
            }
        };

        // Start run: creates the run via POST /v1/runs (server assigns the id).
        match tokio::task::spawn_blocking(move || Session::start(config)).await {
            Ok(Ok(session)) => {
                println!("perflab-metrics: started run {}", session.run_id());
                Self {
                    session: Some(session),
                }
            }
            Ok(Err(e)) => {
                eprintln!("perflab-metrics disabled: failed to start session: {}", e);
                Self { session: None }
            }
            Err(e) => {
                eprintln!("perflab-metrics disabled: start task failed: {}", e);
                Self { session: None }
            }
        }
    }

    /// Flushes final metrics and stops the run, marking it completed (or
    /// failed) via `PATCH /v1/runs/:id`.
    ///
    /// Consumes the session; a no-op when reporting is disabled.
    pub async fn finish(self, failed: Option<String>) {
        let Some(session) = self.session else {
            return;
        };

        let run_id = session.run_id().to_string();
        let outcome = if failed.is_some() { "failed" } else { "completed" };

        let result = tokio::task::spawn_blocking(move || match failed.as_deref() {
            Some(reason) => session.fail(reason),
            None => session.complete(),
        })
        .await;

        match result {
            Ok(Ok(())) => println!("perflab-metrics: run {} {}", run_id, outcome),
            Ok(Err(e)) => eprintln!("perflab-metrics: failed to finalize run {}: {}", run_id, e),
            Err(e) => eprintln!("perflab-metrics: finalize task failed: {}", e),
        }
    }
}

/// Builds the PerfLab run configuration from layered sources plus benchly args.
fn build_config(args: &Args) -> anyhow::Result<RunConfig> {
    let mut config = load_run_config()?;

    let test = args.test.to_lowercase();
    if config.benchmark_name == "perflab-harness" {
        config.benchmark_name = format!("benchly-{}", test);
    }
    if config.run_label.is_none() {
        config.run_label = Some(args.run_label.clone());
    }
    config.run_type = RunType::Benchmark;

    config.set_attribute("test", test);
    config.set_attribute("database", args.database.clone());
    config.set_attribute("collection", args.collection.clone());
    config.set_attribute("workers", args.workers.to_string());
    config.set_attribute("doc_size", args.doc_size.to_string());
    config.set_attribute("batch_size", args.batch_size.to_string());
    config.set_attribute("indexed", args.indexed.to_string());
    config.extend_attributes(args.parsed_metadata_properties()?);

    Ok(config)
}

/// Loads the PerfLab config with the crate's precedence (defaults -> file ->
/// env), but discovers `perflab.toml` by walking up from the working directory
/// so the repo-root config is found regardless of where benchly is launched.
///
/// An explicit `PERFLAB_CONFIG` override always takes precedence.
fn load_run_config() -> anyhow::Result<RunConfig> {
    if std::env::var_os("PERFLAB_CONFIG").is_some() {
        return Ok(RunConfig::load()?);
    }

    if let Some(path) = find_config_file() {
        let mut config = RunConfig::from_toml_file(path)?;
        config.apply_env();
        return Ok(config);
    }

    Ok(RunConfig::load()?)
}

/// Whether `perflab.toml` activates reporting via `enabled = true`.
///
/// Missing file, unreadable file, parse errors, or an absent key all resolve to
/// `false`, so metrics stay opt-in.
fn config_enabled() -> bool {
    let Some(path) = config_file_path() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };

    #[derive(Deserialize)]
    struct Activation {
        #[serde(default)]
        enabled: bool,
    }

    toml::from_str::<Activation>(&text)
        .map(|activation| activation.enabled)
        .unwrap_or(false)
}

/// Resolves the config file path: `PERFLAB_CONFIG` if set, otherwise the nearest
/// `perflab.toml` walking up from the working directory.
fn config_file_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PERFLAB_CONFIG") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    find_config_file()
}

/// Searches for `perflab.toml` starting at the current directory and walking up
/// through parent directories.
fn find_config_file() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("perflab.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

