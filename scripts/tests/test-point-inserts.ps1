#!/usr/bin/env pwsh

$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot/../..").Path
$temp = Join-Path ([IO.Path]::GetTempPath()) ("point-inserts-test-" + [Guid]::NewGuid().ToString("N"))
$oldCapture = $env:BENCHLY_INSERT_TEST_CAPTURE
$oldExit = $env:BENCHLY_INSERT_TEST_EXIT
$fixture = @'
#!/usr/bin/env pwsh
@{ arguments = @($args) } | ConvertTo-Json -Compress |
    Add-Content -LiteralPath $env:BENCHLY_INSERT_TEST_CAPTURE
exit ([int]$env:BENCHLY_INSERT_TEST_EXIT)
'@

function Assert-Option($arguments, $name, $value) {
    $index = [array]::IndexOf($arguments, $name)
    if ($index -lt 0 -or $arguments[$index + 1] -ne [string]$value) {
        throw "Expected $name $value; received: $($arguments -join ' ')"
    }
}

try {
    $scripts = Join-Path $temp "scripts"
    $binaryDir = Join-Path $temp "src/benchly/target/release"
    $secrets = Join-Path $temp "secrets"
    $null = New-Item -ItemType Directory -Path $scripts, $binaryDir, $secrets
    $entry = Join-Path $temp "Load-Test-PointInserts.ps1"
    $writeRunner = Join-Path $scripts "run-write-bench.ps1"
    $benchlyRunner = Join-Path $scripts "run-benchly.ps1"
    $binary = Join-Path $binaryDir "benchly"
    $secret = Join-Path $secrets "1103_m30.secret"
    $capture = Join-Path $temp "arguments.jsonl"
    Copy-Item "$root/Load-Test-PointInserts.ps1" $entry
    Copy-Item "$root/scripts/run-write-bench.ps1" $writeRunner
    Copy-Item "$root/scripts/run-benchly.ps1" $benchlyRunner
    Set-Content -LiteralPath $binary -Value $fixture -Encoding utf8NoBOM
    Set-Content -LiteralPath $secret -Value "mongodb://fixture.invalid"
    & chmod +x $binary
    if ($LASTEXITCODE -ne 0) { throw "Could not make fixture executable." }
    $env:BENCHLY_INSERT_TEST_CAPTURE = $capture
    $env:BENCHLY_INSERT_TEST_EXIT = "0"

    $output = & pwsh -NoProfile -File $entry -SkipBuild 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Default run failed: $output" }
    $arguments = (Get-Content $capture | ConvertFrom-Json).arguments
    foreach ($pair in @(
        @("--test", "write"), @("--workers", "256"), @("--duration", "45"),
        @("--collection", "benchly_point_inserts"), @("--database", "benchmark_db"),
        @("--doc-size", "1024"), @("--batch-size", "0"), @("--insert-timeout-ms", "5000"),
        @("--insert-max-time-ms", "5000")
    )) {
        Assert-Option $arguments $pair[0] $pair[1]
    }
    if ($arguments -contains "--no-drop-collection" -or $arguments -notcontains "--stop-on-failure" -or
        $arguments -notcontains "cluster.type=rust_baseline" -or
        $arguments -notcontains "cluster.version=v2.2.0-505c9b0b0a" -or
        $arguments -notcontains "cluster.size=m30" -or
        -not ($arguments -match "^run.group=\d{8}\.\d+$")) {
        throw "Default metadata or insert behavior was not forwarded."
    }
    Write-Host "PASS: default single inserts, separate collection, deadline, and metadata."

    foreach ($noDropFlag in @("-SkipPreload", "-NoDropCollection")) {
        Clear-Content $capture
        $output = & pwsh -NoProfile -File $entry -SkipBuild -Workers 1,2 $noDropFlag 2>&1
        if ($LASTEXITCODE -ne 0) { throw "No-drop sweep failed: $output" }
        $runs = @(Get-Content $capture | ConvertFrom-Json)
        if ($runs.Count -ne 2) { throw "Expected two no-drop worker-count runs." }
        foreach ($run in $runs) {
            if ($run.arguments -notcontains "--no-drop-collection") {
                throw "$noDropFlag was not forwarded through the wrappers."
            }
        }
        if (($output -join "`n") -match "drops and recreates") {
            throw "No-drop mode incorrectly warned that it would drop the collection."
        }
    }
    Write-Host "PASS: SkipPreload and NoDropCollection preserve the collection throughout the sweep."

    Clear-Content $capture
    $output = & pwsh -NoProfile -File $entry -SkipBuild -Workers 2 -Duration 1 `
        -MongoDbUrlFile $secret -Database custom_db -Collection custom_inserts `
        -DocSize 2048 -InsertTimeoutMs 250 -InsertMaxTimeMs 100 -Indexed `
        -ClusterType rust_v1_baseline -ClusterVersion custom -ClusterSize m40 `
        --set custom=value 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Custom run failed: $output" }
    $arguments = (Get-Content $capture | ConvertFrom-Json).arguments
    Assert-Option $arguments "--insert-timeout-ms" 250
    Assert-Option $arguments "--insert-max-time-ms" 100
    Assert-Option $arguments "--doc-size" 2048
    Assert-Option $arguments "--database" custom_db
    Assert-Option $arguments "--collection" custom_inserts
    foreach ($expected in @("--indexed", "cluster.type=rust_v1_baseline", "cluster.version=custom", "cluster.size=m40", "custom=value")) {
        if ($arguments -notcontains $expected) { throw "Missing $expected in native arguments." }
    }
    Write-Host "PASS: custom timeout, dataset options, cluster fields, and extra metadata."

    Clear-Content $capture
    $output = & pwsh -NoProfile -File $writeRunner -MongoDbUrlFile $secret `
        -Workers 1,2 -PauseSeconds 0 -SkipBuild -BatchSize 4 -InsertTimeoutMs 500 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Sweep failed: $output" }
    $runs = @(Get-Content $capture | ConvertFrom-Json)
    if ($runs.Count -ne 2) { throw "Expected two worker-count runs." }
    for ($i = 0; $i -lt 2; $i++) {
        Assert-Option $runs[$i].arguments "--workers" ($i + 1)
        Assert-Option $runs[$i].arguments "--insert-timeout-ms" 500
        Assert-Option $runs[$i].arguments "--batch-size" 4
    }
    Write-Host "PASS: deadline forwarding throughout a batched write sweep."

    Clear-Content $capture
    $output = & pwsh -NoProfile -File $writeRunner -MongoDbUrlFile $secret `
        -Workers 1,2 -PauseSeconds 0 -SkipBuild -InsertMaxTimeMs 300 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Wire timeout sweep failed: $output" }
    $runs = @(Get-Content $capture | ConvertFrom-Json)
    if ($runs.Count -ne 2) { throw "Expected two wire-timeout runs." }
    foreach ($run in $runs) { Assert-Option $run.arguments "--insert-max-time-ms" 300 }
    Write-Host "PASS: wire timeout forwarding throughout a point-insert sweep."

    Clear-Content $capture
    $output = & pwsh -NoProfile -File $benchlyRunner -MongoDbUrlFile $secret -Test write -SkipBuild 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Legacy invocation failed: $output" }
    $arguments = (Get-Content $capture | ConvertFrom-Json).arguments
    if ($arguments -contains "--insert-timeout-ms" -or $arguments -contains "--insert-max-time-ms") {
        throw "Existing invocations gained an explicit deadline."
    }
    Write-Host "PASS: omitted timeout preserves existing native arguments."

    Clear-Content $capture
    $output = & pwsh -NoProfile -File $entry -SkipBuild -InsertMaxTimeMs 0 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Client-only mode failed: $output" }
    $arguments = (Get-Content $capture | ConvertFrom-Json).arguments
    if ($arguments -contains "--insert-max-time-ms") { throw "Wire timeout was not disabled." }
    Assert-Option $arguments "--insert-timeout-ms" 5000
    Write-Host "PASS: explicit opt-out retains the client-only deadline."

    $output = & pwsh -NoProfile -File $writeRunner -MongoDbUrlFile $secret `
        -SkipBuild -BatchSize 2 -InsertMaxTimeMs 100 2>&1
    if ($LASTEXITCODE -eq 0) { throw "Wire timeout unexpectedly accepted batched inserts." }
    $output = & pwsh -NoProfile -File $benchlyRunner -MongoDbUrlFile $secret `
        -SkipBuild -Test read -InsertMaxTimeMs 100 2>&1
    if ($LASTEXITCODE -eq 0) { throw "Wire insert timeout unexpectedly accepted read workload." }
    Write-Host "PASS: unsupported wire timeout combinations fail explicitly."

    $env:BENCHLY_INSERT_TEST_EXIT = "7"
    $output = & pwsh -NoProfile -File $entry -SkipBuild 2>&1
    if ($LASTEXITCODE -ne 7) { throw "Native failure was not propagated through the scripts: $output" }
    Write-Host "PASS: native failure produces a nonzero script exit."

    $output = & pwsh -NoProfile -File $entry -InsertTimeoutMs 0 2>&1
    if ($LASTEXITCODE -eq 0) { throw "The insert script accepted a zero deadline." }
    Write-Host "PASS: new insert script rejects a nonpositive deadline."
}
finally {
    $env:BENCHLY_INSERT_TEST_CAPTURE = $oldCapture
    $env:BENCHLY_INSERT_TEST_EXIT = $oldExit
    foreach ($path in @($capture, $binary, $secret, $entry, $writeRunner, $benchlyRunner)) {
        if ($path -and (Test-Path -LiteralPath $path)) { Remove-Item -LiteralPath $path }
    }
    foreach ($relative in @("src/benchly/target/release", "src/benchly/target", "src/benchly", "src", "scripts", "secrets", "")) {
        $path = if ($relative) { Join-Path $temp $relative } else { $temp }
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path }
    }
}
