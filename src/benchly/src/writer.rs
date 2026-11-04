use anyhow::{Context, Result};
use hdrhistogram::Histogram;
use mongodb::{
    Collection,
    bson::{Bson, Document, RawDocumentBuf, oid::ObjectId, rawdoc},
    error::{ErrorKind, InsertManyError, WriteConcernError, WriteFailure},
    options::{Acknowledgment, ReadPreference, SelectionCriteria},
};
use std::future::IntoFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

use crate::stats::Stats;

static GLOBAL_DOC_COUNTER: AtomicU64 = AtomicU64::new(0);

async fn insert_point(
    collection: &Collection<RawDocumentBuf>,
    document: &RawDocumentBuf,
    client_timeout: Option<Duration>,
    max_time_ms: Option<u64>,
) -> Result<()> {
    let Some(max_time_ms) = max_time_ms else {
        with_insert_timeout(collection.insert_one(document), client_timeout).await?;
        return Ok(());
    };

    // The pinned driver has no maxTimeMS option on insert_one.
    let mut command = rawdoc! {
        "insert": collection.name(),
        "documents": [document.clone()],
        "ordered": true,
        "maxTimeMS": i64::try_from(max_time_ms)?,
    };
    if let Some(write_concern) = collection.write_concern() {
        anyhow::ensure!(
            write_concern.w != Some(Acknowledgment::Nodes(0)),
            "Insert maxTimeMS requires acknowledged writes (w must not be 0)"
        );
        command.append(
            "writeConcern",
            RawDocumentBuf::from_document(&mongodb::bson::to_document(write_concern)?)?,
        );
    }
    let db = collection.client().database(&collection.namespace().db);
    let response = with_insert_timeout(
        db.run_raw_command(command)
            .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary)),
        client_timeout,
    )
    .await?;
    validate_insert_response(&response)
}

fn validate_insert_response(response: &Document) -> Result<()> {
    let has_write_errors = if let Some(errors) = response.get("writeErrors") {
        match errors {
            Bson::Array(errors) => !errors.is_empty(),
            _ => anyhow::bail!("Insert command returned malformed writeErrors: {errors}"),
        }
    } else {
        false
    };
    if has_write_errors || response.contains_key("writeConcernError") {
        let errors: InsertManyError = mongodb::bson::from_document(response.clone())
            .context("Insert command returned malformed write errors")?;
        return Err(mongodb::error::Error::from(ErrorKind::InsertMany(errors)).into());
    }
    anyhow::ensure!(
        matches!(response.get("n"), Some(Bson::Int32(1) | Bson::Int64(1))),
        "Insert command did not acknowledge exactly one document (n={:?})",
        response.get("n")
    );
    Ok(())
}

fn is_timeout_code(code: i32) -> bool {
    matches!(code, 50 | 89 | 262)
}

fn is_write_concern_timeout(error: &WriteConcernError) -> bool {
    is_timeout_code(error.code)
        || (error.code == 64
            && error
                .details
                .as_ref()
                .is_some_and(|details| details.get_bool("wtimeout") == Ok(true)))
}

fn is_insert_timeout(error: &anyhow::Error) -> bool {
    if error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some()
    {
        return true;
    }
    let Some(error) = error.downcast_ref::<mongodb::error::Error>() else {
        return false;
    };
    match error.kind.as_ref() {
        ErrorKind::Command(error) => is_timeout_code(error.code),
        ErrorKind::Io(error) => error.kind() == std::io::ErrorKind::TimedOut,
        ErrorKind::Write(WriteFailure::WriteError(error)) => is_timeout_code(error.code),
        ErrorKind::Write(WriteFailure::WriteConcernError(error)) => is_write_concern_timeout(error),
        ErrorKind::InsertMany(error) => {
            let write_errors = error.write_errors.as_deref().unwrap_or_default();
            // A mixed response must not hide a non-timeout write failure.
            (!write_errors.is_empty() || error.write_concern_error.is_some())
                && write_errors.iter().all(|error| is_timeout_code(error.code))
                && error
                    .write_concern_error
                    .as_ref()
                    .is_none_or(is_write_concern_timeout)
        }
        _ => false,
    }
}

async fn with_insert_timeout<T>(
    operation: impl IntoFuture<Output = mongodb::error::Result<T>>,
    timeout: Option<Duration>,
) -> Result<T> {
    let future = operation.into_future();
    let result = match timeout {
        Some(limit) => tokio::time::timeout(limit, future).await.with_context(|| {
            format!(
                "Insert timed out after {} ms; the write may still have been applied on the server",
                limit.as_millis()
            )
        })?,
        None => future.await,
    };
    Ok(result?)
}

pub fn make_raw_document(size: usize, indexed: bool) -> RawDocumentBuf {
    make_document(size, indexed, false)
}

fn make_document(size: usize, indexed: bool, unique_ids: bool) -> RawDocumentBuf {
    let counter = GLOBAL_DOC_COUNTER.fetch_add(1, Ordering::Relaxed);
    let id = if unique_ids {
        format!("doc_{}", ObjectId::new().to_hex())
    } else {
        format!("doc_{:012}", counter)
    };
    // Keep the BSON size unchanged when using the longer, cross-run unique IDs.
    let extra_id_bytes = if unique_ids { id.len() - 16 } else { 0 };

    if indexed {
        // With indexed field
        let padding_size = size.saturating_sub(100 + extra_id_bytes); // Account for _id and indexed_field
        let padding = "x".repeat(padding_size);
        rawdoc! {
            "_id": id,
            "indexed_field": counter as i64,
            "payload": padding
        }
    } else {
        // Non-indexed, just _id and payload
        let padding_size = size.saturating_sub(60 + extra_id_bytes); // Account for _id field
        let padding = "x".repeat(padding_size);
        rawdoc! {
            "_id": id,
            "payload": padding
        }
    }
}

pub struct WriterResult {
    pub histogram: Histogram<u64>,
    pub error: Option<anyhow::Error>,
}

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
    insert_timeout: Option<Duration>,
    insert_max_time_ms: Option<u64>,
    unique_ids: bool,
) -> WriterResult {
    let sleep_duration = if max_writes_per_sec > 0 {
        let per_worker_rate = max_writes_per_sec as f64 / num_workers as f64;
        Duration::from_secs_f64(1.0 / per_worker_rate)
    } else {
        Duration::ZERO
    };

    let mut local_hist = Histogram::<u64>::new(3).unwrap();

    // Warmup: do one insert to establish connection, then wait for all workers
    let raw_doc = make_document(doc_size, indexed, unique_ids);
    let warmup_error =
        match insert_point(&collection, &raw_doc, insert_timeout, insert_max_time_ms).await {
            Ok(()) => None,
            Err(error) => {
                eprintln!("[Writer {}] warmup insert failed: {}", writer_id, error);
                if stop_on_failure && !is_insert_timeout(&error) {
                    running.store(false, Ordering::Relaxed);
                    Some(error)
                } else {
                    None
                }
            }
        };
    // All workers must reach the barrier, even when their warmup insert fails.
    warmup_barrier.wait().await;
    if let Some(error) = warmup_error {
        return WriterResult {
            histogram: local_hist,
            error: Some(error),
        };
    }

    while running.load(Ordering::Relaxed) {
        let start = Instant::now();

        if batch_size > 1 {
            let docs: Vec<RawDocumentBuf> = (0..batch_size)
                .map(|_| make_document(doc_size, indexed, unique_ids))
                .collect();
            let doc_refs: Vec<&RawDocumentBuf> = docs.iter().collect();

            match with_insert_timeout(collection.insert_many(doc_refs), insert_timeout).await {
                Ok(_) => {
                    let latency_us = start.elapsed().as_micros() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_us);
                        let _ = stats.snapshot_hist.lock().await.record(latency_us);
                        stats.record_latency(latency_us);
                        stats.record_op(batch_size as u64);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Writer {}] insert_many error: {}", writer_id, e);
                    if stop_on_failure && !is_insert_timeout(&e) {
                        running.store(false, Ordering::Relaxed);
                        return WriterResult {
                            histogram: local_hist,
                            error: Some(e),
                        };
                    }
                }
            }
        } else {
            let raw_doc = make_document(doc_size, indexed, unique_ids);
            match insert_point(&collection, &raw_doc, insert_timeout, insert_max_time_ms).await {
                Ok(_) => {
                    let latency_us = start.elapsed().as_micros() as u64;
                    if stats.is_recording() {
                        let _ = local_hist.record(latency_us);
                        let _ = stats.snapshot_hist.lock().await.record(latency_us);
                        stats.record_latency(latency_us);
                        stats.record_op(1);
                    }
                }
                Err(e) => {
                    stats.record_failure();
                    eprintln!("[Writer {}] insert_one error: {}", writer_id, e);
                    if stop_on_failure && !is_insert_timeout(&e) {
                        running.store(false, Ordering::Relaxed);
                        return WriterResult {
                            histogram: local_hist,
                            error: Some(e),
                        };
                    }
                }
            }
        }

        if !sleep_duration.is_zero() {
            tokio::time::sleep(sleep_duration).await;
        }
    }

    WriterResult {
        histogram: local_hist,
        error: None,
    }
}

#[cfg(test)]
mod wire_tests;

#[cfg(test)]
mod tests {
    use super::*;

    async fn unresponsive_collection() -> (Collection<RawDocumentBuf>, tokio::net::TcpListener) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = mongodb::Client::with_uri_str(format!(
            "mongodb://127.0.0.1:{port}/?directConnection=true&serverSelectionTimeoutMS=60000"
        ))
        .await
        .unwrap();
        (client.database("test").collection("inserts"), listener)
    }

    #[tokio::test]
    async fn insert_deadline_expires_for_a_pending_operation() {
        let limit = Duration::from_millis(25);
        let start = Instant::now();
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            with_insert_timeout(
                std::future::pending::<mongodb::error::Result<()>>(),
                Some(limit),
            ),
        )
        .await
        .expect("insert deadline did not fire")
        .unwrap_err();
        assert!(start.elapsed() >= limit);
        assert!(error.to_string().contains("timed out after 25 ms"));
        assert!(error.to_string().contains("may still have been applied"));
    }

    #[tokio::test]
    async fn successful_insert_preserves_its_result_with_and_without_deadline() {
        for timeout in [None, Some(Duration::from_secs(1))] {
            let result = with_insert_timeout(async { Ok::<_, mongodb::error::Error>(42) }, timeout)
                .await
                .unwrap();
            assert_eq!(result, 42);
        }
    }

    #[tokio::test]
    async fn driver_error_is_not_reported_as_a_timeout() {
        let error = with_insert_timeout(
            async { Err::<(), _>(mongodb::error::Error::custom("simulated insert failure")) },
            Some(Duration::from_secs(1)),
        )
        .await
        .unwrap_err();
        assert!(error.downcast_ref::<mongodb::error::Error>().is_some());
        assert!(!error.to_string().contains("timed out"));
    }

    #[test]
    fn io_timeout_classification_uses_the_error_kind_not_message_text() {
        let timeout: anyhow::Error = mongodb::error::Error::from(ErrorKind::Io(Arc::new(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "socket deadline"),
        )))
        .into();
        assert!(is_insert_timeout(&timeout.context("insert failed")));
        let not_timeout: anyhow::Error = mongodb::error::Error::from(ErrorKind::Io(Arc::new(
            std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "timeout in message"),
        )))
        .into();
        assert!(!is_insert_timeout(&not_timeout));
        assert!(!is_insert_timeout(&anyhow::anyhow!("insert timed out")));
    }

    #[tokio::test]
    async fn warmup_timeout_releases_barrier_and_keeps_writer_running() {
        let (collection, _listener) = unresponsive_collection().await;
        let running = Arc::new(AtomicBool::new(true));
        let barrier = Arc::new(Barrier::new(2));
        let handle = tokio::spawn(writer_task(
            collection,
            Arc::new(Stats::new()),
            running.clone(),
            0,
            1024,
            0,
            -1,
            1,
            true,
            barrier.clone(),
            false,
            Some(Duration::from_millis(25)),
            None,
            false,
        ));
        tokio::time::timeout(Duration::from_secs(2), barrier.wait())
            .await
            .expect("warmup timeout deadlocked the barrier");
        assert!(running.load(Ordering::Relaxed));
        assert!(!handle.is_finished());
        running.store(false, Ordering::Relaxed);
        let result = tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("writer did not stop at the end of the run")
            .unwrap();
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn timed_out_inserts_are_failures_not_successful_operations() {
        for batch_size in [0, 3] {
            let (collection, _listener) = unresponsive_collection().await;
            let running = Arc::new(AtomicBool::new(true));
            let barrier = Arc::new(Barrier::new(2));
            let stats = Arc::new(Stats::new());
            let handle = tokio::spawn(writer_task(
                collection,
                stats.clone(),
                running.clone(),
                0,
                1024,
                batch_size,
                -1,
                1,
                true,
                barrier.clone(),
                false,
                Some(Duration::from_millis(25)),
                None,
                false,
            ));
            tokio::time::timeout(Duration::from_secs(2), barrier.wait())
                .await
                .expect("warmup barrier did not complete");
            stats.start_recording().await;
            tokio::time::timeout(Duration::from_secs(2), async {
                while stats.get_current_counts().2 < 2 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("insert deadlines did not allow the workload to continue");
            assert!(running.load(Ordering::Relaxed));
            running.store(false, Ordering::Relaxed);
            let result = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("writer did not stop")
                .unwrap();
            let (ops, docs, failures) = stats.get_current_counts();
            assert_eq!(ops, 0);
            assert_eq!(docs, 0);
            assert!(failures >= 2);
            assert!(result.error.is_none());
            assert!(result.histogram.is_empty());
        }
    }
}
