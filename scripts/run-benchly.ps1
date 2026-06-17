#!/usr/bin/env pwsh

[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$MongoDbUrlFile,

    [string]$Database = "benchmark_db",
    [string]$Collection = "benchly_test",
    [ValidateSet("write", "read", "find", "update", "aggregate", "leak_cursor", "server_info")]
    [string]$Test = "write",
    [int]$Workers = 8,
    [int]$DocSize = 1024,
    [int]$BatchSize = 0,
    [int]$MaxWritesPerSec = -1,
    [int]$Duration = 120,
    [int]$Warmup = 5,
    [int]$PreloadCount = 250000,
    [string]$AggregationType = "count",
    [string]$UpdateType = "setfield",
    [int]$CursorBatchSize = 101,
    [int]$FindLimit = 100,
    [string]$RunLabel = "insert_bench_1kb",
    [string]$OutputDir = "$PSScriptRoot/../bench-results",
    [switch]$Indexed,
    [switch]$SkipPreload,
    [switch]$SkipBuild,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs = @()
)

$ErrorActionPreference = "Stop"

$metadataProperties = @()
for ($i = 0; $i -lt $RemainingArgs.Count; $i++) {
  $argument = $RemainingArgs[$i]

  if ($argument -eq "--set" -or $argument -eq "-set") {
    if ($i + 1 -ge $RemainingArgs.Count) {
      throw "Missing key=value after $argument"
    }

    $metadataProperties += $RemainingArgs[$i + 1]
    $i++
    continue
  }

  if ($argument.StartsWith("--set=")) {
    $metadataProperties += $argument.Substring("--set=".Length)
    continue
  }

  if ($argument.StartsWith("-set=")) {
    $metadataProperties += $argument.Substring("-set=".Length)
    continue
  }

  throw "Unsupported extra argument: $argument"
}

$benchlyDir = Resolve-Path (Join-Path $PSScriptRoot "../src/benchly")
$binaryPath = Join-Path $benchlyDir "target/release/benchly"
$resolvedMongoDbUrlFile = Resolve-Path $MongoDbUrlFile

if (-not $SkipBuild -or -not (Test-Path $binaryPath)) {
  Push-Location $benchlyDir
  try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
      throw "cargo build --release failed with exit code $LASTEXITCODE"
    }
  }
  finally {
    Pop-Location
  }
}

$metadataProperties += "benchly.driver=benchly"

$args = @(
  "--mongodb-url-file", $resolvedMongoDbUrlFile,
  "--database", $Database,
  "--collection", $Collection,
  "--test", $Test,
  "--workers", $Workers,
  "--doc-size", $DocSize,
  "--batch-size", $BatchSize,
  "--max-writes-per-sec", $MaxWritesPerSec,
  "--duration", $Duration,
  "--warmup", $Warmup,
  "--preload-count", $PreloadCount,
  "--aggregation-type", $AggregationType,
  "--update-type", $UpdateType,
  "--cursor-batch-size", $CursorBatchSize,
  "--find-limit", $FindLimit,
  "--stop-on-failure",
  "--output-dir", $OutputDir,
  "--run-label", $RunLabel
)

foreach ($metadataProperty in $metadataProperties) {
  $args += @("--set", $metadataProperty)
}

if ($Indexed) {
  $args += "--indexed"
}

if ($SkipPreload) {
  $args += "--no-drop-collection"
}

& $binaryPath @args
if ($LASTEXITCODE -ne 0) {
  exit $LASTEXITCODE
}