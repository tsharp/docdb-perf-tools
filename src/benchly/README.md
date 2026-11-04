# Benchly - MongoDB Performance Benchmarking Tool

A high-performance, Rust-based MongoDB benchmarking tool with support for multiple workload types.

## Workload Types

Benchly supports six main modes:

### 1. Write Workload (`--test write`)
Tests write performance using `insert_one` or `insert_many` operations.

**Features:**
- Configurable document size and batch size
- Rate limiting support (max writes per second)
- Support for indexed/non-indexed documents
- Concurrent writers with connection pooling
- Optional client-side per-insert deadline (`--insert-timeout-ms`)
- Optional wire-protocol execution limit for point inserts (`--insert-max-time-ms`)

**Quick Start:**
```bash
./run-bench.ps1 -Workers 8 -DocSize 1024 -Duration 120
```

See existing PowerShell scripts for more examples.

For write workloads, `--no-drop-collection` appends to the collection without
dropping it or changing its indexes. It uses unique string IDs (`doc_` plus
ObjectId hex) for warmup, point, and batched inserts to avoid ID collisions
between processes/runs. Payload padding accounts for the longer IDs where the
requested size allows. Default drop/recreate workloads and preload retain the
sequential `doc_000000000000` ID format. With no-drop, `--indexed` adds fields
but does not create an index.

Use `--insert-timeout-ms 5000` to bound each insert call to 5 seconds, including
server selection and pool wait. It applies to worker warmup and to the entire
`insert_many` call when batching. Omit the option to preserve the driver's
configured timeout behavior. Startup ping and collection/index setup are not
covered. A timed-out call may still have been applied on the server and is not
retried. Insert timeouts are logged and counted as measurement failures, but
workers continue with new documents even with `--stop-on-failure`. Warmup
timeouts are logged without aborting startup and are excluded from report counts.
Timeouts alone do not cause a nonzero exit. With `--stop-on-failure`, other
insert errors still stop the write run and produce a nonzero exit code after the
report is written. Without it, all insert failures are counted and the workload
continues. Timeout configuration is recorded in report
metadata as `benchly.insert_timeout_ms`.

Use `--insert-max-time-ms 5000` to send `maxTimeMS: 5000` in an explicit
single-document `insert` command. This requires `--test write` and
`--batch-size 0` or `1`. It covers worker warmup and point inserts, not setup
commands. The pinned driver's `insert_one` options lack `maxTimeMS`, so this
opt-in path uses `run_raw_command`, preserves configured write concern, and
validates `writeErrors`, `writeConcernError`, and `n`. Unacknowledged writes
(`w=0`) are rejected. Omit the option for the original driver insert path.

The gateway/server must honor `maxTimeMS` to enforce the execution limit.
This is distinct from `writeConcern.wtimeout`, which concerns acknowledgement.
Server timeout errors are reported as failures without retrying the same
document or stopping the run. Unsupported-option errors are not timeouts and
still stop the run with `--stop-on-failure`. Timeout classification uses typed
client deadlines/I/O timeouts, server codes (`50`, `89`, `262`), and write-concern
code `64` with `errInfo.wtimeout: true`, not message text. Responses containing
any non-timeout write error remain fatal under `--stop-on-failure`. Startup
connection checks and collection/index setup errors remain fatal.
Keep the optional client deadline higher than this wire limit to
allow connection and response overhead. Metadata includes
`benchly.insert_max_time_ms`. A timeout does not guarantee rollback.

---

### 2. Read Workload (`--test read`)
Tests point read performance using `find_one` operations.

**Features:**
- Pre-loads documents for reading
- Partitions document IDs among workers (non-overlapping)
- Random selection from worker-specific batches
- Support for indexed/non-indexed documents

**Quick Start:**
```bash
./target/release/benchly \
  --test read \
  --mongodb-url "mongodb://..." \
  --workers 8 \
  --preload-count 250000 \
  --duration 120 \
  --output-dir ./bench-results \
  --cluster-name my-cluster
```

---

### 3. Aggregate Workload (`--test aggregate`)
Tests aggregation pipeline performance with various query patterns.

**Features:**
- 6 different aggregation types (count, groupbycount, matchrangecount, etc.)
- Random range generation for realistic workloads
- Automatic index creation on `indexed_field`
- Complex multi-stage pipeline testing

**Quick Start:**
```bash
./run-aggregate.ps1 -AggregationType matchgroupsort -Workers 8
```

**Available Types:** count, groupbycount, matchrangecount, groupbymodsum, sortlimit, matchgroupsort

📖 **[Full Documentation](AGGREGATE-WORKLOAD.md)**

---

### 4. Update Workload (`--test update`)
Tests update performance using pure `updateOne` operations.

**Features:**
- 4 different update patterns (setfield, incrementcounter, etc.)
- Non-overlapping document partitions per worker
- Atomic increment and conditional update support

### 5. Find-And-Update Workload (`--test find_and_update`)
Tests update performance using `findOneAndUpdate` operations.

**Features:**
- Same update patterns as `update`
- Non-overlapping document partitions per worker
- Returns updated documents as part of each operation

**Quick Start:**
```bash
./run-update.ps1 -UpdateType setfield -Workers 8
```

**Available Types:** setfield, incrementcounter, setmultiplefields, conditionalupdate

📖 **[Full Documentation](UPDATE-WORKLOAD.md)**

---

### 6. Cursor Leak Workload (`--test leak_cursor`)
⚠️ Stress tests cursor leak handling by rapidly creating cursors without consuming them.

**Features:**
- Creates cursors as fast as possible
- Intentionally drops cursors without consuming (leak test)
- Configurable cursor batch size
- Tests server-side cursor cleanup and timeouts

**Quick Start:**
```bash
./run-leak-cursor.ps1 -Workers 8 -CursorBatchSize 101
```

**⚠️ WARNING:** This test intentionally leaks cursors to stress test cleanup mechanisms. Use with caution!

📖 **[Full Documentation](CURSOR-LEAK-WORKLOAD.md)**

---

### 7. Server Info (`--test server_info`)
Prints general server metadata without running a benchmark workload.

**Fields shown:**
- Server version and git version
- OpenSSL version
- Minimum and maximum wire version
- Maximum BSON object size and message size
- Replica set hosts, set name, and primary
- Logical session timeout

**Quick Start:**
```bash
./target/release/benchly \
  --test server_info \
  --mongodb-url-file ../../secrets/m80.secret
```

---

## Common Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--mongodb-url` | (required) | MongoDB connection string |
| `--test` | `write` | Workload type: write, read, find, update, find_and_update, aggregate, leak_cursor, server_info |
| `--workers` | `8` | Number of concurrent workers |
| `--duration` | `120` | Test duration in seconds |
| `--warmup` | `5` | Warmup period before recording |
| `--doc-size` | `1024` | Document size in bytes |
| `--database` | `benchmark_db` | Database name |
| `--collection` | `benchly_test` | Collection name |
| `--drop-collection` | flag | Drop collection before test |
| `--stop-on-failure` | flag | Stop all workers on first error; insert timeouts are counted but do not stop write workloads |
| `--output-dir` | (optional) | Directory for JSON reports |
| `--cluster-name` | (optional) | Cluster identifier for reports |
| `--run-label` | varies | Label for this test run |

### Workload-Specific Parameters

**Read/Update/Aggregate/Leak_Cursor:**
- `--preload-count` (default: 250000) - Number of documents to pre-load

**Aggregate:**
- `--aggregation-type` (default: count) - Type of aggregation pipeline

**Update:**
- `--update-type` (default: setfield) - Type of update operation

**Leak_Cursor:**
- `--cursor-batch-size` (default: 101) - Batch size for each leaked cursor

**Write:**
- `--batch-size` (default: 0) - Batch size for insert_many (0 = insert_one)
- `--max-writes-per-sec` (default: -1) - Rate limit (-1 = unlimited)

**All (optional):**
- `--indexed` - Use indexed documents (creates index on indexed_field)

---

## Building

```bash
cargo build --release
```

The binary will be created at `./target/release/benchly`

### Docker Image

```bash
docker build -t benchly:local -f apps/benchly/Dockerfile apps/benchly
docker run --rm benchly:local --help
```

### Docker Runner

The Docker runner builds the image, mounts a local results directory, and runs any Benchly workload inside the container:

```powershell
$env:BENCHLY_MONGODB_URL = "mongodb://..."
./apps/docker-runner/run-benchmark.ps1 -Test write -Workers 8 -Duration 120 -RunLabel write_smoke
```

Build the image without running a benchmark:

```powershell
./apps/docker-runner/run-benchmark.ps1 -BuildOnly
```

Preview the Docker command without running it:

```powershell
./apps/docker-runner/run-benchmark.ps1 -DryRun -SkipBuild -MongoDbUrl "mongodb://..."
```

---

## PowerShell Scripts

Convenience scripts for running benchmarks:

- **`run-bench.ps1`** - Write workload
- **`run-aggregate.ps1`** - Aggregate workload  
- **`run-update.ps1`** - Update workload
- **`run-leak-cursor.ps1`** - Cursor leak workload (⚠️ stress test)

All scripts support common parameters:
- `-ClusterName` - Target cluster
- `-Workers` - Number of concurrent workers
- `-Duration` - Test duration in seconds
- `-PreloadCount` - Documents to pre-load (read/update/aggregate/leak_cursor)
- `-SkipPreload` - Skip collection drop/preload phase

---

## Example Workflows

### Performance Testing
```bash
# Test write performance
./run-bench.ps1 -Workers 24 -Duration 300

# Test read performance with 500k documents
./run-update.ps1 -UpdateType setfield -Workers 24 -PreloadCount 500000 -SkipPreload

# Test complex aggregations
./run-aggregate.ps1 -AggregationType matchgroupsort -Workers 16 -Duration 300
```

### Workload Comparison
```bash
# Run all workloads on same dataset
./run-bench.ps1 -RunLabel "baseline_write"
./run-update.ps1 -UpdateType setfield -SkipPreload -RunLabel "baseline_update"
./run-aggregate.ps1 -AggregationType count -SkipPreload -RunLabel "baseline_agg"
```

---

## Output

All workloads generate JSON reports with:
- Start/end timestamps
- Total operations and throughput (ops/sec)
- Latency percentiles (p50, p95, p99, p999)
- Failure count
- Test configuration details

Reports are saved to: `{output-dir}/{cluster-name}/{workers}_users/{run-label}/{run-label}_{timestamp}_report.json`

---

## Notes

- All workloads record latency in microseconds with HDRHistogram and emit
  fractional millisecond values in reports and PerfLab metrics
- Connection pools are sized to match worker count
- Retry writes/reads are disabled for accurate latency measurements
- Workers use separate document partitions to minimize contention
- Each workload has a warmup phase to establish connections

---

## License

See LICENSE file for details.
