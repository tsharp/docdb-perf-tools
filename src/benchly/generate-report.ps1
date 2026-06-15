#!/usr/bin/env pwsh

param(
  [string]$resultsDir = "$PSScriptRoot/bench-results",
  [string]$outputFile = "$PSScriptRoot/report.csv"
)

# Find all report.json files
$reportFiles = Get-ChildItem -Path $resultsDir -Recurse -Filter "*_report.json"

if ($reportFiles.Count -eq 0) {
  Write-Host "No report.json files found in $resultsDir"
  exit 1
}

Write-Host "Found $($reportFiles.Count) report files"

# Initialize array to store all data
$allData = @()

foreach ($file in $reportFiles) {
  Write-Host "Processing: $($file.FullName)"
  
  # Parse path to extract cluster and users
  # Path format: bench-results/{cluster}/{users}_users/{test}/{test}_report.json
  $pathParts = $file.DirectoryName -split '/'
  
  # Find the indices
  $benchResultsIndex = [array]::IndexOf($pathParts, 'bench-results')
  
  if ($benchResultsIndex -ge 0 -and $benchResultsIndex + 2 -lt $pathParts.Length) {
    $cluster = $pathParts[$benchResultsIndex + 1]
    $usersFolder = $pathParts[$benchResultsIndex + 2]
    
    # Extract user count from folder name like "128_users"
    if ($usersFolder -match '^(\d+)_users$') {
      $users = [int]$Matches[1]
    } else {
      Write-Host "  Warning: Could not parse user count from '$usersFolder', skipping"
      continue
    }
    
    # Extract test name from the parent folder
    $test = $pathParts[$benchResultsIndex + 3]
    
    # Parse cluster name to extract size and type
    $clusterSize = $null
    $clusterType = $null
    
    if ($cluster -match '^trsharp-m(\d+)-(.+)$') {
      $clusterSize = [int]$Matches[1]
      $clusterType = $Matches[2]
    }
    
    # Read and parse the JSON file
    $jsonContent = Get-Content -Path $file.FullName -Raw | ConvertFrom-Json
    
    $writes = $jsonContent.writes
    $meta = $jsonContent.metadata
    
    $dataRow = [PSCustomObject]@{
      'Cluster'              = $cluster
      'Cluster Size'         = $clusterSize
      'Cluster Type'         = $clusterType
      'Users'                = $users
      'Test'                 = $test
      'Request Count'        = $writes.total_operations
      'Failure Count'        = $writes.total_failures
      'Median Response Time' = $writes.latency_ms.p50
      'Average Response Time'= $writes.latency_ms.avg
      'Min Response Time'    = $writes.latency_ms.min
      'Max Response Time'    = $writes.latency_ms.max
      'Average Content Size' = 0
      'Requests/s'           = [math]::Round($writes.operations_per_sec, 1)
      'Failures/s'           = [math]::Round($writes.failures_per_sec, 1)
      '50%'                  = $writes.latency_ms.p50
      '75%'                  = $writes.latency_ms.p75
      '90%'                  = $writes.latency_ms.p90
      '95%'                  = $writes.latency_ms.p95
      '99%'                  = $writes.latency_ms.p99
      '100%'                 = $writes.latency_ms.p100
    }
    
    $allData += $dataRow
  } else {
    Write-Host "  Warning: Unexpected path structure, skipping"
  }
}

if ($allData.Count -eq 0) {
  Write-Host "No data extracted"
  exit 1
}

Write-Host "Extracted $($allData.Count) data rows"

$columnOrder = @(
  'Cluster',
  'Cluster Size',
  'Cluster Type',
  'Users',
  'Test',
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

$allData | Select-Object $columnOrder | Sort-Object 'Cluster Size', 'Cluster Type', Users, Test | Export-Csv -Path $outputFile -NoTypeInformation

Write-Host "Report generated: $outputFile"
Write-Host "Total rows: $($allData.Count)"
