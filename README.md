# docdb-perf-tools

Utilities for running MongoDB-compatible DocumentDB performance benchmarks. The
main workload runner is `benchly`, a Rust binary under `src/benchly`, with
PowerShell wrapper scripts for common read, write, and find benchmark sweeps.

## Recommended Setup

Use a dedicated Ubuntu machine for benchmark runs. A shared workstation or host
with other active services can add CPU, memory, disk, and network noise that
makes results harder to compare.

Recommended machine characteristics:

- Fresh Ubuntu VM or physical machine with `sudo` access.
- No unrelated workloads running during the benchmark window.
- Network placement close to the target database, ideally in the same region or
  private network path when possible.
- Enough CPU and network headroom to drive the largest worker counts in the
  sweep. The default wrappers test `8, 24, 48, 64, 128, 256` workers.
- A dedicated test database or collection. The benchmark wrappers may drop and
  recreate benchmark collections unless you pass options such as `-SkipPreload`.

## Setup

From the repository root:

```bash
cd docdb-perf-tools
chmod +x scripts/install-pre-reqs.sh scripts/install-rust.sh
./scripts/install-pre-reqs.sh
./scripts/install-rust.sh
source "$HOME/.cargo/env"
```

`install-pre-reqs.sh` installs build tools, `make`, and PowerShell. `install-rust.sh`
installs Rust through `rustup`; accept the default Rust install unless you have a
specific local policy.

## Connection Secret

Do not pass the connection string directly on the command line. Copy it into a
local secret file and pass that file to the benchmark scripts with
`-MongoDbUrlFile`.

If you already have the connection string in a secure local file:

```bash
cp /path/to/mongodb-url.secret ./local.secret
chmod 600 ./local.secret
```

Or create the file manually:

```bash
printf '%s\n' '<mongodb-or-documentdb-connection-string>' > ./local.secret
chmod 600 ./local.secret
```

The secret file should contain only the connection string, with no quotes or JSON
wrapper. Files such as `*.secret`, `*.secret.*`, `.env`, and the `secrets/`
directory are ignored by this repo, but still avoid committing or sharing them.

## Build

Build the release binary before running benchmark sweeps:

```bash
make
```

This builds `src/benchly/target/release/benchly`.

## Run Benchmarks

From the `scripts` directory, pass the secret file explicitly:

```bash
cd scripts
./run-read-bench.ps1 -MongoDbUrlFile ../local.secret
./run-write-bench.ps1 -MongoDbUrlFile ../local.secret
./run-find-bench.ps1 -MongoDbUrlFile ../local.secret
```

You can also run the wrappers from the repository root:

```bash
./scripts/run-read-bench.ps1 -MongoDbUrlFile ./local.secret
./scripts/run-write-bench.ps1 -MongoDbUrlFile ./local.secret
./scripts/run-find-bench.ps1 -MongoDbUrlFile ./local.secret
```

The default benchmark sweep runs each workload at `8, 24, 48, 64, 128, 256`
workers, with 1 KB documents, a 5 second warmup, a 300 second measurement window,
and a 15 second pause between worker counts.

## Common Parameters

The read, write, and find wrappers share these parameters:

| Parameter | Default | Description |
| --- | --- | --- |
| `-MongoDbUrlFile` | Required | Path to a file containing the connection string. |
| `-Database` | `benchmark_db` | Database used for the run. |
| `-Collection` | `benchly_test` | Collection used for the run. |
| `-Workers` | `8,24,48,64,128,256` | Worker counts to sweep. Pass comma-separated values or repeated values. |
| `-DocSize` | `1024` | Document size in bytes. |
| `-Duration` | `300` | Measurement duration in seconds for each worker count. |
| `-Warmup` | `5` | Warmup period in seconds before measurements are recorded. |
| `-OutputDir` | `../bench-results` | Directory where benchmark reports are written. |
| `-Indexed` | Off | Adds indexed fields and creates the supporting index. |
| `-SkipPreload` | Off | Reuses existing data and avoids dropping the collection. |
| `-SkipBuild` | Off | Skips building the Rust binary in the wrapper script. Useful after `make`. |

Read and find workloads also support `-PreloadCount`, which defaults to
`250000`. Find workloads support `-FindLimit` and `-CursorBatchSize`. Write
workloads support `-BatchSize` and `-MaxWritesPerSec`.

Examples:

```bash
./scripts/run-read-bench.ps1 \
  -MongoDbUrlFile ./local.secret \
  -Workers 8,24,48 \
  -Duration 120 \
  -RunLabel read_smoke

./scripts/run-write-bench.ps1 \
  -MongoDbUrlFile ./local.secret \
  -Workers 64 \
  -BatchSize 100 \
  -RunLabel write_batch_100
```

Additional report metadata can be attached with `--set key=value` and is passed
through to the generated report:

```bash
./scripts/run-find-bench.ps1 \
  -MongoDbUrlFile ./local.secret \
  --set cluster=my-test-cluster \
  --set cluster.size=8 \
  --set cluster.type=standard
```

## Results

Benchmark reports are written under `bench-results/`. To produce a CSV summary
from all generated reports:

```bash
./scripts/collect-results.ps1
```

The default summary output is `bench-results/summary.csv`.

## Troubleshooting

- If `cargo` is not found after installing Rust, run `source "$HOME/.cargo/env"`
  or open a new shell.
- If a PowerShell script is not executable, run it with `pwsh`, for example
  `pwsh ./scripts/run-read-bench.ps1 -MongoDbUrlFile ./local.secret`.
- If connection attempts fail, verify the machine can reach the database endpoint
  and that the connection string in the secret file is a single non-empty line.
- Run one workload sweep at a time for cleaner measurements.