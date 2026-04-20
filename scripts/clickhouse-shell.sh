#!/usr/bin/env bash
# Interactive ClickHouse SQL shell via Docker.
#
# Usage:
#   ./scripts/clickhouse-shell.sh                    # Interactive shell
#   ./scripts/clickhouse-shell.sh "SELECT 1"         # Run a single query
#   echo "SELECT * FROM liquidations" | ./scripts/clickhouse-shell.sh  # Pipe a query

set -euo pipefail

DB="${CLICKHOUSE_DATABASE:-liquidation_indexer}"

# Check if ClickHouse container is running
if ! docker compose ps --status running 2>/dev/null | grep -q clickhouse; then
    echo "ClickHouse is not running. Starting it..."
    docker compose up -d
    echo "Waiting for ClickHouse..."
    for i in $(seq 1 15); do
        if curl -s http://localhost:8123/ping > /dev/null 2>&1; then
            break
        fi
        sleep 1
    done
fi

if [ $# -gt 0 ]; then
    # Single query mode
    docker compose exec -T clickhouse clickhouse-client --database="$DB" --query "$*"
else
    # Interactive mode
    echo "Connecting to ClickHouse database: $DB"
    echo "Type SQL queries. Use Ctrl-D to exit."
    echo ""
    docker compose exec clickhouse clickhouse-client --database="$DB" --multiline
fi
