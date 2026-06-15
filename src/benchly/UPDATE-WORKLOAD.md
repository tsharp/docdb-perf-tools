# Find and Update Workload

The update workload tests MongoDB update performance using `findOneAndUpdate` operations with various update patterns.

## Update Types

### 1. `setfield` - Simple Field Update
Updates a single field with a new value.
```javascript
{$set: {
  updated_field: "updated_by_worker_0_at_12345",
  update_count: 12345
}}
```

### 2. `incrementcounter` - Increment Counter
Increments counter fields using atomic operations.
```javascript
{
  $inc: {counter: 1, worker_updates: 1},
  $set: {last_worker: 0}
}
```

### 3. `setmultiplefields` - Update Multiple Fields
Updates multiple fields simultaneously to test larger update payloads.
```javascript
{$set: {
  field1: "value_12345",
  field2: 4567,
  field3: 0,
  updated_at: 12345,
  status: "updated"
}}
```

### 4. `conditionalupdate` - Conditional Update with Version
Updates with version increment to simulate optimistic locking patterns.
```javascript
{
  $set: {
    status: "even",
    last_update: 12345,
    worker_id: 0
  },
  $inc: {version: 1}
}
```

## Usage

### Command Line

```bash
cargo build --release

./target/release/benchly \
  --mongodb-url "mongodb://..." \
  --test update \
  --update-type setfield \
  --workers 8 \
  --duration 120 \
  --preload-count 250000 \
  --drop-collection \
  --output-dir ./bench-results \
  --run-label update_test \
  --cluster-name my-cluster
```

### PowerShell Script

```powershell
# Run a specific update type
./run-update.ps1 -UpdateType setfield -Workers 8 -PreloadCount 250000

# Available update types:
# - setfield
# - incrementcounter
# - setmultiplefields
# - conditionalupdate

# Skip preload if data already exists
./run-update.ps1 -UpdateType incrementcounter -SkipPreload

# Customize cluster and workers
./run-update.ps1 `
  -ClusterName "trsharp-m80-csharp" `
  -Workers 24 `
  -UpdateType setmultiplefields `
  -PreloadCount 500000 `
  -Duration 300
```

## Key Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--test` | `write` | Set to `update` for update workload |
| `--update-type` | `setfield` | Type of update operation to perform |
| `--preload-count` | `250000` | Number of documents to pre-load |
| `--workers` | `8` | Number of concurrent update workers |
| `--duration` | `120` | Test duration in seconds |
| `--warmup` | `5` | Warmup period in seconds |
| `--doc-size` | `1024` | Document size in bytes |
| `--drop-collection` | flag | Whether to drop and re-create collection |
| `--indexed` | flag | Create index on `indexed_field` |

## How It Works

1. **Preload Phase**: Creates documents with unique IDs in the collection
2. **Partitioning**: Divides document IDs evenly among workers (non-overlapping)
3. **Update Loop**: Each worker continuously:
   - Randomly selects a document ID from its partition
   - Performs `findOneAndUpdate` with the specified update pattern
   - Records latency and throughput metrics
4. **Reporting**: Generates performance metrics including p50, p95, p99, p999 latencies

## Notes

- Uses `findOneAndUpdate` with `ReturnDocument::After` to return updated document
- Each worker has exclusive access to a subset of document IDs (no contention)
- Random selection within partitions simulates realistic access patterns
- Update counters are globally unique across all workers
- Results include latency percentiles and operations per second
- Compatible with existing benchly reporting infrastructure

## Example Results

Typical metrics reported:
- **Throughput**: Updates/second across all workers
- **Latency percentiles**: p50, p95, p99, p999 in milliseconds
- **Success rate**: Percentage of successful updates
- **Duration**: Actual test runtime

## Comparison with Other Workloads

| Workload | Operation | Use Case |
|----------|-----------|----------|
| **write** | insert_one/insert_many | Testing write throughput |
| **read** | find_one | Testing read latency |
| **update** | find_one_and_update | Testing update performance |
| **aggregate** | aggregate pipeline | Testing complex queries |
