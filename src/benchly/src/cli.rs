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

    /// Drop collection before starting
    #[arg(long, default_value = "true")]
    pub drop_collection: bool,

    /// Do not drop collection before starting
    #[arg(long = "no-drop-collection", default_value = "false")]
    pub no_drop_collection: bool,

    /// Stop all workers on first failure
    #[arg(long, default_value = "false")]
    pub stop_on_failure: bool,

    /// Output directory for JSON report
    #[arg(long)]
    pub output_dir: Option<String>,

    /// Run label / benchmark name
    #[arg(long, default_value = "insert_bench")]
    pub run_label: String,

    /// Test type: read, find, write, update, find_and_update, aggregate, leak_cursor, or server_info
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

        Ok(properties)
    }

    pub fn should_drop_collection(&self) -> bool {
        self.drop_collection && !self.no_drop_collection
    }

    /// Cooldown seconds, defaulting to the warmup value when unset.
    pub fn cooldown(&self) -> u64 {
        self.cooldown.unwrap_or(self.warmup)
    }
}
