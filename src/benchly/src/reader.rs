use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    Collection,
    bson::{Document, doc},
};
use rand::seq::SliceRandom;
use rand::{SeedableRng, rngs::SmallRng};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::Barrier;

use crate::stats::Stats;

/// Returns the local histogram with all latency samples from this reader.
pub async fn reader_task(
    collection: Collection<Document>,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    worker_id: usize,
    id_batch: Vec<String>,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    // Use SmallRng which is Send-safe, seeded with worker_id for reproducibility
    let mut rng = SmallRng::seed_from_u64(worker_id as u64);

    // Clone ID batch for random selection
    let ids = id_batch.clone();

    // Warmup: do one read to establish connection, then wait for all workers
    if let Some(id) = ids.first() {
        match collection.find_one(doc! { "_id": id }).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Reader {}] warmup read failed: {}", worker_id, e);
            }
        }
    }
    warmup_barrier.wait().await;

    while running.load(Ordering::Relaxed) {
        // Randomly select an ID from this worker's batch
        if let Some(id) = ids.choose(&mut rng) {
            let start = Instant::now();

            match collection.find_one(doc! { "_id": id }).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_ms);
                        stats.record_op(1);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Reader {}] find_one error: {}", worker_id, e);
                    if stop_on_failure {
                        running.store(false, Ordering::Relaxed);
                        return Ok(local_hist);
                    }
                }
            }
        }

        // Small yield to prevent CPU spinning
        tokio::task::yield_now().await;
    }

    Ok(local_hist)
}
