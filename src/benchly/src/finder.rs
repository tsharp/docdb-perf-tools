use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    Database,
    bson::{Document, doc},
    options::FindOptions,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::Barrier;

use crate::stats::Stats;

pub async fn finder_task(
    database: Database,
    collection_name: String,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    worker_id: usize,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
    find_limit: i64,
    batch_size: i64,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    let collection = database.collection::<Document>(&collection_name);

    match consume_find(&collection, find_limit, batch_size).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("[Finder {}] warmup find failed: {}", worker_id, e);
        }
    }
    warmup_barrier.wait().await;

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        match consume_find(&collection, find_limit, batch_size).await {
            Ok(documents_returned) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                if stats.is_recording() {
                    let _ = local_hist.record(latency_ms);
                    let _ = stats.snapshot_hist.lock().await.record(latency_ms);
                    stats.record_op(documents_returned);
                }
            }
            Err(e) => {
                stats.record_failure();
                eprintln!("[Finder {}] find error: {}", worker_id, e);
                if stop_on_failure {
                    running.store(false, Ordering::Relaxed);
                    return Ok(local_hist);
                }
            }
        }

        tokio::task::yield_now().await;
    }

    Ok(local_hist)
}

async fn consume_find(
    collection: &mongodb::Collection<Document>,
    find_limit: i64,
    batch_size: i64,
) -> Result<u64> {
    let options = FindOptions::builder()
        .limit(find_limit)
        .batch_size(batch_size as u32)
        .build();
    let mut cursor = collection.find(doc! {}).with_options(options).await?;
    let mut documents_returned = 0;
    while cursor.advance().await? {
        documents_returned += 1;
    }
    Ok(documents_returned)
}
