#!/usr/bin/env pwsh

[CmdletBinding(PositionalBinding = $false)]
param(
    [ValidateNotNullOrEmpty()]
    [string]$LogicalDatabase = "benchmark_db",

    [ValidateNotNullOrEmpty()]
    [string]$Collection = "benchly_test",

    [ValidateRange(1, [int]::MaxValue)]
    [int]$DocumentCount = 2000000,

    [ValidateRange(1, [int]::MaxValue)]
    [int]$Clients = 16,

    [ValidateRange(1, [int]::MaxValue)]
    [int]$Jobs = 4,

    [ValidateRange(1, [int]::MaxValue)]
    [int]$Duration = 60
)

$ErrorActionPreference = "Stop"

if ($Jobs -gt $Clients) {
    throw "Jobs must not exceed Clients."
}

if ([string]::IsNullOrWhiteSpace($env:PGDATABASE)) {
    throw "Set PGDATABASE to the PostgreSQL database containing the DocumentDB extensions."
}

$psqlCommand = Get-Command psql -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
if (-not $psqlCommand) {
    throw "Required command not found: psql. Run ./scripts/install-pre-reqs.sh first."
}

$pgbenchCommand = Get-Command pgbench -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
if (-not $pgbenchCommand) {
    throw "Required command not found: pgbench. Run ./scripts/install-pre-reqs.sh first."
}

$roleSql = @"
SELECT json_build_object('oid', oid, 'name', rolname)::text
FROM pg_catalog.pg_roles
WHERE rolname = current_user;
"@
$userOidOutput = $roleSql |
    & $psqlCommand.Source -X -qAt -v ON_ERROR_STOP=1
$psqlExitCode = $LASTEXITCODE
if ($psqlExitCode -ne 0) {
    throw "psql failed while resolving the current PostgreSQL user (exit code $psqlExitCode)."
}

$roleJson = ($userOidOutput -join "`n").Trim()
try {
    $currentRole = $roleJson | ConvertFrom-Json
}
catch {
    throw "Could not parse the current PostgreSQL role returned by psql: $roleJson"
}

$userOid = [string]$currentRole.oid
$targetRole = [string]$currentRole.name
if ($userOid -notmatch "^[1-9][0-9]*$") {
    throw "Could not resolve the OID for the current PostgreSQL user."
}
if ([string]::IsNullOrWhiteSpace($targetRole)) {
    throw "Could not resolve the current PostgreSQL user name."
}

$databaseLiteral = $LogicalDatabase.Replace('\', '\\').Replace("'", "''")
$collectionLiteral = $Collection.Replace('\', '\\').Replace("'", "''")
$sqlFile = Join-Path ([IO.Path]::GetTempPath()) (
    "gateway-point-read-{0}.sql" -f [Guid]::NewGuid().ToString("N")
)

$sql = @"
\set aid random(0, :document_count - 1)
SELECT cursorPage, continuation, persistConnection, cursorId
FROM mongo_api_internal.run_query_as_user(
    1::int4,
    :user_oid::oid::regrole,
    E'$databaseLiteral'::text,
    helio_core.bson_build_document(
        'find', E'$collectionLiteral'::text,
        'filter', helio_core.bson_build_document(
            '_id',
            ('doc_' || lpad(:aid::text, 12, '0'))::text
        )
    )::helio_core.bson
);
"@

$pgbenchExitCode = 0
try {
    Set-Content -LiteralPath $sqlFile -Value $sql -Encoding utf8NoBOM

    $pgHost = if ([string]::IsNullOrWhiteSpace($env:PGHOST)) { "<libpq default>" } else { $env:PGHOST }
    $pgPort = if ([string]::IsNullOrWhiteSpace($env:PGPORT)) { "<libpq default>" } else { $env:PGPORT }
    $pgUser = if ([string]::IsNullOrWhiteSpace($env:PGUSER)) { "<libpq default>" } else { $env:PGUSER }

    Write-Host "PostgreSQL endpoint: ${pgHost}:${pgPort}; database: $($env:PGDATABASE); user: $pgUser"
    Write-Host "Target role: $targetRole (OID $userOid; selected from current_user)"
    Write-Host "Logical database: $LogicalDatabase; collection: $Collection"
    Write-Host "Document IDs: doc_000000000000 through doc_$("{0:D12}" -f ($DocumentCount - 1))"
    Write-Host "Clients: $Clients; workers: $Jobs; duration: ${Duration}s; mode: prepared"

    $pgbenchArgs = @(
        "-n",
        "-M", "prepared",
        "-c", $Clients,
        "-j", $Jobs,
        "-T", $Duration,
        "-D", "user_oid=$userOid",
        "-D", "document_count=$DocumentCount",
        "-f", $sqlFile
    )
    & $pgbenchCommand.Source @pgbenchArgs
    $pgbenchExitCode = $LASTEXITCODE
}
finally {
    Remove-Item -LiteralPath $sqlFile -Force -ErrorAction SilentlyContinue
}

if ($pgbenchExitCode -ne 0) {
    exit $pgbenchExitCode
}
