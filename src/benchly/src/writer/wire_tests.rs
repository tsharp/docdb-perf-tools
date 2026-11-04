use super::*;
use mongodb::bson::doc;
use std::{collections::VecDeque, sync::Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};

struct WireServer {
    port: u16,
    inserts: mpsc::UnboundedReceiver<Document>,
    task: JoinHandle<()>,
}

impl Drop for WireServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl WireServer {
    async fn start(response: Document) -> Self {
        Self::start_sequence(vec![response]).await
    }

    async fn start_sequence(responses: Vec<Document>) -> Self {
        assert!(!responses.is_empty());
        let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, inserts) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (socket, _) = connection.unwrap();
                        connections.spawn(serve_connection(socket, sender.clone(), responses.clone()));
                    }
                    result = connections.join_next(), if !connections.is_empty() => {
                        result.unwrap().unwrap();
                    }
                }
            }
        });
        Self {
            port,
            inserts,
            task,
        }
    }

    async fn collection(&self, options: &str) -> Collection<RawDocumentBuf> {
        mongodb::Client::with_uri_str(format!(
            "mongodb://127.0.0.1:{}/?directConnection=true&retryWrites=false&serverSelectionTimeoutMS=1000{options}",
            self.port
        ))
        .await
        .unwrap()
        .database("wire_test")
        .collection("inserts")
    }

    async fn next_insert(&mut self) -> Document {
        tokio::time::timeout(Duration::from_secs(2), self.inserts.recv())
            .await
            .expect("insert did not reach the wire server")
            .unwrap()
    }
}

async fn serve_connection(
    mut socket: TcpStream,
    inserts: mpsc::UnboundedSender<Document>,
    responses: Arc<Mutex<VecDeque<Document>>>,
) {
    loop {
        let length = match socket.read_i32_le().await {
            Ok(length) => length,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return,
            Err(error) => panic!("wire read failed: {error}"),
        };
        assert!((16..=48_000_000).contains(&length));
        let request_id = socket.read_i32_le().await.unwrap();
        let _response_to = socket.read_i32_le().await.unwrap();
        let opcode = socket.read_i32_le().await.unwrap();
        let mut body = vec![0; length as usize - 16];
        socket.read_exact(&mut body).await.unwrap();

        // The driver handshakes with OP_QUERY; commands use OP_MSG.
        let document_offset = match opcode {
            2004 => 4 + body[4..].iter().position(|byte| *byte == 0).unwrap() + 1 + 8,
            2013 => {
                assert_eq!(body[4], 0);
                5
            }
            other => panic!("unexpected wire opcode {other}"),
        };
        let command = Document::from_reader(&mut &body[document_offset..]).unwrap();
        let reply = if command.contains_key("insert") {
            inserts.send(command).unwrap();
            let mut responses = responses.lock().unwrap();
            if responses.len() > 1 {
                responses.pop_front().unwrap()
            } else {
                responses.front().unwrap().clone()
            }
        } else {
            doc! {
                "ok": 1.0, "ismaster": true, "isWritablePrimary": true,
                "minWireVersion": 0, "maxWireVersion": 17,
                "maxBsonObjectSize": 16_777_216, "maxMessageSizeBytes": 48_000_000,
                "maxWriteBatchSize": 100_000,
            }
        };
        let mut payload = Vec::new();
        if opcode == 2004 {
            payload.extend(0_i32.to_le_bytes()); // OP_REPLY flags
            payload.extend(0_i64.to_le_bytes()); // cursor ID
            payload.extend(0_i32.to_le_bytes()); // startingFrom
            payload.extend(1_i32.to_le_bytes()); // numberReturned
        } else {
            payload.extend(0_i32.to_le_bytes()); // OP_MSG flags
            payload.push(0); // BSON body section
        }
        reply.to_writer(&mut payload).unwrap();
        socket
            .write_i32_le((16 + payload.len()) as i32)
            .await
            .unwrap();
        socket.write_i32_le(1).await.unwrap();
        socket.write_i32_le(request_id).await.unwrap();
        socket
            .write_i32_le(if opcode == 2004 { 1 } else { 2013 })
            .await
            .unwrap();
        socket.write_all(&payload).await.unwrap();
    }
}

#[tokio::test]
async fn point_insert_sends_max_time_and_preserves_write_concern() {
    let mut server = WireServer::start(doc! { "ok": 1.0, "n": 1 }).await;
    let collection = server.collection("&w=majority&wTimeoutMS=900").await;
    let document = rawdoc! { "_id": "wire-id", "payload": "test" };
    insert_point(
        &collection,
        &document,
        Some(Duration::from_secs(2)),
        Some(321),
    )
    .await
    .unwrap();
    let command = server.next_insert().await;
    assert_eq!(command.get_i64("maxTimeMS").unwrap(), 321);
    assert_eq!(command.get_str("insert").unwrap(), "inserts");
    assert_eq!(command.get_str("$db").unwrap(), "wire_test");
    assert!(command.get_bool("ordered").unwrap());
    assert_eq!(
        command.get_array("documents").unwrap(),
        &vec![Bson::Document(doc! {
            "_id": "wire-id", "payload": "test",
        })]
    );
    let concern = command.get_document("writeConcern").unwrap();
    assert_eq!(concern.get_str("w").unwrap(), "majority");
    assert!(matches!(
        concern.get("wtimeout"),
        Some(Bson::Int32(900) | Bson::Int64(900))
    ));
}

#[tokio::test]
async fn omitted_wire_timeout_keeps_native_insert_path() {
    let mut server = WireServer::start(doc! { "ok": 1.0, "n": 1 }).await;
    insert_point(
        &server.collection("").await,
        &rawdoc! { "_id": "native" },
        Some(Duration::from_secs(2)),
        None,
    )
    .await
    .unwrap();
    assert!(!server.next_insert().await.contains_key("maxTimeMS"));
}

#[tokio::test]
async fn non_timeout_server_errors_still_fail_warmup_and_release_workers() {
    for (response, expected) in [
        (
            doc! { "ok": 0.0, "code": 13, "errmsg": "unauthorized" },
            "unauthorized",
        ),
        (
            doc! { "ok": 0.0, "code": 9, "errmsg": "unsupported maxTimeMS" },
            "unsupported maxTimeMS",
        ),
        (
            doc! { "ok": 1.0, "n": 0, "writeErrors": [{ "index": 0, "code": 11000, "errmsg": "duplicate key" }] },
            "duplicate key",
        ),
        (
            doc! { "ok": 1.0, "n": 1, "writeConcernError": { "code": 64, "errmsg": "not a timeout" } },
            "not a timeout",
        ),
        (
            doc! { "ok": 1.0, "n": 0, "writeErrors": [
                { "index": 0, "code": 50, "errmsg": "timeout" },
                { "index": 1, "code": 11000, "errmsg": "duplicate key" }
            ] },
            "duplicate key",
        ),
        (doc! { "ok": 1.0, "n": 0 }, "exactly one document"),
        (doc! { "ok": 1.0 }, "exactly one document"),
    ] {
        let mut server = WireServer::start(response).await;
        let running = Arc::new(AtomicBool::new(true));
        let barrier = Arc::new(Barrier::new(2));
        let collection = server.collection("").await;
        let stats = Arc::new(Stats::new());
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            let (result, _) = tokio::join!(
                writer_task(
                    collection,
                    stats.clone(),
                    running.clone(),
                    0,
                    1024,
                    0,
                    -1,
                    1,
                    true,
                    barrier.clone(),
                    false,
                    Some(Duration::from_secs(2)),
                    Some(100),
                    false,
                ),
                barrier.wait()
            );
            result
        })
        .await
        .expect("server error blocked warmup");
        assert!(result.error.unwrap().to_string().contains(expected));
        assert!(!running.load(Ordering::Relaxed));
        assert_eq!(stats.get_current_counts().0, 0);
        assert_eq!(
            server.next_insert().await.get_i64("maxTimeMS").unwrap(),
            100
        );
    }
}

#[tokio::test]
async fn server_timeouts_allow_warmup_and_measured_inserts_to_continue() {
    let responses = [
        doc! { "ok": 0.0, "code": 50, "errmsg": "server time limit expired" },
        doc! { "ok": 0.0, "code": 262, "errmsg": "operation time limit exceeded" },
        doc! { "ok": 0.0, "code": 89, "errmsg": "network timeout" },
        doc! { "ok": 1.0, "n": 0, "writeErrors": [{ "index": 0, "code": 50, "errmsg": "write timeout" }] },
        doc! { "ok": 1.0, "n": 1, "writeConcernError": {
            "code": 64, "errmsg": "ack timeout", "errInfo": { "wtimeout": true }
        } },
    ];
    for (max_time_ms, batch_size) in [(Some(100), 0), (None, 0), (None, 3)] {
        for response in &responses {
            let mut server = WireServer::start_sequence(vec![
                response.clone(),
                response.clone(),
                doc! { "ok": 1.0, "n": if batch_size > 1 { batch_size as i32 } else { 1 } },
            ])
            .await;
            let running = Arc::new(AtomicBool::new(true));
            let barrier = Arc::new(Barrier::new(2));
            let stats = Arc::new(Stats::new());
            stats.start_recording().await;
            let handle = tokio::spawn(writer_task(
                server.collection("").await,
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
                Some(Duration::from_secs(2)),
                max_time_ms,
                false,
            ));
            tokio::time::timeout(Duration::from_secs(3), async {
                barrier.wait().await;
                loop {
                    let (ops, _, failures) = stats.get_current_counts();
                    if ops >= 2 && failures >= 1 {
                        break;
                    }
                    assert!(running.load(Ordering::Relaxed), "timeout stopped the run");
                    assert!(!handle.is_finished(), "timeout stopped the worker");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("inserts did not recover after the timeout");
            assert!(running.load(Ordering::Relaxed));
            running.store(false, Ordering::Relaxed);
            let result = tokio::time::timeout(Duration::from_secs(2), handle)
                .await
                .expect("writer did not finish normally")
                .unwrap();
            assert!(result.error.is_none());
            let (ops, docs, failures) = stats.get_current_counts();
            assert_eq!(failures, 1);
            assert_eq!(docs, ops * batch_size.max(1) as u64);
            assert_eq!(result.histogram.len(), ops);
            if max_time_ms.is_some() {
                let warmup = server.next_insert().await;
                let timed_out = server.next_insert().await;
                let recovered = server.next_insert().await;
                assert_ne!(
                    warmup.get_array("documents").unwrap(),
                    timed_out.get_array("documents").unwrap()
                );
                assert_ne!(
                    timed_out.get_array("documents").unwrap(),
                    recovered.get_array("documents").unwrap()
                );
            }
        }
    }
}

#[tokio::test]
async fn non_timeout_error_still_stops_a_measured_run() {
    let server = WireServer::start_sequence(vec![
        doc! { "ok": 1.0, "n": 1 },
        doc! { "ok": 1.0, "n": 0, "writeErrors": [{ "index": 0, "code": 11000, "errmsg": "duplicate key" }] },
    ]).await;
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(2));
    let stats = Arc::new(Stats::new());
    stats.start_recording().await;
    let collection = server.collection("").await;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let (result, _) = tokio::join!(
            writer_task(
                collection,
                stats.clone(),
                running.clone(),
                0,
                1024,
                0,
                -1,
                1,
                true,
                barrier.clone(),
                false,
                Some(Duration::from_secs(2)),
                Some(100),
                false,
            ),
            barrier.wait(),
        );
        result
    })
    .await
    .expect("non-timeout error did not stop the run");
    assert!(result.error.unwrap().to_string().contains("duplicate key"));
    assert!(!running.load(Ordering::Relaxed));
    assert_eq!(stats.get_current_counts(), (0, 0, 1));
}

#[tokio::test]
async fn unacknowledged_writes_are_rejected_before_sending_insert() {
    let mut server = WireServer::start(doc! { "ok": 1.0, "n": 1 }).await;
    let error = insert_point(
        &server.collection("&w=0").await,
        &rawdoc! { "_id": "unacknowledged" },
        Some(Duration::from_secs(2)),
        Some(100),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("acknowledged writes"));
    assert!(server.inserts.try_recv().is_err());
}
