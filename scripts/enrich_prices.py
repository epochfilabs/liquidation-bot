#!/usr/bin/env python3
"""
Enrich ClickHouse liquidation data with USD oracle prices from Dune.

Downloads price data from a Dune query execution and updates the
repay_amount_usd, collateral_price, and debt_price columns in ClickHouse.

Usage:
    # 1. Execute the oracle price export query on Dune (query 7351659)
    # 2. Download the CSV:
    export DUNE_API_KEY=your_key
    python3 scripts/enrich_prices.py \
        --execution-id 01KPRG6RENE3RWJYRDQFQYDDCF \
        --clickhouse-url http://localhost:8123 \
        --clickhouse-db liquidation_indexer \
        --clickhouse-user default \
        --clickhouse-password dev
"""

import argparse
import csv
import io
import json
import os
import subprocess
import sys

def download_dune_csv(api_key, execution_id):
    """Download results from a Dune execution as CSV."""
    url = f"https://api.dune.com/api/v1/execution/{execution_id}/results/csv"
    result = subprocess.run(
        ["/usr/bin/curl", "-s", url, "-H", f"X-Dune-API-Key: {api_key}"],
        capture_output=True, text=True, timeout=60
    )
    return result.stdout

def update_clickhouse(ch_url, ch_db, ch_user, ch_password, prices):
    """Update ClickHouse liquidations table with oracle prices."""

    # Group prices by tx_id
    tx_prices = {}
    for row in prices:
        tx_id = row['tx_id']
        if tx_id not in tx_prices:
            tx_prices[tx_id] = {}
        price_str = row.get('price', '')
        if not price_str or price_str in ('', '<nil>', 'None', 'null'):
            continue
        try:
            price_val = float(price_str)
        except (ValueError, TypeError):
            continue
        decimals_str = row.get('decimals', '6')
        try:
            decimals_val = int(decimals_str) if decimals_str and decimals_str not in ('<nil>', 'None') else 6
        except (ValueError, TypeError):
            decimals_val = 6

        if row['role'] == 'debt':
            tx_prices[tx_id]['debt_price'] = price_val
            tx_prices[tx_id]['debt_decimals'] = decimals_val
            tx_prices[tx_id]['debt_symbol'] = row.get('symbol', '')
        elif row['role'] == 'collateral':
            tx_prices[tx_id]['collateral_price'] = price_val
            tx_prices[tx_id]['col_decimals'] = decimals_val
            tx_prices[tx_id]['col_symbol'] = row.get('symbol', '')

    print(f"Prices for {len(tx_prices)} transactions")

    # Build ALTER TABLE UPDATE statements in batches
    updated = 0
    skipped = 0

    for tx_id, p in tx_prices.items():
        debt_price = p.get('debt_price')
        col_price = p.get('collateral_price')

        if not debt_price and not col_price:
            skipped += 1
            continue

        # Build SET clause
        sets = []
        if debt_price:
            sets.append(f"debt_price = {debt_price}")
        if col_price:
            sets.append(f"collateral_price = {col_price}")

        # Compute repay_amount_usd if we have debt price and decimals
        if debt_price:
            decimals = p.get('debt_decimals', 6)
            sets.append(f"repay_amount_usd = repay_amount / {10**decimals} * {debt_price}")

        set_clause = ", ".join(sets)

        # ClickHouse ALTER TABLE UPDATE
        # Note: tx_signature in CH has null padding, so use LIKE
        sig_escaped = tx_id.replace("'", "\\'")
        query = f"ALTER TABLE liquidations UPDATE {set_clause} WHERE tx_signature LIKE '{sig_escaped}%'"

        auth = f"user={ch_user}&password={ch_password}&database={ch_db}"
        result = subprocess.run(
            ["/usr/bin/curl", "-s", f"{ch_url}/?{auth}", "-d", query],
            capture_output=True, text=True, timeout=10
        )

        if result.stdout.strip():
            print(f"  Error on {tx_id[:12]}: {result.stdout[:100]}")
        else:
            updated += 1

        if updated % 500 == 0 and updated > 0:
            print(f"  Updated {updated}...")

    # Also update failed_liquidation_attempts
    for tx_id, p in tx_prices.items():
        debt_price = p.get('debt_price')
        if not debt_price:
            continue
        decimals = p.get('debt_decimals', 6)
        sig_escaped = tx_id.replace("'", "\\'")
        sets = [f"debt_price = {debt_price}"]
        sets.append(f"repay_amount_usd = repay_amount / {10**decimals} * {debt_price}")
        if p.get('collateral_price'):
            sets.append(f"collateral_price = {p['collateral_price']}")
        set_clause = ", ".join(sets)
        query = f"ALTER TABLE failed_liquidation_attempts UPDATE {set_clause} WHERE tx_signature LIKE '{sig_escaped}%'"
        subprocess.run(
            ["/usr/bin/curl", "-s", f"{ch_url}/?user={ch_user}&password={ch_password}&database={ch_db}", "-d", query],
            capture_output=True, text=True, timeout=10
        )

    print(f"\nDone: {updated} updated, {skipped} skipped (no price)")

def main():
    parser = argparse.ArgumentParser(description="Enrich ClickHouse with Dune oracle prices")
    parser.add_argument("--execution-id", required=True, help="Dune execution ID for the price export query")
    parser.add_argument("--clickhouse-url", default="http://localhost:8123")
    parser.add_argument("--clickhouse-db", default="liquidation_indexer")
    parser.add_argument("--clickhouse-user", default="default")
    parser.add_argument("--clickhouse-password", default="dev")
    args = parser.parse_args()

    api_key = os.environ.get("DUNE_API_KEY")
    if not api_key:
        print("Set DUNE_API_KEY environment variable")
        sys.exit(1)

    print(f"Downloading prices from Dune execution {args.execution_id}...")
    csv_data = download_dune_csv(api_key, args.execution_id)

    if not csv_data or csv_data.startswith("<!"):
        print(f"Error: {csv_data[:200]}")
        sys.exit(1)

    reader = csv.DictReader(io.StringIO(csv_data))
    prices = list(reader)
    print(f"Downloaded {len(prices)} price rows")

    print(f"\nUpdating ClickHouse at {args.clickhouse_url}/{args.clickhouse_db}...")
    update_clickhouse(
        args.clickhouse_url, args.clickhouse_db,
        args.clickhouse_user, args.clickhouse_password,
        prices
    )

if __name__ == "__main__":
    main()
