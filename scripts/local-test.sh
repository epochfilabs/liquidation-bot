#!/usr/bin/env bash
set -euo pipefail

# Local integration test script for the liquidation indexer.
#
# Prerequisites:
#   - Docker installed and running
#   - Rust toolchain
#
# Usage:
#   ./scripts/local-test.sh           # Full test: start ClickHouse, apply schema, run tests
#   ./scripts/local-test.sh --no-ch   # Skip ClickHouse, just run fixture processing tests
#   ./scripts/local-test.sh --backfill <start_slot> <end_slot>   # Run live backfill

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

CH_URL="http://localhost:8123"
CH_DB="liquidation_indexer"

# -------------------------------------------------------
# Step 1: Start ClickHouse
# -------------------------------------------------------
start_clickhouse() {
    echo "==> Starting ClickHouse via Docker Compose..."
    docker compose up -d

    echo "==> Waiting for ClickHouse to be ready..."
    for i in $(seq 1 30); do
        if curl -s "$CH_URL/ping" > /dev/null 2>&1; then
            echo "    ClickHouse is ready."
            return 0
        fi
        sleep 1
    done
    echo "    ERROR: ClickHouse did not start within 30 seconds."
    exit 1
}

# -------------------------------------------------------
# Step 2: Apply schema
# -------------------------------------------------------
apply_schema() {
    echo "==> Creating database..."
    curl -s "$CH_URL/" -d "CREATE DATABASE IF NOT EXISTS $CH_DB"

    echo "==> Applying schema migrations..."
    # Split the migration file by semicolons and execute each statement
    # (ClickHouse HTTP interface only accepts one statement per request)
    python3 -c "
import re
with open('schema/migrations/001_initial_schema.sql') as f:
    sql = f.read()

# Remove comments
sql = re.sub(r'--.*$', '', sql, flags=re.MULTILINE)

# Split on semicolons
statements = [s.strip() for s in sql.split(';') if s.strip()]

import urllib.request, urllib.parse
for i, stmt in enumerate(statements):
    url = f'$CH_URL/?database=$CH_DB'
    data = stmt.encode('utf-8')
    req = urllib.request.Request(url, data=data)
    try:
        urllib.request.urlopen(req)
        print(f'    Statement {i+1}/{len(statements)} OK')
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        if 'already exists' in body.lower():
            print(f'    Statement {i+1}/{len(statements)} SKIP (already exists)')
        else:
            print(f'    Statement {i+1}/{len(statements)} ERROR: {body[:200]}')
"
    echo "    Schema applied."
}

# -------------------------------------------------------
# Step 3: Run fixture processing test (no ClickHouse needed)
# -------------------------------------------------------
run_fixture_tests() {
    echo ""
    echo "==> Running fixture processing tests..."
    echo "    This processes real mainnet transactions through the full decoder + processor pipeline."
    echo ""
    cargo test -p backfill --test local_integration process_all_venue_fixtures -- --nocapture 2>&1 | \
        grep -E "(\[|OK|FAIL|--|TOTAL|ERROR|===)" || true
}

# -------------------------------------------------------
# Step 4: Write fixtures to ClickHouse
# -------------------------------------------------------
write_to_clickhouse() {
    echo ""
    echo "==> Writing fixtures to ClickHouse..."
    CLICKHOUSE_URL="$CH_URL" CLICKHOUSE_DATABASE="$CH_DB" \
        cargo test -p backfill --test local_integration write_fixtures_to_clickhouse -- --nocapture 2>&1 | \
        grep -E "(write|SKIP|ERROR|flush)" || true
}

# -------------------------------------------------------
# Step 5: Query ClickHouse to verify data
# -------------------------------------------------------
verify_data() {
    echo ""
    echo "==> Verifying data in ClickHouse..."
    echo ""

    echo "--- Liquidations per venue ---"
    curl -s "$CH_URL/?database=$CH_DB" -d \
        "SELECT venue, count() as count FROM liquidations GROUP BY venue ORDER BY venue FORMAT PrettyCompact"
    echo ""

    echo "--- Failed attempts per venue ---"
    curl -s "$CH_URL/?database=$CH_DB" -d \
        "SELECT venue, count() as count, groupArray(error_message) as errors FROM failed_liquidation_attempts GROUP BY venue ORDER BY venue FORMAT PrettyCompact"
    echo ""

    echo "--- Transaction metadata ---"
    curl -s "$CH_URL/?database=$CH_DB" -d \
        "SELECT count() as tx_count, sum(jito_tip_lamports) as total_jito_tips FROM tx_metadata FORMAT PrettyCompact"
    echo ""

    echo "--- All events summary ---"
    curl -s "$CH_URL/?database=$CH_DB" -d "
        SELECT
            'liquidations' as tbl, count() as rows FROM liquidations
        UNION ALL
        SELECT
            'failed_attempts', count() FROM failed_liquidation_attempts
        UNION ALL
        SELECT
            'obligations_snapshots', count() FROM obligations_snapshots
        UNION ALL
        SELECT
            'reserves_snapshots', count() FROM reserves_snapshots
        UNION ALL
        SELECT
            'tx_metadata', count() FROM tx_metadata
        FORMAT PrettyCompact
    "
    echo ""
}

# -------------------------------------------------------
# Step 6: Run live backfill against Triton RPC
# -------------------------------------------------------
run_backfill() {
    local start_slot=$1
    local end_slot=$2

    echo ""
    echo "==> Running live backfill: slots $start_slot to $end_slot"

    # Load .env for RPC URL
    if [ -f .env ]; then
        export $(grep -v '^#' .env | xargs)
    fi

    BACKFILL_START_SLOT=$start_slot \
    BACKFILL_END_SLOT=$end_slot \
    CLICKHOUSE_URL="$CH_URL" \
    CLICKHOUSE_DATABASE="$CH_DB" \
    RUST_LOG=info \
        cargo run -p backfill 2>&1
}

# -------------------------------------------------------
# Main
# -------------------------------------------------------
case "${1:-}" in
    --no-ch)
        run_fixture_tests
        ;;
    --backfill)
        start_clickhouse
        apply_schema
        run_backfill "${2:?start_slot required}" "${3:?end_slot required}"
        verify_data
        ;;
    --verify)
        verify_data
        ;;
    --schema)
        start_clickhouse
        apply_schema
        ;;
    *)
        start_clickhouse
        apply_schema
        run_fixture_tests
        write_to_clickhouse
        verify_data
        echo ""
        echo "=== Local test complete ==="
        echo ""
        echo "To run a live backfill (known Kamino liquidation slots):"
        echo "  ./scripts/local-test.sh --backfill 414544140 414544150"
        echo ""
        echo "To query ClickHouse directly:"
        echo "  curl 'http://localhost:8123/?database=liquidation_indexer' -d 'SELECT * FROM liquidations FORMAT PrettyCompact'"
        echo ""
        echo "To stop ClickHouse:"
        echo "  docker compose down"
        ;;
esac
