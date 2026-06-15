# Cursor Leak Workload

⚠️ **WARNING**: This workload intentionally creates MongoDB cursors without consuming them to test cursor leak handling and cleanup behavior.

## Purpose

Tests how MongoDB handles unclosed/leaked cursors by:
- Creating cursors as fast as possible
- **NOT** consuming the cursor results
- Intentionally dropping cursors without closing them
- Running multiple concurrent workers

This stresses:
- Server-side cursor tracking and cleanup
- Cursor timeout mechanisms
- Memory management under cursor pressure
- Session/connection pool behavior with leaked cursors

## How It Works

Each worker continuously:
1. Opens a cursor with `find()` and specified batch size
2. **Does NOT iterate** or consume the cursor
3. Drops the cursor reference immediately (leak)
4. Repeats as fast as possible

The server must track and eventually clean up these abandoned cursors.

## Usage

### Command Line

```bash
cargo build --release

./target/release/benchly \
  --mongodb-url "mongodb://..." \
  --test leak_cursor \
  --workers 8 \
  --duration 120 \
  --preload-count 250000 \
  --cursor-batch-size 101 \
  --drop-collection \
  --output-dir ./bench-results \
  --run-label cursor_leak_test \
  --cluster-name my-cluster
```

### PowerShell Script

```powershell
# Run cursor leak test with 8 workers
./run-leak-cursor.ps1 -Workers 8 -Duration 120

# Higher cursor batch size for more data per cursor
./run-leak-cursor.ps1 -Workers 24 -CursorBatchSize 1000 -Duration 300

# Skip preload if data already exists
./run-leak-cursor.ps1 -Workers 16 -SkipPreload

# Customize cluster and parameters
./run-leak-cursor.ps1 `
  -ClusterName "trsharp-m80-csharp" `
  -Workers 24 `
  -CursorBatchSize 500 `
  -PreloadCount 500000 `
  -Duration 300
```

## Key Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--test` | `write` | Set to `leak_cursor` for cursor leak workload |
| `--cursor-batch-size` | `101` | Batch size for each cursor |
| `--preload-count` | `250000` | Number of documents to query |
| `--workers` | `8` | Number of concurrent cursor leakers |
| `--duration` | `120` | Test duration in seconds |
| `--warmup` | `5` | Warmup period in seconds |
| `--drop-collection` | flag | Whether to drop and re-create collection |

## Metrics Reported

- **Cursor creation rate**: Operations/second = cursors created per second
- **Latency**: Time to create each cursor (p50, p95, p99, p999)
- **Total cursors**: Logged every 10,000 cursors from worker 0

## What to Monitor

When running this test, monitor on the MongoDB server side:

1. **Cursor count**: Active cursors on the server
2. **Cursor timeouts**: How quickly cursors are cleaned up
3. **Memory usage**: Impact of leaked cursors on memory
4. **Connection behavior**: Connection pool under cursor leak stress

## Expected Behavior

- Cursors should be created rapidly (thousands/second)
- Server should eventually timeout and clean up leaked cursors
- Default cursor timeout is typically 10 minutes
- Memory pressure may increase during the test
- Throughput may decrease if server is overwhelmed

## Safety Notes

⚠️ **Use with caution on production systems!**

- This intentionally creates resource leaks
- May cause memory pressure on the server
- May impact other operations on the same cluster
- Stop the test if server shows signs of distress

## Example Output

```
BENCHLY - CURSOR LEAK BENCHMARK
================================================================================
  Cluster:          trsharp-m40-rust
  Workers:          8
  Duration:         120s
  Cursor batch size: 101
  WARNING:          This test intentionally leaks cursors!
================================================================================

Pre-loading 250000 documents...
All leakers ready. Recording for 120s...
WARNING: Cursors will be created but NOT consumed - this tests cursor leak handling!

[Leaker 0] Created 10000 cursors (leaked, not consumed)
[Leaker 0] Created 20000 cursors (leaked, not consumed)
[Leaker 0] Created 30000 cursors (leaked, not consumed)
...
```

## Use Cases

- Testing cursor timeout configurations
- Stress testing cursor cleanup mechanisms
- Validating server-side cursor limits
- Testing behavior under cursor exhaustion
- Benchmarking cursor creation overhead

## Comparison with Other Workloads

| Workload | Cursor Behavior | Purpose |
|----------|----------------|---------|
| **read** | Creates and consumes cursors | Normal read performance |
| **aggregate** | Creates and consumes cursors | Aggregation pipeline performance |
| **leak_cursor** | Creates but **never** consumes | Cursor leak handling stress test |

---

**Remember**: This is a stress test for cursor leak scenarios. Use responsibly!
