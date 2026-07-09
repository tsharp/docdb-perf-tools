mod aggregator;
mod cli;
mod cursor_leaker;
mod finder;
mod metrics;
mod reader;
mod report;
mod stats;
mod updater;
mod writer;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use hdrhistogram::Histogram;
use mongodb::{
    bson::{doc, rawdoc, Document, RawDocumentBuf},
    options::{ClientOptions, TlsOptions},
    Client, IndexModel,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Barrier;

use aggregator::{aggregator_task, AggregationType};
use cli::Args;
use cursor_leaker::cursor_leaker_task;
use finder::finder_task;
use reader::reader_task;
use stats::Stats;
use updater::{UpdateOperation, UpdateType, updater_task};
use writer::{make_raw_document, writer_task};

async fn create_client(args: &Args) -> Result<Client> {
    let mongodb_url = args.resolve_mongodb_url()?;
    let mut client_options = ClientOptions::parse(&mongodb_url)
        .await
        .context("Failed to parse MongoDB URL")?;

    client_options.retry_writes = Some(false);
    client_options.retry_reads = Some(false);
    client_options.max_pool_size = Some(args.workers as u32 * 2);
    client_options.min_pool_size = Some(args.workers as u32);

    // Allow invalid TLS certificates for testing/development
    let mut tls_options = TlsOptions::default();
    tls_options.allow_invalid_certificates = Some(true);
    client_options.tls = Some(tls_options.into());

    let client =
        Client::with_options(client_options).context("Failed to create MongoDB client")?;

    // Pre-flight: `Client::with_options` is lazy and does not actually connect,
    // so ping the server up front to surface connection problems clearly before
    // the benchmark starts doing work.
    println!("Validating MongoDB connection...");
    client
        .database("admin")
        .run_command(doc! { "ping": 1 })
        .await
        .context("MongoDB pre-flight connection check failed (could not ping server)")?;
    println!("MongoDB connection OK.");

    Ok(client)
}

/// Records for `duration` seconds with periodic snapshots, then stops
/// recording so nothing is counted or exported for the `cooldown` "overlap".
/// The workers keep running during cooldown to keep the connection and server
/// warm while the final recorded intervals finish exporting, but their
/// operations are no longer emitted to PerfLab.
async fn record_window(stats: &Stats, duration: u64, cooldown: u64) {
    let record_start = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        stats.print_snapshot().await;
        if record_start.elapsed().as_secs() >= duration {
            break;
        }
    }

    // Stop recording first so the cooldown window is neither counted locally
    // nor exported to PerfLab.
    stats.stop_recording().await;

    if cooldown > 0 {
        println!(
            "Cooling down for {}s (load stays on, metrics no longer emitted)...\n",
            cooldown
        );
        tokio::time::sleep(Duration::from_secs(cooldown)).await;
    }
}

async fn write_report(
    stats: &Stats,
    hist: &Histogram<u64>,
    args: &Args,
    start_time_str: &str,
    end_time_str: &str,
) -> Result<()> {
    let report = stats
        .generate_report(hist, args, start_time_str, end_time_str)
        .await;

    if let Some(output_dir) = &args.output_dir {
        let base_dir = PathBuf::from(output_dir)
            .join(format!("{}_users", args.workers))
            .join(&args.run_label);

        fs::create_dir_all(&base_dir).context("Failed to create output directory")?;

        let ts = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let report_path = base_dir.join(format!("{}_{}_report.json", args.run_label, ts));
        let json = serde_json::to_string_pretty(&report).context("Failed to serialize report")?;
        fs::write(&report_path, &json).context("Failed to write report file")?;

        println!("Report written to: {}", report_path.display());
    } else {
        let json = serde_json::to_string_pretty(&report).context("Failed to serialize report")?;
        println!("\n--- JSON REPORT ---\n{}\n--- END REPORT ---", json);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    args.parsed_metadata_properties()?;

    let test_type = args.test.to_lowercase();

    if test_type == "read" {
        run_read_benchmark(args).await
    } else if test_type == "find" {
        run_find_benchmark(args).await
    } else if test_type == "aggregate" {
        run_aggregate_benchmark(args).await
    } else if test_type == "update" {
        run_update_benchmark(args, UpdateOperation::UpdateOne).await
    } else if test_type == "find_and_update" {
        run_update_benchmark(args, UpdateOperation::FindOneAndUpdate).await
    } else if test_type == "leak_cursor" {
        run_cursor_leak_benchmark(args).await
    } else if test_type == "server_info" {
        run_server_info(args).await
    } else {
        run_write_benchmark(args).await
    }
}

async fn run_server_info(args: Args) -> Result<()> {
    println!("\n================================================================================");
    println!("BENCHLY - SERVER INFO");
    println!("================================================================================");

    let client = create_client(&args).await?;
    let admin_db = client.database("admin");

    let hello = admin_db
        .run_command(doc! { "hello": 1 })
        .await
        .context("Failed to run hello command")?;
    let build_info = admin_db
        .run_command(doc! { "buildInfo": 1 })
        .await
        .context("Failed to run buildInfo command")?;

    println!("hello:");
    println!("{}", serde_json::to_string_pretty(&hello).context("Failed to serialize hello response")?);
    println!();
    println!("buildInfo:");
    println!("{}", serde_json::to_string_pretty(&build_info).context("Failed to serialize buildInfo response")?);
    Ok(())
}

async fn run_find_benchmark(args: Args) -> Result<()> {
    println!("\n================================================================================");
    println!("BENCHLY - FIND BENCHMARK");
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Document size:    {} bytes", args.doc_size);
    println!("  Indexed:          {}", args.indexed);
    println!("  Preload count:    {}", args.preload_count);
    println!("  Find limit:       {}", args.find_limit);
    println!("  Cursor batch size: {}", args.cursor_batch_size);
    println!("================================================================================\n");

    let client = create_client(&args).await?;
    let db = client.database(&args.database);
    let collection = db.collection::<Document>(&args.collection);
    let raw_collection = db.collection::<RawDocumentBuf>(&args.collection);

    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Pre-loading {} documents...", args.preload_count);

        let batch_size = 1000;
        let mut loaded = 0;
        while loaded < args.preload_count {
            let remaining = args.preload_count - loaded;
            let current_batch = remaining.min(batch_size);

            let docs: Vec<RawDocumentBuf> = (0..current_batch)
                .map(|_| make_raw_document(args.doc_size, args.indexed))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            raw_collection
                .insert_many(doc_refs)
                .await
                .context("Failed to pre-load documents")?;

            loaded += current_batch;
            if loaded % 50000 == 0 {
                println!("  Loaded {}/{} documents...", loaded, args.preload_count);
            }
        }

        println!("Pre-load complete: {} documents", loaded);

        if args.indexed {
            println!("Creating index on indexed_field...");
            let index = IndexModel::builder()
                .keys(doc! { "indexed_field": 1 })
                .build();
            collection
                .create_index(index)
                .await
                .context("Failed to create index")?;
        }

        println!("Waiting 5s for database to settle...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    let mut handles = Vec::new();
    for i in 0..args.workers {
        let db_clone = db.clone();
        let coll_name = args.collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let stop_on_fail = args.stop_on_failure;
        let find_limit = args.find_limit;
        let cursor_batch_size = args.cursor_batch_size;

        let handle = tokio::spawn(async move {
            finder_task(
                db_clone,
                coll_name,
                stats_clone,
                running_clone,
                i,
                stop_on_fail,
                barrier_clone,
                find_limit,
                cursor_batch_size,
            )
            .await
        });
        handles.push(handle);
    }

    println!("Waiting for all {} finders to warm up...", args.workers);
    barrier.wait().await;

    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!("All finders ready. Recording for {}s...\n", args.duration);

    record_window(&stats, args.duration, args.cooldown()).await;

    running.store(false, Ordering::Relaxed);

    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Finder error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}

async fn run_write_benchmark(args: Args) -> Result<()> {
    println!("\n================================================================================");
    println!("BENCHLY - WRITE BENCHMARK");
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Document size:    {} bytes", args.doc_size);
    println!("  Indexed:          {}", args.indexed);
    println!(
        "  Batch size:       {}",
        if args.batch_size > 1 {
            format!("{}", args.batch_size)
        } else {
            "1 (insert_one)".to_string()
        }
    );
    println!(
        "  Max writes/sec:   {}",
        if args.max_writes_per_sec < 0 {
            "unlimited".to_string()
        } else {
            format!("{}", args.max_writes_per_sec)
        }
    );
    println!("================================================================================\n");

    let client = create_client(&args).await?;

    let db = client.database(&args.database);
    let collection = db.collection::<RawDocumentBuf>(&args.collection);

    // Drop collection
    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Creating collection...");
        collection
            .insert_one(rawdoc! { "_init": true })
            .await
            .context("Failed to create collection")?;

        // Create index if indexed mode
        if args.indexed {
            println!("Creating index on indexed_field...");
            let index = IndexModel::builder()
                .keys(doc! { "indexed_field": 1 })
                .build();
            collection
                .create_index(index)
                .await
                .context("Failed to create index")?;
        }

        println!("Collection ready, waiting 5s...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    // +1 for the main task which also waits on the barrier
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    // Spawn workers — each returns its own histogram (no shared mutex)
    let mut handles = Vec::new();
    for i in 0..args.workers {
        let coll = collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let batch = args.batch_size;
        let max_wps = args.max_writes_per_sec;
        let num_workers = args.workers;
        let stop_on_fail = args.stop_on_failure;
        let ds = args.doc_size;
        let indexed = args.indexed;

        let handle = tokio::spawn(async move {
            writer_task(
                coll,
                stats_clone,
                running_clone,
                i,
                ds,
                batch,
                max_wps,
                num_workers,
                stop_on_fail,
                barrier_clone,
                indexed,
            )
            .await
        });
        handles.push(handle);
    }

    // Wait for ALL workers to complete their warmup insert
    println!("Waiting for all {} workers to warm up...", args.workers);
    barrier.wait().await;

    // Start recording — all workers are now connected and ready
    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!("All workers ready. Recording for {}s...\n", args.duration);

    record_window(&stats, args.duration, args.cooldown()).await;

    // Stop
    running.store(false, Ordering::Relaxed);

    // Collect and merge histograms from all workers
    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Writer error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    // Report
    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}

async fn run_read_benchmark(args: Args) -> Result<()> {
    println!("\n================================================================================");
    println!("BENCHLY - POINT READ BENCHMARK");
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Document size:    {} bytes", args.doc_size);
    println!("  Indexed:          {}", args.indexed);
    println!("  Preload count:    {}", args.preload_count);
    println!("================================================================================\n");

    let client = create_client(&args).await?;

    let db = client.database(&args.database);

    // Use Document collection for reads (easier for queries)
    let collection = db.collection::<Document>(&args.collection);
    let raw_collection = db.collection::<RawDocumentBuf>(&args.collection);

    // Drop and pre-load collection
    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Pre-loading {} documents...", args.preload_count);

        // Pre-load documents in batches
        let batch_size = 1000;
        let mut loaded = 0;
        while loaded < args.preload_count {
            let remaining = args.preload_count - loaded;
            let current_batch = remaining.min(batch_size);

            let docs: Vec<RawDocumentBuf> = (0..current_batch)
                .map(|_| make_raw_document(args.doc_size, args.indexed))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            raw_collection
                .insert_many(doc_refs)
                .await
                .context("Failed to pre-load documents")?;

            loaded += current_batch;
            if loaded % 50000 == 0 {
                println!("  Loaded {}/{} documents...", loaded, args.preload_count);
            }
        }

        println!("Pre-load complete: {} documents", loaded);

        // Create index if indexed mode
        if args.indexed {
            println!("Creating index on indexed_field...");
            let index = IndexModel::builder()
                .keys(doc! { "indexed_field": 1 })
                .build();
            collection
                .create_index(index)
                .await
                .context("Failed to create index")?;
        }

        println!("Waiting 5s for database to settle...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // Fetch all document IDs and partition them among workers
    println!("Fetching document IDs for worker partitioning...");
    let mut all_ids: Vec<String> = Vec::new();
    let mut cursor = collection
        .find(doc! {})
        .await
        .context("Failed to query documents")?;

    use mongodb::bson::Bson;
    while let Ok(true) = cursor.advance().await {
        if let Ok(doc) = cursor.deserialize_current() {
            if let Some(Bson::String(id)) = doc.get("_id") {
                all_ids.push(id.clone());
            }
        }
    }

    println!("Fetched {} document IDs", all_ids.len());

    // Partition IDs among workers (non-overlapping batches)
    let ids_per_worker = all_ids.len() / args.workers;
    let mut worker_id_batches: Vec<Vec<String>> = Vec::new();

    for i in 0..args.workers {
        let start = i * ids_per_worker;
        let end = if i == args.workers - 1 {
            all_ids.len() // Last worker gets any remaining IDs
        } else {
            (i + 1) * ids_per_worker
        };
        let batch = all_ids[start..end].to_vec();
        println!(
            "Worker {} assigned {} IDs (range: {}..{})",
            i,
            batch.len(),
            start,
            end
        );
        worker_id_batches.push(batch);
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    // Spawn readers
    let mut handles = Vec::new();
    for i in 0..args.workers {
        let coll = collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let stop_on_fail = args.stop_on_failure;
        let id_batch = worker_id_batches[i].clone();

        let handle = tokio::spawn(async move {
            reader_task(
                coll,
                stats_clone,
                running_clone,
                i,
                id_batch,
                stop_on_fail,
                barrier_clone,
            )
            .await
        });
        handles.push(handle);
    }

    // Wait for all readers to warm up
    println!("\nWaiting for all {} readers to warm up...", args.workers);
    barrier.wait().await;

    // Start recording
    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!("All readers ready. Recording for {}s...\n", args.duration);

    record_window(&stats, args.duration, args.cooldown()).await;

    // Stop
    running.store(false, Ordering::Relaxed);

    // Collect and merge histograms
    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Reader error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    // Report
    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}

async fn run_aggregate_benchmark(args: Args) -> Result<()> {
    let agg_type = AggregationType::from_str(&args.aggregation_type)
        .ok_or_else(|| anyhow::anyhow!("Invalid aggregation type: {}. Valid types: count, groupbycount, matchrangecount, groupbymodsum, sortlimit, matchgroupsort", args.aggregation_type))?;

    println!("\n================================================================================");
    println!("BENCHLY - AGGREGATE QUERY BENCHMARK");
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Document size:    {} bytes", args.doc_size);
    println!("  Indexed:          {}", args.indexed);
    println!("  Preload count:    {}", args.preload_count);
    println!("  Aggregation type: {}", agg_type.to_string());
    println!("================================================================================\n");

    let client = create_client(&args).await?;

    let db = client.database(&args.database);
    let collection = db.collection::<Document>(&args.collection);
    let raw_collection = db.collection::<RawDocumentBuf>(&args.collection);

    // Drop and pre-load collection
    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Pre-loading {} documents...", args.preload_count);

        // Pre-load documents in batches
        let batch_size = 1000;
        let mut loaded = 0;
        while loaded < args.preload_count {
            let remaining = args.preload_count - loaded;
            let current_batch = remaining.min(batch_size);

            // Always use indexed documents for aggregate queries
            let docs: Vec<RawDocumentBuf> = (0..current_batch)
                .map(|_| make_raw_document(args.doc_size, true))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            raw_collection
                .insert_many(doc_refs)
                .await
                .context("Failed to pre-load documents")?;

            loaded += current_batch;
            if loaded % 50000 == 0 {
                println!("  Loaded {}/{} documents...", loaded, args.preload_count);
            }
        }

        println!("Pre-load complete: {} documents", loaded);

        // Create index on indexed_field (required for most aggregations)
        println!("Creating index on indexed_field...");
        let index = IndexModel::builder()
            .keys(doc! { "indexed_field": 1 })
            .build();
        collection
            .create_index(index)
            .await
            .context("Failed to create index")?;

        println!("Waiting 5s for database to settle...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    // Spawn aggregators
    let mut handles = Vec::new();
    for i in 0..args.workers {
        let db_clone = db.clone();
        let coll_name = args.collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let stop_on_fail = args.stop_on_failure;
        let agg_type_clone = agg_type.clone();
        let preload = args.preload_count;

        let handle = tokio::spawn(async move {
            aggregator_task(
                db_clone,
                coll_name,
                stats_clone,
                running_clone,
                i,
                agg_type_clone,
                preload,
                stop_on_fail,
                barrier_clone,
            )
            .await
        });
        handles.push(handle);
    }

    // Wait for all aggregators to warm up
    println!("Waiting for all {} aggregators to warm up...", args.workers);
    barrier.wait().await;

    // Start recording
    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!(
        "All aggregators ready. Recording for {}s...\n",
        args.duration
    );

    record_window(&stats, args.duration, args.cooldown()).await;

    // Stop
    running.store(false, Ordering::Relaxed);

    // Collect and merge histograms
    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Aggregator error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    // Report
    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}

async fn run_update_benchmark(args: Args, update_operation: UpdateOperation) -> Result<()> {
    let update_type = UpdateType::from_str(&args.update_type)
        .ok_or_else(|| anyhow::anyhow!("Invalid update type: {}. Valid types: setfield, incrementcounter, setmultiplefields, conditionalupdate", args.update_type))?;

    let benchmark_name = match update_operation {
        UpdateOperation::UpdateOne => "UPDATE BENCHMARK",
        UpdateOperation::FindOneAndUpdate => "FIND AND UPDATE BENCHMARK",
    };
    let operation_name = match update_operation {
        UpdateOperation::UpdateOne => "update_one",
        UpdateOperation::FindOneAndUpdate => "find_one_and_update",
    };

    println!("\n================================================================================");
    println!("BENCHLY - {}", benchmark_name);
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Document size:    {} bytes", args.doc_size);
    println!("  Indexed:          {}", args.indexed);
    println!("  Preload count:    {}", args.preload_count);
    println!("  Update type:      {}", update_type.to_string());
    println!("  Update op:        {}", operation_name);
    println!("  Full payload:     {}", args.full_update_payload);
    println!("================================================================================\n");

    let client = create_client(&args).await?;

    let db = client.database(&args.database);
    let collection = db.collection::<Document>(&args.collection);
    let raw_collection = db.collection::<RawDocumentBuf>(&args.collection);

    // Drop and pre-load collection
    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Pre-loading {} documents...", args.preload_count);

        // Pre-load documents in batches
        let batch_size = 1000;
        let mut loaded = 0;
        while loaded < args.preload_count {
            let remaining = args.preload_count - loaded;
            let current_batch = remaining.min(batch_size);

            let docs: Vec<RawDocumentBuf> = (0..current_batch)
                .map(|_| make_raw_document(args.doc_size, args.indexed))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            raw_collection
                .insert_many(doc_refs)
                .await
                .context("Failed to pre-load documents")?;

            loaded += current_batch;
            if loaded % 50000 == 0 {
                println!("  Loaded {}/{} documents...", loaded, args.preload_count);
            }
        }

        println!("Pre-load complete: {} documents", loaded);

        // Create index if indexed mode
        if args.indexed {
            println!("Creating index on indexed_field...");
            let index = IndexModel::builder()
                .keys(doc! { "indexed_field": 1 })
                .build();
            collection
                .create_index(index)
                .await
                .context("Failed to create index")?;
        }

        println!("Waiting 5s for database to settle...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // Fetch all document IDs and partition them among workers
    println!("Fetching document IDs for worker partitioning...");
    let mut all_ids: Vec<String> = Vec::new();
    let mut cursor = collection
        .find(doc! {})
        .await
        .context("Failed to query documents")?;

    use mongodb::bson::Bson;
    while let Ok(true) = cursor.advance().await {
        if let Ok(doc) = cursor.deserialize_current() {
            if let Some(Bson::String(id)) = doc.get("_id") {
                all_ids.push(id.clone());
            }
        }
    }

    println!("Fetched {} document IDs", all_ids.len());

    // Partition IDs among workers (non-overlapping batches)
    let ids_per_worker = all_ids.len() / args.workers;
    let mut worker_id_batches: Vec<Vec<String>> = Vec::new();

    for i in 0..args.workers {
        let start = i * ids_per_worker;
        let end = if i == args.workers - 1 {
            all_ids.len() // Last worker gets any remaining IDs
        } else {
            (i + 1) * ids_per_worker
        };
        let batch = all_ids[start..end].to_vec();
        println!(
            "Worker {} assigned {} IDs (range: {}..{})",
            i,
            batch.len(),
            start,
            end
        );
        worker_id_batches.push(batch);
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    // Spawn updaters
    let mut handles = Vec::new();
    for i in 0..args.workers {
        let db_clone = db.clone();
        let coll_name = args.collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let stop_on_fail = args.stop_on_failure;
        let id_batch = worker_id_batches[i].clone();
        let update_type_clone = update_type.clone();

        let handle = tokio::spawn(async move {
            updater_task(
                db_clone,
                coll_name,
                stats_clone,
                running_clone,
                i,
                id_batch,
                args.doc_size,
                args.indexed,
                update_type_clone,
                update_operation,
                args.full_update_payload,
                stop_on_fail,
                barrier_clone,
            )
            .await
        });
        handles.push(handle);
    }

    // Wait for all updaters to warm up
    println!("\nWaiting for all {} updaters to warm up...", args.workers);
    barrier.wait().await;

    // Start recording
    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!("All updaters ready. Recording for {}s...\n", args.duration);

    record_window(&stats, args.duration, args.cooldown()).await;

    // Stop
    running.store(false, Ordering::Relaxed);

    // Collect and merge histograms
    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Updater error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    // Report
    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}

async fn run_cursor_leak_benchmark(args: Args) -> Result<()> {
    println!("\n================================================================================");
    println!("BENCHLY - CURSOR LEAK BENCHMARK");
    println!("================================================================================");
    println!("  Database:         {}", args.database);
    println!("  Collection:       {}", args.collection);
    println!("  Workers:          {}", args.workers);
    println!("  Duration:         {}s", args.duration);
    println!("  Warmup:           {}s", args.warmup);
    println!("  Preload count:    {}", args.preload_count);
    println!("  Cursor batch size: {}", args.cursor_batch_size);
    println!("  WARNING:          This test intentionally leaks cursors!");
    println!("================================================================================\n");

    let client = create_client(&args).await?;

    let db = client.database(&args.database);
    let collection = db.collection::<Document>(&args.collection);
    let raw_collection = db.collection::<RawDocumentBuf>(&args.collection);

    // Drop and pre-load collection
    if args.should_drop_collection() {
        println!("Dropping collection...");
        let _ = collection.drop().await;
        println!("Pre-loading {} documents...", args.preload_count);

        // Pre-load documents in batches
        let batch_size = 1000;
        let mut loaded = 0;
        while loaded < args.preload_count {
            let remaining = args.preload_count - loaded;
            let current_batch = remaining.min(batch_size);

            let docs: Vec<RawDocumentBuf> = (0..current_batch)
                .map(|_| make_raw_document(args.doc_size, args.indexed))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            raw_collection
                .insert_many(doc_refs)
                .await
                .context("Failed to pre-load documents")?;

            loaded += current_batch;
            if loaded % 50000 == 0 {
                println!("  Loaded {}/{} documents...", loaded, args.preload_count);
            }
        }

        println!("Pre-load complete: {} documents", loaded);

        // Create index if indexed mode
        if args.indexed {
            println!("Creating index on indexed_field...");
            let index = IndexModel::builder()
                .keys(doc! { "indexed_field": 1 })
                .build();
            collection
                .create_index(index)
                .await
                .context("Failed to create index")?;
        }

        println!("Waiting 5s for database to settle...\n");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let metrics = metrics::MetricsSession::start(&args).await;
    let stats = Arc::new(Stats::new());
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(args.workers + 1));

    // Spawn cursor leakers
    let mut handles = Vec::new();
    for i in 0..args.workers {
        let db_clone = db.clone();
        let coll_name = args.collection.clone();
        let stats_clone = stats.clone();
        let running_clone = running.clone();
        let barrier_clone = barrier.clone();
        let stop_on_fail = args.stop_on_failure;
        let batch_size = args.cursor_batch_size;

        let handle = tokio::spawn(async move {
            cursor_leaker_task(
                db_clone,
                coll_name,
                stats_clone,
                running_clone,
                i,
                stop_on_fail,
                barrier_clone,
                batch_size,
            )
            .await
        });
        handles.push(handle);
    }

    // Wait for all leakers to warm up
    println!(
        "Waiting for all {} cursor leakers to warm up...",
        args.workers
    );
    barrier.wait().await;

    // Start recording
    let start_time_str = Utc::now().to_rfc3339();
    stats.start_recording().await;
    println!("All leakers ready. Recording for {}s...\n", args.duration);
    println!(
        "WARNING: Cursors will be created but NOT consumed - this tests cursor leak handling!\n"
    );

    record_window(&stats, args.duration, args.cooldown()).await;

    // Stop
    running.store(false, Ordering::Relaxed);

    // Collect and merge histograms
    let mut merged_hist = Histogram::<u64>::new(3).unwrap();
    for handle in handles {
        match handle.await {
            Ok(Ok(hist)) => {
                merged_hist.add(&hist).ok();
            }
            Ok(Err(e)) => eprintln!("Leaker error: {}", e),
            Err(e) => eprintln!("Task join error: {}", e),
        }
    }

    metrics.finish(None).await;

    // Report
    stats.print_summary(&merged_hist).await;

    let end_time_str = Utc::now().to_rfc3339();
    write_report(&stats, &merged_hist, &args, &start_time_str, &end_time_str).await?;

    std::process::exit(0);
}
