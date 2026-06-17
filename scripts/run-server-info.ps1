#!/usr/bin/env pwsh

[CmdletBinding(PositionalBinding = $false)]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$MongoDbUrlFile,

    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$resolvedMongoDbUrlFile = (Resolve-Path $MongoDbUrlFile).Path

$runnerParams = @{
    MongoDbUrlFile = $resolvedMongoDbUrlFile
    Test           = "server_info"
    Workers        = 1
    Duration       = 1
    Warmup         = 0
    PreloadCount   = 0
    RunLabel       = "server_info"
    SkipPreload    = $true
}

if ($SkipBuild) {
    $runnerParams.SkipBuild = $true
}

Write-Host "=== Benchly Server Info ===" -ForegroundColor Cyan
Write-Host "  Connection: $resolvedMongoDbUrlFile"
Write-Host ""

& "$PSScriptRoot/run-benchly.ps1" @runnerParams

if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}