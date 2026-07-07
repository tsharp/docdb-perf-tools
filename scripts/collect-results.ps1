#!/usr/bin/env pwsh

param(
  [string]$resultsDir = "$PSScriptRoot/../bench-results",
  [string]$outputFile = "$PSScriptRoot/../bench-results/summary.csv"
)

$ErrorActionPreference = "Stop"

function Get-JsonValue {
  param(
    $Object,

    [Parameter(Mandatory = $true)]
    [string[]]$Path
  )

  $current = $Object
  foreach ($part in $Path) {
    if ($null -eq $current) {
      return $null
    }

    $property = $current.PSObject.Properties[$part]
    if ($null -eq $property) {
      return $null
    }

    $current = $property.Value
  }

  return $current
}

function Get-MetadataProperty {
  param(
    [Parameter(Mandatory = $true)]
    $Json,

    [Parameter(Mandatory = $true)]
    [string]$Name
  )

  return Get-JsonValue -Object $Json -Path @('metadata', 'properties', $Name)
}

function Get-FirstValue {
  param([object[]]$Values)

  foreach ($value in $Values) {
    if ($null -eq $value) {
      continue
    }

    if ($value -is [string] -and [string]::IsNullOrWhiteSpace($value)) {
      continue
    }

    if ($value -is [array] -and $value.Count -eq 0) {
      continue
    }

    if ($value -isnot [string] -or $value -ne '') {
      return $value
    }
  }

  return $null
}

$resolvedResultsDir = (Resolve-Path $resultsDir).Path

# Find all report.json files
$reportFiles = Get-ChildItem -Path $resolvedResultsDir -Recurse -Filter "*_report.json"

if ($reportFiles.Count -eq 0) {
  Write-Host "No report.json files found in $resultsDir"
  exit 1
}

Write-Host "Found $($reportFiles.Count) report files"

# Initialize array to store all data
$allData = @()

foreach ($file in $reportFiles) {
  Write-Host "Processing: $($file.FullName)"
  
  # Read and parse the JSON file first. Newer reports carry metadata that can
  # replace path-derived values.
  $jsonContent = Get-Content -Path $file.FullName -Raw | ConvertFrom-Json

  # Supported path formats:
  #   bench-results/{cluster}/{users}_users/{test}/{test}_report.json
  #   bench-results/{users}_users/{test}/{test}_report.json
  $relativeDirectory = [System.IO.Path]::GetRelativePath($resolvedResultsDir, $file.DirectoryName)
  $pathParts = @($relativeDirectory -split '[\\/]') | Where-Object { $_ -and $_ -ne '.' }

  $usersFolderIndex = -1
  for ($index = 0; $index -lt $pathParts.Count; $index++) {
    if ($pathParts[$index] -match '^(\d+)_users$') {
      $usersFolderIndex = $index
      break
    }
  }

  if ($usersFolderIndex -lt 0) {
    Write-Host "  Warning: Could not find a *_users folder in '$relativeDirectory', skipping"
    continue
  }

  $users = [int]($pathParts[$usersFolderIndex] -replace '_users$', '')
  $test = Get-FirstValue @(
    (Get-JsonValue -Object $jsonContent -Path @('metadata', 'run_label')),
    (Get-JsonValue -Object $jsonContent -Path @('metadata', 'benchmark_name')),
    $(if ($usersFolderIndex + 1 -lt $pathParts.Count) { $pathParts[$usersFolderIndex + 1] } else { $null })
  )

  $cluster = Get-FirstValue @(
    (Get-MetadataProperty -Json $jsonContent -Name 'cluster'),
    $(if ($usersFolderIndex -gt 0) { $pathParts[$usersFolderIndex - 1] } else { $null })
  )

  # Parse cluster name to extract size and type when present.
  # Format: trsharp-m{size}-{type}
  $clusterSize = $null
  $clusterType = $null
  if ($cluster -match '^trsharp-m(\d+)-(.+)$') {
    $clusterSize = [int]$Matches[1]
    $clusterType = $Matches[2]
  }

  $clusterSize = Get-FirstValue @((Get-MetadataProperty -Json $jsonContent -Name 'cluster.size'), $clusterSize)
  $clusterType = Get-FirstValue @((Get-MetadataProperty -Json $jsonContent -Name 'cluster.type'), $clusterType)

  $summary = Get-JsonValue -Object $jsonContent -Path @('summary')
  $writes = Get-JsonValue -Object $jsonContent -Path @('writes')
  $latency = Get-FirstValue @(
    (Get-JsonValue -Object $summary -Path @('percentiles_ms')),
    (Get-JsonValue -Object $writes -Path @('latency_ms'))
  )

  $requestCount = Get-FirstValue @(
    (Get-JsonValue -Object $summary -Path @('total_requests')),
    (Get-JsonValue -Object $writes -Path @('total_operations'))
  )
  $failureCount = Get-FirstValue @(
    (Get-JsonValue -Object $summary -Path @('total_failures')),
    (Get-JsonValue -Object $writes -Path @('total_failures'))
  )
  $requestsPerSecond = Get-FirstValue @(
    (Get-JsonValue -Object $summary -Path @('requests_per_sec')),
    (Get-JsonValue -Object $writes -Path @('operations_per_sec'))
  )

  # Create a custom object with the data we need. All fields are nullable so
  # partial report shapes still contribute a row.
  $dataRow = [PSCustomObject]@{
    'Cluster Size' = $clusterSize
    'Cluster Type' = $clusterType
    'Users' = $users
    'Test' = $test
    'Document Size' = Get-MetadataProperty -Json $jsonContent -Name 'doc.size'
    'Driver Language' = Get-JsonValue -Object $jsonContent -Path @('metadata', 'driver_language')
    'Driver' = Get-JsonValue -Object $jsonContent -Path @('metadata', 'driver')
    'Driver Version' = Get-JsonValue -Object $jsonContent -Path @('metadata', 'driver_version')
    'Start Time' = Get-JsonValue -Object $jsonContent -Path @('metadata', 'start_time')
    'End Time' = Get-JsonValue -Object $jsonContent -Path @('metadata', 'end_time')
    'Request Count' = $requestCount
    'Failure Count' = $failureCount
    'Median Response Time' = Get-JsonValue -Object $latency -Path @('p50')
    'Average Response Time' = Get-FirstValue @(
      (Get-JsonValue -Object $summary -Path @('avg_response_time_ms')),
      (Get-JsonValue -Object $latency -Path @('avg'))
    )
    'Min Response Time' = Get-FirstValue @(
      (Get-JsonValue -Object $summary -Path @('min_response_time_ms')),
      (Get-JsonValue -Object $latency -Path @('min'))
    )
    'Max Response Time' = Get-FirstValue @(
      (Get-JsonValue -Object $summary -Path @('max_response_time_ms')),
      (Get-JsonValue -Object $latency -Path @('max'))
    )
    'Average Content Size' = Get-FirstValue @((Get-JsonValue -Object $summary -Path @('avg_content_size')), 0.0)
    'Requests/s' = $requestsPerSecond
    'Failures/s' = Get-FirstValue @(
      (Get-JsonValue -Object $summary -Path @('failures_per_sec')),
      (Get-JsonValue -Object $writes -Path @('failures_per_sec')),
      0.0
    )
    '50%' = Get-JsonValue -Object $latency -Path @('p50')
    '75%' = Get-JsonValue -Object $latency -Path @('p75')
    '90%' = Get-JsonValue -Object $latency -Path @('p90')
    '95%' = Get-JsonValue -Object $latency -Path @('p95')
    '99%' = Get-JsonValue -Object $latency -Path @('p99')
    '100%' = Get-FirstValue @(
      (Get-JsonValue -Object $latency -Path @('p100')),
      (Get-JsonValue -Object $latency -Path @('max'))
    )
  }

  $allData += $dataRow
}

if ($allData.Count -eq 0) {
  Write-Host "No data extracted"
  exit 1
}

Write-Host "Extracted $($allData.Count) data rows"

# Define column order with cluster metadata, users, and test first.
$columnOrder = @(
  'Cluster Size',
  'Cluster Type',
  'Users', 
  'Test',
  'Document Size',
  'Driver Language',
  'Driver',
  'Driver Version',
  'Start Time',
  'End Time',
  'Request Count',
  'Failure Count',
  'Median Response Time',
  'Average Response Time',
  'Min Response Time',
  'Max Response Time',
  'Average Content Size',
  'Requests/s',
  'Failures/s',
  '50%',
  '75%',
  '90%',
  '95%',
  '99%',
  '100%'
)

# Export to CSV with custom column order
$allData | Select-Object $columnOrder | Sort-Object 'Cluster Size', 'Cluster Type', Users, Test | Export-Csv -Path $outputFile -NoTypeInformation

Write-Host "Report generated: $outputFile"
Write-Host "Total rows: $($allData.Count)"
