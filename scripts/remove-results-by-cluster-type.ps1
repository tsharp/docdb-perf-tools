#!/usr/bin/env pwsh

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
  [Parameter(Mandatory = $true)]
  [ValidateNotNullOrEmpty()]
  [string]$ClusterType,

  [Parameter()]
  [ValidateNotNullOrEmpty()]
  [string]$ResultsDir = "$PSScriptRoot/../bench-results"
)

$ErrorActionPreference = 'Stop'

$resolvedResultsDir = (Resolve-Path -LiteralPath $ResultsDir).Path
$reportFiles = @(Get-ChildItem -LiteralPath $resolvedResultsDir -Recurse -File -Filter '*_report.json')
$matchingFiles = [System.Collections.Generic.List[System.IO.FileInfo]]::new()

foreach ($file in $reportFiles) {
  try {
    $report = Get-Content -LiteralPath $file.FullName -Raw | ConvertFrom-Json
  }
  catch {
    throw "Failed to parse benchmark report '$($file.FullName)': $($_.Exception.Message)"
  }

  $actualClusterType = $report.metadata.properties.'cluster.type'
  if ($actualClusterType -eq $ClusterType) {
    $matchingFiles.Add($file)
  }
}

if ($matchingFiles.Count -eq 0) {
  Write-Host "No benchmark reports found with cluster.type '$ClusterType' in '$resolvedResultsDir'."
  exit 0
}

Write-Host "Found $($matchingFiles.Count) benchmark report(s) with cluster.type '$ClusterType':"
foreach ($file in $matchingFiles) {
  Write-Host "  $($file.FullName)"
}

$target = "$($matchingFiles.Count) benchmark report(s) under '$resolvedResultsDir'"
$action = "Delete reports with cluster.type '$ClusterType'"
if (-not $PSCmdlet.ShouldProcess($target, $action)) {
  exit 0
}

foreach ($file in $matchingFiles) {
  Remove-Item -LiteralPath $file.FullName -Force
}

Write-Host "Deleted $($matchingFiles.Count) benchmark report(s) with cluster.type '$ClusterType'."
