#!/usr/bin/env pwsh

[CmdletBinding(PositionalBinding = $false)]
param(
	[Parameter(Mandatory = $true)]
	[ValidateNotNullOrEmpty()]
	[string]$MongoDbUrlFile,

	[ValidateSet("benchly")]
	[string]$Driver = "benchly",

	[string]$Database = "benchmark_db",
	[string]$Collection = "benchly_test",
	[string[]]$Workers = @("8", "16", "24", "48", "64", "128", "256"),
	[int]$DocSize = 1024,
	[int]$Duration = 300,
	[int]$Warmup = 5,
	[int]$PreloadCount = 250000,
	[string]$RunLabel = "read_bench_1kb",
	[string]$OutputDir = "$PSScriptRoot/../bench-results",
	[int]$PauseSeconds = 5,
	[switch]$Indexed,
	[switch]$SkipPreload,
	[switch]$SkipBuild,
	[Parameter(ValueFromRemainingArguments = $true)]
	[string[]]$RemainingArgs = @()
)

$ErrorActionPreference = "Stop"

$resolvedMongoDbUrlFile = (Resolve-Path $MongoDbUrlFile).Path
$parsedWorkers = @(
	foreach ($workerCount in $Workers) {
		foreach ($value in $workerCount -split ",") {
			$trimmedValue = $value.Trim()
			if ([string]::IsNullOrWhiteSpace($trimmedValue)) {
				continue
			}

			$parsedValue = 0
			if (-not [int]::TryParse($trimmedValue, [ref]$parsedValue) -or $parsedValue -lt 1) {
				throw "Workers values must be positive integers: $trimmedValue"
			}

			$parsedValue
		}
	}
)

if ($parsedWorkers.Count -eq 0) {
	throw "At least one worker count is required."
}

Write-Host "=== Benchly Read Worker Sweep ===" -ForegroundColor Cyan
Write-Host "  Driver:        $Driver"
Write-Host "  Connection:    $resolvedMongoDbUrlFile"
Write-Host "  Database:      $Database"
Write-Host "  Collection:    $Collection"
Write-Host "  Worker:        $($parsedWorkers -join ', ')"
Write-Host "  Doc size:      $DocSize bytes"
Write-Host "  Preload count: $PreloadCount"
Write-Host "  Duration:      ${Duration}s"
Write-Host "  Warmup:        ${Warmup}s"
Write-Host "  Run label:     $RunLabel"
Write-Host "  Output dir:    $OutputDir"
Write-Host ""

$runIndex = 0
foreach ($currentWorkers in $parsedWorkers) {
	Write-Host "--- Running read benchmark with $currentWorkers workers using $Driver ---" -ForegroundColor Green

	$runnerParams = @{
		MongoDbUrlFile = $resolvedMongoDbUrlFile
		Database       = $Database
		Collection     = $Collection
		Test           = "read"
		Workers        = $currentWorkers
		DocSize        = $DocSize
		Duration       = $Duration
		Warmup         = $Warmup
		PreloadCount   = $PreloadCount
		RunLabel       = $RunLabel
		OutputDir      = $OutputDir
	}

	if ($Driver -ne "benchly") {
		$runnerParams.Driver = $Driver
	}

	if ($Indexed) {
		$runnerParams.Indexed = $true
	}

	if ($SkipPreload) {
		$runnerParams.SkipPreload = $true
	}

	if ($SkipBuild -or $runIndex -gt 0) {
		$runnerParams.SkipBuild = $true
	}

	$metadataArgs = $RemainingArgs

	$runnerScript = if ($Driver -eq "benchly") { "$PSScriptRoot/run-benchly.ps1" } else { "$PSScriptRoot/run-jbenchly.ps1" }
	& $runnerScript @runnerParams @metadataArgs
	if ($LASTEXITCODE -ne 0) {
		exit $LASTEXITCODE
	}

	$runIndex++
	Write-Host ""

	if ($runIndex -lt $parsedWorkers.Count -and $PauseSeconds -gt 0) {
		Write-Host "  Completed $currentWorkers workers. Pausing ${PauseSeconds}s before next run..." -ForegroundColor Yellow
		Start-Sleep -Seconds $PauseSeconds
	}
}

Write-Host ""
Write-Host "=== Read sweep complete ===" -ForegroundColor Cyan