# Aggregate Query Workload

The aggregate workload tests MongoDB aggregation pipeline performance with various query patterns.

## Aggregation Types

### 1. `count` - Simple Count
Simple document count aggregation.
```javascript
[{$count: "total"}]
```

### 2. `groupbycount` - Group By with Count
Groups documents by the indexed_field and counts each group.
```javascript
[{$group: {_id: "$indexed_field", count: {$sum: 1}}}]
```

### 3. `matchrangecount` - Match Range and Count
Matches a random range of documents (10% of total) and counts them.
```javascript
[
  {$match: {indexed_field: {$gte: X, $lte: Y}}},
  {$count: "total"}
]
```

### 4. `groupbymodsum` - Group by Modulo with Sum
Groups documents by indexed_field modulo 100 and sums values.
```javascript
[{$group: {
  _id: {$mod: ["$indexed_field", 100]},
  total: {$sum: "$indexed_field"},
  count: {$sum: 1}
}}]
```

### 5. `sortlimit` - Sort and Limit
Sorts documents by indexed_field and limits results.
```javascript
[
  {$sort: {indexed_field: 1}},
  {$limit: 100}
]
```

### 6. `matchgroupsort` - Complex Pipeline
Complex pipeline combining match, group, sort, and limit operations.
```javascript
[
  {$match: {indexed_field: {$gte: X, $lte: Y}}},
  {$group: {
    _id: {$mod: ["$indexed_field", 50]},
    count: {$sum: 1},
    avg_value: {$avg: "$indexed_field"}
  }},
  {$sort: {count: -1}},
  {$limit: 10}
]
```

## Usage

### Command Line

```bash
cargo build --release

./target/release/benchly \
  --mongodb-url "mongodb://..." \
  --test aggregate \
  --aggregation-type matchgroupsort \
  --workers 8 \
  --duration 120 \
  --preload-count 250000 \
  --drop-collection true \
  --output-dir ./bench-results \
  --run-label aggregate_test \
  --cluster-name my-cluster
```

### PowerShell Script

```powershell
# Run a specific aggregation type
./run-aggregate.ps1 -AggregationType matchgroupsort -Workers 8 -PreloadCount 250000

# Available aggregation types:
# - count
# - groupbycount
# - matchrangecount
# - groupbymodsum
# - sortlimit
# - matchgroupsort

# Skip preload if data already exists
./run-aggregate.ps1 -AggregationType count -SkipPreload

# Customize cluster and workers
./run-aggregate.ps1 `
  -ClusterName "trsharp-m80-csharp" `
  -Workers 24 `
  -AggregationType matchgroupsort `
  -PreloadCount 500000 `
  -Duration 300
```

## Key Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--test` | `write` | Set to `aggregate` for aggregate workload |
| `--aggregation-type` | `count` | Type of aggregation pipeline to run |
| `--preload-count` | `250000` | Number of documents to pre-load |
| `--workers` | `8` | Number of concurrent aggregation workers |
| `--duration` | `120` | Test duration in seconds |
| `--warmup` | `5` | Warmup period in seconds |
| `--doc-size` | `1024` | Document size in bytes |
| `--drop-collection` | `true` | Whether to drop and re-create collection |

## Notes

- All aggregate queries use indexed documents with an `indexed_field`
- An index is automatically created on `indexed_field` during setup
- Random ranges are generated per query to simulate realistic workloads
- Workers run concurrent aggregation queries throughout the test duration
- Results include latency percentiles (p50, p95, p99, p999) and throughput
