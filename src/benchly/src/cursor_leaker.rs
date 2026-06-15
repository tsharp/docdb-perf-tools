use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    bson::doc,
    options::FindOptions,
    Database,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Barrier;

use crate::stats::Stats;

static GLOBAL_CURSOR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Rapidly creates cursors without consuming them - tests cursor leak handling.
/// Returns the local histogram with all latency samples from cursor creation.
pub async fn cursor_leaker_task(
    database: Database,
    collection_name: String,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    worker_id: usize,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
    batch_size: i64,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    let collection = database.collection::<mongodb::bson::Document>(&collection_name);
    
    // Warmup: create one cursor to establish connection
    let warmup_options = FindOptions::builder()
        .batch_size(batch_size as u32)
        .build();
    
    match collection.find(doc! {}).with_options(warmup_options).await {
        Ok(_cursor) => {
            // Intentionally drop cursor without consuming
        }
        Err(e) => {
            eprintln!("[Leaker {}] warmup cursor creation failed: {}", worker_id, e);
        }
    }
    warmup_barrier.wait().await;

    let find_options = FindOptions::builder()
        .batch_size(batch_size as u32)
        .build();

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        match collection.find(doc! {}).with_options(find_options.clone()).await {
            Ok(_cursor) => {
                // Intentionally drop cursor without consuming - this is the leak!
                let cursor_num = GLOBAL_CURSOR_COUNTER.fetch_add(1, Ordering::Relaxed);
                
                let latency_ms = start.elapsed().as_millis() as u64;
                if stats.is_recording() {
                    let _ = local_hist.record(latency_ms);
                    stats.record_op(1);
                }

                // Log every 10000 cursors from worker 0 for visibility
                if worker_id == 0 && cursor_num % 10000 == 0 {
                    println!("[Leaker 0] Created {} cursors (leaked, not consumed)", cursor_num);
                }
            }
            Err(e) => {
                stats.record_failure();
                eprintln!("[Leaker {}] cursor creation error: {}", worker_id, e);
                if stop_on_failure {
                    running.store(false, Ordering::Relaxed);
                    return Ok(local_hist);
                }
            }
        }

        // No yield - run as fast as possible to maximize cursor creation rate
    }

    Ok(local_hist)
}
