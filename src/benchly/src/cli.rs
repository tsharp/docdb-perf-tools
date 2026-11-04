use clap::Parser;
use std::collections::BTreeMap;
use std::fs;

#[derive(Parser, Debug, Clone)]
#[command(name = "benchly")]
#[command(about = "MongoDB benchmark - read/write tests")]
pub struct Args {
    /// MongoDB connection string. Falls back to --mongodb-url-file, BENCHLY_MONGODB_URL, then MONGODB_URL.
    #[arg(short, long)]
    pub mongodb_url: Option<String>,

    /// File containing the MongoDB connection string
    #[arg(long)]
    pub mongodb_url_file: Option<String>,

    /// Database name
    #[arg(short, long, default_value = "benchmark_db")]
    pub database: String,

    /// Collection name
    #[arg(short, long, default_value = "benchly_test")]
    pub collection: String,

    /// Number of concurrent worker tasks
    #[arg(short = 'w', long, default_value = "8")]
    pub workers: usize,

    /// Duration to run (seconds)
    #[arg(short = 't', long, default_value = "120")]
    pub duration: u64,

    /// Warmup time (seconds) before recording stats
    #[arg(long, default_value = "5")]
    pub warmup: u64,

    /// Cooldown time (seconds) after recording: workers keep running so the
    /// final metric intervals are exported at full load before shutdown,
    /// avoiding an end-of-run dip. Defaults to the warmup value.
    #[arg(long)]
    pub cooldown: Option<u64>,

    /// Document size in bytes
    #[arg(short = 's', long, default_value = "1024")]
    pub doc_size: usize,

    /// Maximum writes per second (-1 = unlimited)
    #[arg(long, default_value = "-1", allow_hyphen_values = true)]
    pub max_writes_per_sec: i64,

    /// Use insert_many with this batch size (0 = use insert_one)
    #[arg(long, default_value = "0")]
    pub batch_size: usize,

    /// Client-side deadline per insert (including warmup and batches), in milliseconds
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub insert_timeout_ms: Option<u64>,

    /// Send maxTimeMS on point-insert commands (write workload, batch size 0 or 1)
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=i32::MAX as u64))]
    pub insert_max_time_ms: Option<u64>,

    /// Drop collection before starting
    #[arg(long, default_value = "true")]
    pub drop_collection: bool,

    /// Do not drop collection before starting
    #[arg(long = "no-drop-collection", default_value = "false")]
    pub no_drop_collection: bool,

    /// Stop all workers on first failure (write workloads continue after insert timeouts)
    #[arg(long, default_value = "false")]
    pub stop_on_failure: bool,

    /// Output directory for JSON report
    #[arg(long)]
    pub output_dir: Option<String>,

    /// Run label / benchmark name
    #[arg(long, default_value = "insert_bench")]
    pub run_label: String,

    /// Test type: preload, read, find, write, update, find_and_update, aggregate, leak_cursor, or server_info
    #[arg(long, default_value = "write")]
    pub test: String,

    /// Additional report metadata as key=value. Repeat for multiple values.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub metadata_properties: Vec<String>,

    /// Use indexed documents (adds an indexed field)
    #[arg(long, default_value = "false")]
    pub indexed: bool,

    /// Number of documents to pre-load for read test
    #[arg(long, default_value = "250000")]
    pub preload_count: usize,

    /// Aggregation type for aggregate test (count, groupbycount, matchrangecount, groupbymodsum, sortlimit, matchgroupsort)
    #[arg(long, default_value = "count")]
    pub aggregation_type: String,

    /// Update type for update test (setfield, incrementcounter, setmultiplefields, conditionalupdate)
    #[arg(long, default_value = "setfield")]
    pub update_type: String,

    /// Send full replacement payload on each update (replace_one/find_one_and_replace)
    #[arg(long, default_value = "false")]
    pub full_update_payload: bool,

    /// Cursor batch size for leak_cursor test
    #[arg(long, default_value = "101")]
    pub cursor_batch_size: i64,

    /// Maximum documents to consume per find operation
    #[arg(long, default_value = "100")]
    pub find_limit: i64,

    /// Stream application metrics to the PerfLab API via the perflab-metrics
    /// crate. Can also be activated with `enabled = true` in perflab.toml.
    /// Configure the endpoint/auth with a perflab.toml file or PERFLAB_*
    /// environment variables.
    #[arg(long, default_value = "false")]
    pub perflab: bool,
}

impl Args {
    pub fn resolve_mongodb_url(&self) -> anyhow::Result<String> {
        if let Some(mongodb_url) = self.mongodb_url.as_deref() {
            let mongodb_url = mongodb_url.trim();
            if !mongodb_url.is_empty() {
                return Ok(mongodb_url.to_string());
            }
        }

        if let Some(path) = self.mongodb_url_file.as_deref() {
            let mongodb_url = fs::read_to_string(path).map_err(|error| {
                anyhow::anyhow!(
                    "Failed to read MongoDB connection string file {}: {}",
                    path,
                    error
                )
            })?;
            let mongodb_url = mongodb_url.trim();
            if !mongodb_url.is_empty() {
                return Ok(mongodb_url.to_string());
            }
            anyhow::bail!("MongoDB connection string file is empty: {}", path);
        }

        if let Ok(mongodb_url) = std::env::var("BENCHLY_MONGODB_URL") {
            let mongodb_url = mongodb_url.trim();
            if !mongodb_url.is_empty() {
                return Ok(mongodb_url.to_string());
            }
        }

        if let Ok(mongodb_url) = std::env::var("MONGODB_URL") {
            let mongodb_url = mongodb_url.trim();
            if !mongodb_url.is_empty() {
                return Ok(mongodb_url.to_string());
            }
        }

        anyhow::bail!(
            "Provide --mongodb-url, --mongodb-url-file, BENCHLY_MONGODB_URL, or MONGODB_URL"
        );
    }

    pub fn parsed_metadata_properties(&self) -> anyhow::Result<BTreeMap<String, String>> {
        let mut properties = BTreeMap::new();
        for value in &self.metadata_properties {
            let Some((key, metadata_value)) = value.split_once('=') else {
                anyhow::bail!("Metadata values must use key=value format: {}", value);
            };

            let key = key.trim();
            if key.is_empty() {
                anyhow::bail!("Metadata keys must not be empty: {}", value);
            }

            properties.insert(key.to_string(), metadata_value.to_string());
        }

        if let Some(timeout_ms) = self.insert_timeout_ms {
            properties.insert(
                "benchly.insert_timeout_ms".to_string(),
                timeout_ms.to_string(),
            );
        }
        if let Some(max_time_ms) = self.insert_max_time_ms {
            properties.insert(
                "benchly.insert_max_time_ms".to_string(),
                max_time_ms.to_string(),
            );
        }

        Ok(properties)
    }

    pub fn validate_insert_max_time(&self) -> anyhow::Result<()> {
        if self.insert_max_time_ms.is_some() {
            anyhow::ensure!(
                self.test.eq_ignore_ascii_case("write") && self.batch_size <= 1,
                "--insert-max-time-ms requires --test write and --batch-size 0 or 1 (point inserts)"
            );
        }
        Ok(())
    }

    pub fn should_drop_collection(&self) -> bool {
        self.drop_collection && !self.no_drop_collection
    }

    /// Cooldown seconds, defaulting to the warmup value when unset.
    pub fn cooldown(&self) -> u64 {
        self.cooldown.unwrap_or(self.warmup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_timeout_is_optional_positive_and_reported_as_metadata() {
        let defaults = Args::try_parse_from(["benchly"]).unwrap();
        assert_eq!(defaults.insert_timeout_ms, None);
        assert!(
            !defaults
                .parsed_metadata_properties()
                .unwrap()
                .contains_key("benchly.insert_timeout_ms")
        );
        assert!(Args::try_parse_from(["benchly", "--insert-timeout-ms", "0"]).is_err());
        assert!(Args::try_parse_from(["benchly", "--insert-timeout-ms", "-1"]).is_err());
        let configured = Args::try_parse_from(["benchly", "--insert-timeout-ms", "500"]).unwrap();
        assert_eq!(configured.insert_timeout_ms, Some(500));
        assert_eq!(
            configured.parsed_metadata_properties().unwrap()["benchly.insert_timeout_ms"],
            "500"
        );
    }

    #[test]
    fn wire_timeout_requires_positive_point_insert_configuration() {
        let defaults = Args::try_parse_from(["benchly"]).unwrap();
        assert_eq!(defaults.insert_max_time_ms, None);
        for value in ["0", "-1", "2147483648"] {
            assert!(Args::try_parse_from(["benchly", "--insert-max-time-ms", value]).is_err());
        }
        let mut args = Args::try_parse_from(["benchly", "--insert-max-time-ms", "5000"]).unwrap();
        assert!(args.validate_insert_max_time().is_ok());
        assert_eq!(
            args.parsed_metadata_properties().unwrap()["benchly.insert_max_time_ms"],
            "5000"
        );
        args.batch_size = 1;
        assert!(args.validate_insert_max_time().is_ok());
        args.batch_size = 2;
        assert!(args.validate_insert_max_time().is_err());
        args.batch_size = 0;
        args.test = "read".to_string();
        assert!(args.validate_insert_max_time().is_err());
    }
}
