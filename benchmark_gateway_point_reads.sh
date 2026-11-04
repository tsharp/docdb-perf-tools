#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: bash benchmark_gateway_point_reads.sh

Benchmark the native gateway's backend find dispatch using prepared pgbench SQL.
The current PostgreSQL connection user is used to execute the find and must be
authorized to invoke the internal dispatch function.

Connection: set PGDATABASE (required), PGHOST, PGPORT, and PGUSER as needed.
Use .pgpass or another libpq authentication mechanism for credentials.

Optional environment variables:
  LOGICAL_DB     Logical document database (default: benchmark_db)
  COLLECTION     Collection name (default: benchly_test)
  DOCUMENT_COUNT Number of preloaded documents (default: 2000000)
  CLIENTS        Concurrent pgbench connections (default: 16)
  JOBS           pgbench worker threads (default: 4)
  DURATION       Benchmark duration in seconds (default: 60)

This measures backend dispatch, not end-to-end gateway performance.
BSON is constructed server-side; results use pgbench's result format.
EOF
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then
    usage
    exit 0
fi
if [[ $# != 0 ]]; then
    usage >&2
    exit 2
fi

for tool in psql pgbench mktemp; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Required command not found: %s\n' "$tool" >&2
        exit 1
    fi
done

: "${PGDATABASE:?Set PGDATABASE to the PostgreSQL database containing the extensions}"
LOGICAL_DB=${LOGICAL_DB:-benchmark_db}
COLLECTION=${COLLECTION:-benchly_test}
DOCUMENT_COUNT=${DOCUMENT_COUNT:-2000000}
CLIENTS=${CLIENTS:-16}
JOBS=${JOBS:-4}
DURATION=${DURATION:-60}

validate_positive_integer() {
    local name=$1 value=$2
    if [[ ! $value =~ ^[1-9][0-9]*$ || ${#value} -gt 10 ]]; then
        printf '%s must be a positive integer no greater than 2147483647\n' "$name" >&2
        exit 2
    fi
    if (( value > 2147483647 )); then
        printf '%s must be no greater than 2147483647\n' "$name" >&2
        exit 2
    fi
}
validate_positive_integer DOCUMENT_COUNT "$DOCUMENT_COUNT"
validate_positive_integer CLIENTS "$CLIENTS"
validate_positive_integer JOBS "$JOBS"
validate_positive_integer DURATION "$DURATION"
if (( JOBS > CLIENTS )); then
    printf 'JOBS must not exceed CLIENTS\n' >&2
    exit 2
fi

# Resolve the role selected by the active libpq connection.
role_info=$(psql -X -qAt -v ON_ERROR_STOP=1 <<'SQL'
SELECT oid::text || '|' || current_user
FROM pg_catalog.pg_roles
WHERE rolname = current_user;
SQL
)
user_oid=${role_info%%|*}
target_role=${role_info#*|}
if [[ ! $user_oid =~ ^[1-9][0-9]*$ ]]; then
    printf 'Could not resolve the OID for the current PostgreSQL user\n' >&2
    exit 1
fi
if [[ -z $target_role || $target_role == "$role_info" ]]; then
    printf 'Could not resolve the current PostgreSQL user name\n' >&2
    exit 1
fi

sql_file=$(mktemp "${TMPDIR:-/tmp}/gateway-point-read.XXXXXXXX.sql")
trap 'rm -f -- "$sql_file"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Explicit escape-string literals preserve quotes and backslashes in names.
database_literal=${LOGICAL_DB//\\/\\\\}
database_literal=${database_literal//\'/\'\'}
collection_literal=${COLLECTION//\\/\\\\}
collection_literal=${collection_literal//\'/\'\'}

cat >"$sql_file" <<SQL
\set aid random(0, :document_count - 1)
SELECT cursorPage, continuation, persistConnection, cursorId
FROM mongo_api_internal.run_query_as_user(
    1::int4,
    :user_oid::oid::regrole,
    E'$database_literal'::text,
    helio_core.bson_build_document(
        'find', E'$collection_literal'::text,
        'filter', helio_core.bson_build_document(
            '_id',
            ('doc_' || lpad(:aid::text, 12, '0'))::text
        )
    )::helio_core.bson
);
SQL

printf 'Target role: %s (OID %s; selected from current_user)\n' \
    "$target_role" "$user_oid"
printf 'Logical database: %s; collection: %s; document count: %s\n' \
    "$LOGICAL_DB" "$COLLECTION" "$DOCUMENT_COUNT"
printf 'Clients: %s; workers: %s; duration: %ss; mode: prepared\n' \
    "$CLIENTS" "$JOBS" "$DURATION"

pgbench -n -M prepared \
    -c "$CLIENTS" -j "$JOBS" -T "$DURATION" \
    -D "user_oid=$user_oid" -D "document_count=$DOCUMENT_COUNT" \
    -f "$sql_file"
