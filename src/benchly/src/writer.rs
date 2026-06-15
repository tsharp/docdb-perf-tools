use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    bson::{rawdoc, RawDocumentBuf},
    Collection,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

use crate::stats::Stats;

static GLOBAL_DOC_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn make_raw_document(size: usize, indexed: bool) -> RawDocumentBuf {
    let counter = GLOBAL_DOC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let id = format!("doc_{:012}", counter);
    
    if indexed {
        // With indexed field
        let padding_size = size.saturating_sub(100); // Account for _id and indexed_field
        let padding = "x".repeat(padding_size);
        rawdoc! { 
            "_id": id,
            "indexed_field": counter as i64,
            "payload": padding 
        }
    } else {
        // Non-indexed, just _id and payload
        let padding_size = size.saturating_sub(60); // Account for _id field
        let padding = "x".repeat(padding_size);
        rawdoc! { 
            "_id": id,
            "payload": padding 
        }
    }
}

/// Returns the local histogram with all latency samples from this writer.
pub async fn writer_task(
    collection: Collection<RawDocumentBuf>,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    writer_id: usize,
    doc_size: usize,
    batch_size: usize,
    max_writes_per_sec: i64,
    num_workers: usize,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
    indexed: bool,
) -> Result<Histogram<u64>> {
    let sleep_duration = if max_writes_per_sec > 0 {
        let per_worker_rate = max_writes_per_sec as f64 / num_workers as f64;
        Duration::from_secs_f64(1.0 / per_worker_rate)
    } else {
        Duration::ZERO
    };

    let mut local_hist = Histogram::<u64>::new(3).unwrap();

    // Warmup: do one insert to establish connection, then wait for all workers
    let raw_doc = make_raw_document(doc_size, indexed);
    match collection.insert_one(&raw_doc).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("[Writer {}] warmup insert failed: {}", writer_id, e);
        }
    }
    warmup_barrier.wait().await;

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        if batch_size > 1 {
            let docs: Vec<RawDocumentBuf> = (0..batch_size)
                .map(|_| make_raw_document(doc_size, indexed))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();
            
            match collection.insert_many(doc_refs).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_ms);
                        stats.record_op(batch_size as u64);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Writer {}] insert_many error: {}", writer_id, e);
                    if stop_on_failure {
                        running.store(false, Ordering::Relaxed);
                        return Ok(local_hist);
                    }
                }
            }
        } else {
            let raw_doc = make_raw_document(doc_size, indexed);
            match collection.insert_one(&raw_doc).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_ms);
                        stats.record_op(1);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Writer {}] insert_one error: {}", writer_id, e);
                    if stop_on_failure {
                        running.store(false, Ordering::Relaxed);
                        return Ok(local_hist);
                    }
                }
            }
        }

        if !sleep_duration.is_zero() {
            tokio::time::sleep(sleep_duration).await;
        }
    }

    Ok(local_hist)
}
