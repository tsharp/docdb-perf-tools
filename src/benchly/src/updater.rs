use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    Database,
    bson::{Document, doc},
    options::{FindOneAndUpdateOptions, ReturnDocument},
};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

use crate::stats::Stats;

static GLOBAL_UPDATE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub enum UpdateType {
    /// Simple field update: {$set: {updated_field: value}}
    SetField,
    /// Increment counter: {$inc: {counter: 1}}
    IncrementCounter,
    /// Update multiple fields: {$set: {field1: val1, field2: val2, updated_at: timestamp}}
    SetMultipleFields,
    /// Update with conditional: {$set: {field: val}}, only if condition matches
    ConditionalUpdate,
}

impl UpdateType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "setfield" => Some(UpdateType::SetField),
            "incrementcounter" => Some(UpdateType::IncrementCounter),
            "setmultiplefields" => Some(UpdateType::SetMultipleFields),
            "conditionalupdate" => Some(UpdateType::ConditionalUpdate),
            _ => None,
        }
    }

    pub fn to_string(&self) -> &str {
        match self {
            UpdateType::SetField => "setfield",
            UpdateType::IncrementCounter => "incrementcounter",
            UpdateType::SetMultipleFields => "setmultiplefields",
            UpdateType::ConditionalUpdate => "conditionalupdate",
        }
    }
}

/// Returns the local histogram with all latency samples from this updater.
pub async fn updater_task(
    database: Database,
    collection_name: String,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    worker_id: usize,
    id_batch: Vec<String>,
    update_type: UpdateType,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
    session_duration: Duration,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    let mut rng = SmallRng::seed_from_u64(worker_id as u64);

    let ids = id_batch.clone();

    // Initial collection reference
    let mut collection = database.collection::<Document>(&collection_name);
    let mut session_start = Instant::now();

    // Warmup: do one update to establish connection
    if let Some(id) = ids.first() {
        let update_doc = build_update(&update_type, &mut rng, worker_id);
        match collection
            .find_one_and_update(doc! { "_id": id }, update_doc)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Updater {}] warmup update failed: {}", worker_id, e);
            }
        }
    }
    warmup_barrier.wait().await;

    let mut options = FindOneAndUpdateOptions::default();
    options.return_document = Some(ReturnDocument::After);

    while running.load(Ordering::Relaxed) {
        // Recreate collection reference every session_duration to cycle connections
        if session_start.elapsed() >= session_duration {
            collection = database.collection::<Document>(&collection_name);
            session_start = Instant::now();
            if worker_id == 0 {
                println!(
                    "[Updater 0] Session recycled at {:?}",
                    session_start.elapsed()
                );
            }
        }

        // Randomly select an ID from this worker's batch
        if let Some(id) = ids.choose(&mut rng) {
            let update_doc = build_update(&update_type, &mut rng, worker_id);
            let start = Instant::now();

            match collection
                .find_one_and_update(doc! { "_id": id }, update_doc)
                .with_options(options.clone())
                .await
            {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_ms);
                        stats.record_op(1);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Updater {}] find_one_and_update error: {}", worker_id, e);
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

fn build_update(update_type: &UpdateType, rng: &mut SmallRng, worker_id: usize) -> Document {
    let counter = GLOBAL_UPDATE_COUNTER.fetch_add(1, Ordering::Relaxed);

    match update_type {
        UpdateType::SetField => {
            // Simple field update
            let value = format!("updated_by_worker_{}_at_{}", worker_id, counter);
            doc! {
                "$set": {
                    "updated_field": value,
                    "update_count": counter as i64
                }
            }
        }
        UpdateType::IncrementCounter => {
            // Increment a counter field
            doc! {
                "$inc": {
                    "counter": 1,
                    "worker_updates": 1
                },
                "$set": {
                    "last_worker": worker_id as i32
                }
            }
        }
        UpdateType::SetMultipleFields => {
            // Update multiple fields at once
            let random_value = rng.gen_range(1..=10000);
            doc! {
                "$set": {
                    "field1": format!("value_{}", counter),
                    "field2": random_value,
                    "field3": worker_id as i32,
                    "updated_at": counter as i64,
                    "status": "updated"
                }
            }
        }
        UpdateType::ConditionalUpdate => {
            // Update with current timestamp - simulating conditional logic
            let new_status = if counter % 2 == 0 { "even" } else { "odd" };
            doc! {
                "$set": {
                    "status": new_status,
                    "last_update": counter as i64,
                    "worker_id": worker_id as i32
                },
                "$inc": {
                    "version": 1
                }
            }
        }
    }
}
