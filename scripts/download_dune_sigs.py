#!/usr/bin/env python3
"""
Download liquidation signatures from a Dune query execution via the Dune MCP.

Since we can't directly call the Dune API (no API key exposed), this script
uses the Dune CSV export URL which is publicly accessible for executed queries.

Usage:
    # Get the query ID from Dune (already created: 7349938)
    # Open in browser to export CSV:
    open "https://dune.com/queries/7349938"

    # Or if you have a DUNE_API_KEY:
    export DUNE_API_KEY=your_key
    python3 scripts/download_dune_sigs.py --query-id 7349938 --output data/kamino_jan_2026.csv

    # Then run backfill:
    BACKFILL_SIGNATURES_FILE=data/kamino_jan_2026.csv cargo run -p backfill
"""

import argparse
import json
import os
import sys
import time
import urllib.request

def main():
    parser = argparse.ArgumentParser(description="Download Dune query results as CSV")
    parser.add_argument("--query-id", type=int, required=True, help="Dune query ID")
    parser.add_argument("--output", type=str, required=True, help="Output CSV file path")
    parser.add_argument("--execution-id", type=str, help="Existing execution ID (skip re-execution)")
    args = parser.parse_args()

    api_key = os.environ.get("DUNE_API_KEY")
    if not api_key:
        print("DUNE_API_KEY not set.")
        print()
        print("Two options to get the data:")
        print()
        print(f"  Option 1: Export from Dune UI (free)")
        print(f"    1. Open https://dune.com/queries/{args.query_id}")
        print(f"    2. Click 'Run' if not already executed")
        print(f"    3. Click 'Export CSV' button")
        print(f"    4. Save as {args.output}")
        print()
        print(f"  Option 2: Set DUNE_API_KEY and re-run this script")
        print(f"    export DUNE_API_KEY=your_key")
        print(f"    python3 {sys.argv[0]} --query-id {args.query_id} --output {args.output}")
        sys.exit(1)

    headers = {"X-Dune-API-Key": api_key}

    # Execute query if no execution ID provided
    if args.execution_id:
        exec_id = args.execution_id
    else:
        print(f"Executing query {args.query_id}...")
        req = urllib.request.Request(
            f"https://api.dune.com/api/v1/query/{args.query_id}/execute",
            data=json.dumps({}).encode(),
            headers={**headers, "Content-Type": "application/json"},
            method="POST",
        )
        resp = urllib.request.urlopen(req)
        result = json.loads(resp.read())
        exec_id = result["execution_id"]
        print(f"Execution ID: {exec_id}")

    # Poll for completion
    print("Waiting for execution to complete...")
    while True:
        req = urllib.request.Request(
            f"https://api.dune.com/api/v1/execution/{exec_id}/status",
            headers=headers,
        )
        resp = urllib.request.urlopen(req)
        status = json.loads(resp.read())
        state = status.get("state", "")
        if state == "QUERY_STATE_COMPLETED":
            print(f"Completed. Rows: {status.get('result_metadata', {}).get('total_row_count', '?')}")
            break
        elif state in ("QUERY_STATE_FAILED", "QUERY_STATE_CANCELLED"):
            print(f"Failed: {status}")
            sys.exit(1)
        else:
            print(f"  State: {state}...")
            time.sleep(2)

    # Download CSV results
    print(f"Downloading results to {args.output}...")
    req = urllib.request.Request(
        f"https://api.dune.com/api/v1/execution/{exec_id}/results/csv",
        headers=headers,
    )
    resp = urllib.request.urlopen(req)
    data = resp.read()

    with open(args.output, "wb") as f:
        f.write(data)

    # Count lines
    line_count = data.decode().count("\n") - 1  # subtract header
    print(f"Saved {line_count} signatures to {args.output}")
    print()
    print(f"Next step:")
    print(f"  BACKFILL_SIGNATURES_FILE={args.output} RUST_LOG=info cargo run -p backfill")

if __name__ == "__main__":
    main()
