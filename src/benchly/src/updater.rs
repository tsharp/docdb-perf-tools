use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    Database,
    bson::{Document, doc},
    options::ReturnDocument,
};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng, rngs::SmallRng};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
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

#[derive(Debug, Clone, Copy)]
pub enum UpdateOperation {
    UpdateOne,
    FindOneAndUpdate,
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
    doc_size: usize,
    indexed: bool,
    update_type: UpdateType,
    update_operation: UpdateOperation,
    full_update_payload: bool,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    let mut rng = SmallRng::seed_from_u64(worker_id as u64);

    let ids = id_batch.clone();
    let payload_padding_size = if indexed {
        doc_size.saturating_sub(100)
    } else {
        doc_size.saturating_sub(60)
    };
    let payload_template = "x".repeat(payload_padding_size);

    // Initial collection reference
    let collection = database.collection::<Document>(&collection_name);

    // Warmup: do one update to establish connection
    if let Some(id) = ids.first() {
        let warmup_result = match update_operation {
            UpdateOperation::UpdateOne => {
                if full_update_payload {
                    let replacement_doc = build_full_replacement(
                        &update_type,
                        &mut rng,
                        worker_id,
                        id,
                        &payload_template,
                        indexed,
                    );
                    collection
                        .replace_one(doc! { "_id": id }, replacement_doc)
                        .await
                        .map(|_| ())
                } else {
                    let update_doc = build_update(&update_type, &mut rng, worker_id);
                    collection
                        .update_one(doc! { "_id": id }, update_doc)
                        .await
                        .map(|_| ())
                }
            }
            UpdateOperation::FindOneAndUpdate => {
                if full_update_payload {
                    let replacement_doc = build_full_replacement(
                        &update_type,
                        &mut rng,
                        worker_id,
                        id,
                        &payload_template,
                        indexed,
                    );
                    collection
                        .find_one_and_replace(doc! { "_id": id }, replacement_doc)
                        .projection(doc! { "_id": 1 })
                        .return_document(ReturnDocument::After)
                        .await
                        .map(|_| ())
                } else {
                    let update_doc = build_update(&update_type, &mut rng, worker_id);
                    collection
                        .find_one_and_update(doc! { "_id": id }, update_doc)
                        .await
                        .map(|_| ())
                }
            }
        };

        match warmup_result {
            Ok(_) => {}
            Err(e) => {
                eprintln!("[Updater {}] warmup update failed: {}", worker_id, e);
            }
        }
    }
    warmup_barrier.wait().await;

    while running.load(Ordering::Relaxed) {
        // Randomly select an ID from this worker's batch
        if let Some(id) = ids.choose(&mut rng) {
            let start = Instant::now();

            let update_result = match update_operation {
                UpdateOperation::UpdateOne => {
                    if full_update_payload {
                        let replacement_doc = build_full_replacement(
                            &update_type,
                            &mut rng,
                            worker_id,
                            id,
                            &payload_template,
                            indexed,
                        );
                        collection
                            .replace_one(doc! { "_id": id }, replacement_doc)
                            .await
                            .map(|_| ())
                    } else {
                        let update_doc = build_update(&update_type, &mut rng, worker_id);
                        collection
                            .update_one(doc! { "_id": id }, update_doc)
                            .await
                            .map(|_| ())
                    }
                }
                UpdateOperation::FindOneAndUpdate => {
                    if full_update_payload {
                        let replacement_doc = build_full_replacement(
                            &update_type,
                            &mut rng,
                            worker_id,
                            id,
                            &payload_template,
                            indexed,
                        );
                        collection
                            .find_one_and_replace(doc! { "_id": id }, replacement_doc)
                            .projection(doc! { "_id": 1 })
                            .return_document(ReturnDocument::After)
                            .await
                            .map(|_| ())
                    } else {
                        let update_doc = build_update(&update_type, &mut rng, worker_id);
                        collection
                            .find_one_and_update(doc! { "_id": id }, update_doc)
                            .await
                            .map(|_| ())
                    }
                }
            };

            match update_result {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_ms);
                        let _ = stats.snapshot_hist.lock().await.record(latency_ms);
                        stats.record_latency(latency_ms);
                        stats.record_op(1);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    let op_name = match update_operation {
                        UpdateOperation::UpdateOne => "update_one",
                        UpdateOperation::FindOneAndUpdate => "find_one_and_update",
                    };
                    eprintln!("[Updater {}] {} error: {}", worker_id, op_name, e);
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

fn build_full_replacement(
    update_type: &UpdateType,
    rng: &mut SmallRng,
    worker_id: usize,
    id: &str,
    payload_template: &str,
    indexed: bool,
) -> Document {
    let counter = GLOBAL_UPDATE_COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut replacement = doc! {
        "_id": id,
        "payload": payload_template
    };

    if indexed {
        replacement.insert("indexed_field", parse_doc_counter(id));
    }

    match update_type {
        UpdateType::SetField => {
            replacement.insert(
                "updated_field",
                format!("updated_by_worker_{}_at_{}", worker_id, counter),
            );
            replacement.insert("update_count", counter as i64);
        }
        UpdateType::IncrementCounter => {
            replacement.insert("counter", counter as i64);
            replacement.insert("worker_updates", counter as i64);
            replacement.insert("last_worker", worker_id as i32);
        }
        UpdateType::SetMultipleFields => {
            replacement.insert("field1", format!("value_{}", counter));
            replacement.insert("field2", rng.gen_range(1..=10000));
            replacement.insert("field3", worker_id as i32);
            replacement.insert("updated_at", counter as i64);
            replacement.insert("status", "updated");
        }
        UpdateType::ConditionalUpdate => {
            let new_status = if counter % 2 == 0 { "even" } else { "odd" };
            replacement.insert("status", new_status);
            replacement.insert("last_update", counter as i64);
            replacement.insert("worker_id", worker_id as i32);
            replacement.insert("version", counter as i64);
        }
    }

    replacement
}

fn parse_doc_counter(id: &str) -> i64 {
    id.strip_prefix("doc_")
        .and_then(|suffix| suffix.parse::<i64>().ok())
        .unwrap_or(0)
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
