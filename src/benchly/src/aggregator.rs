use anyhow::Result;
use hdrhistogram::Histogram;
use mongodb::{
    bson::{doc, Document},
    Database,
};
use rand::Rng;
use rand::{SeedableRng, rngs::SmallRng};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

use crate::stats::Stats;

#[derive(Debug, Clone)]
pub enum AggregationType {
    /// Simple count aggregation: db.collection.aggregate([{$count: "total"}])
    Count,
    /// Group by indexed_field with count: [{$group: {_id: "$indexed_field", count: {$sum: 1}}}]
    GroupByCount,
    /// Match range and count: [{$match: {indexed_field: {$gte: X, $lte: Y}}}, {$count: "total"}]
    MatchRangeCount,
    /// Group by modulo with sum: [{$group: {_id: {$mod: ["$indexed_field", 100]}, total: {$sum: "$indexed_field"}}}]
    GroupByModSum,
    /// Sort and limit: [{$sort: {indexed_field: 1}}, {$limit: 100}]
    SortLimit,
    /// Match, group, and sort: [{$match: {...}}, {$group: {...}}, {$sort: {count: -1}}, {$limit: 10}]
    MatchGroupSort,
}

impl AggregationType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "count" => Some(AggregationType::Count),
            "groupbycount" => Some(AggregationType::GroupByCount),
            "matchrangecount" => Some(AggregationType::MatchRangeCount),
            "groupbymodsum" => Some(AggregationType::GroupByModSum),
            "sortlimit" => Some(AggregationType::SortLimit),
            "matchgroupsort" => Some(AggregationType::MatchGroupSort),
            _ => None,
        }
    }

    pub fn to_string(&self) -> &str {
        match self {
            AggregationType::Count => "count",
            AggregationType::GroupByCount => "groupbycount",
            AggregationType::MatchRangeCount => "matchrangecount",
            AggregationType::GroupByModSum => "groupbymodsum",
            AggregationType::SortLimit => "sortlimit",
            AggregationType::MatchGroupSort => "matchgroupsort",
        }
    }
}

/// Returns the local histogram with all latency samples from this aggregator.
pub async fn aggregator_task(
    database: Database,
    collection_name: String,
    stats: Arc<Stats>,
    running: Arc<AtomicBool>,
    worker_id: usize,
    agg_type: AggregationType,
    preload_count: usize,
    stop_on_failure: bool,
    warmup_barrier: Arc<Barrier>,
    session_duration: Duration,
) -> Result<Histogram<u64>> {
    let mut local_hist = Histogram::<u64>::new(3).unwrap();
    let mut rng = SmallRng::seed_from_u64(worker_id as u64);

    // Initial collection reference
    let mut collection = database.collection::<Document>(&collection_name);
    let mut session_start = Instant::now();

    // Warmup: run one aggregation to establish connection
    let warmup_pipeline = build_pipeline(&agg_type, &mut rng, preload_count);
    match collection.aggregate(warmup_pipeline).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("[Aggregator {}] warmup aggregation failed: {}", worker_id, e);
        }
    }
    warmup_barrier.wait().await;

    while running.load(Ordering::Relaxed) {
        // Recreate collection reference every session_duration to cycle connections
        if session_start.elapsed() >= session_duration {
            collection = database.collection::<Document>(&collection_name);
            session_start = Instant::now();
            if worker_id == 0 {
                println!("[Aggregator 0] Session recycled at {:?}", session_start.elapsed());
            }
        }

        let pipeline = build_pipeline(&agg_type, &mut rng, preload_count);
        let start = Instant::now();

        match collection.aggregate(pipeline).await {
            Ok(mut cursor) => {
                // Consume the cursor to ensure full execution
                while let Ok(true) = cursor.advance().await {
                    // Processing results
                }
                
                let latency_ms = start.elapsed().as_millis() as u64;
                if stats.is_recording() {
                    let _ = local_hist.record(latency_ms);
                    stats.record_op(1);
                }
            }
            Err(e) => {
                stats.record_failure();
                eprintln!("[Aggregator {}] aggregation error: {}", worker_id, e);
                if stop_on_failure {
                    running.store(false, Ordering::Relaxed);
                    return Ok(local_hist);
                }
            }
        }

        // Small yield to prevent CPU spinning
        tokio::task::yield_now().await;
    }

    Ok(local_hist)
}

fn build_pipeline(
    agg_type: &AggregationType,
    rng: &mut SmallRng,
    preload_count: usize,
) -> Vec<Document> {
    match agg_type {
        AggregationType::Count => {
            vec![doc! { "$count": "total" }]
        }
        AggregationType::GroupByCount => {
            vec![
                doc! {
                    "$group": {
                        "_id": "$indexed_field",
                        "count": { "$sum": 1 }
                    }
                }
            ]
        }
        AggregationType::MatchRangeCount => {
            // Random range of ~10% of total documents
            let range_size = (preload_count / 10).max(1000);
            let max_start = preload_count.saturating_sub(range_size);
            let start = rng.gen_range(0..=max_start);
            let end = start + range_size;
            
            vec![
                doc! {
                    "$match": {
                        "indexed_field": {
                            "$gte": start as i64,
                            "$lte": end as i64
                        }
                    }
                },
                doc! { "$count": "total" }
            ]
        }
        AggregationType::GroupByModSum => {
            // Group by indexed_field mod 100 and sum
            vec![
                doc! {
                    "$group": {
                        "_id": { "$mod": ["$indexed_field", 100] },
                        "total": { "$sum": "$indexed_field" },
                        "count": { "$sum": 1 }
                    }
                }
            ]
        }
        AggregationType::SortLimit => {
            let limit = rng.gen_range(10..=100);
            vec![
                doc! { "$sort": { "indexed_field": 1 } },
                doc! { "$limit": limit }
            ]
        }
        AggregationType::MatchGroupSort => {
            // Complex pipeline: match a range, group by mod, sort by count, limit
            let range_size = (preload_count / 5).max(5000);
            let max_start = preload_count.saturating_sub(range_size);
            let start = rng.gen_range(0..=max_start);
            let end = start + range_size;
            
            vec![
                doc! {
                    "$match": {
                        "indexed_field": {
                            "$gte": start as i64,
                            "$lte": end as i64
                        }
                    }
                },
                doc! {
                    "$group": {
                        "_id": { "$mod": ["$indexed_field", 50] },
                        "count": { "$sum": 1 },
                        "avg_value": { "$avg": "$indexed_field" }
                    }
                },
                doc! { "$sort": { "count": -1 } },
                doc! { "$limit": 10 }
            ]
        }
    }
}
