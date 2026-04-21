#!/usr/bin/env python3
"""
Enrich ClickHouse liquidation data with oracle prices from free APIs.

Uses CoinGecko (free, no API key) for major token prices at daily granularity,
then updates ClickHouse repay_amount_usd, debt_price, and collateral_price columns.

For minute-level precision, use Birdeye or Pyth APIs (requires API key).
Daily prices are sufficient for aggregate P&L analysis.

Usage:
    python3 scripts/enrich_prices_birdeye.py \
        --clickhouse-url http://localhost:8123 \
        --clickhouse-db liquidation_indexer \
        --clickhouse-user default \
        --clickhouse-password dev
"""

import json
import subprocess
import sys
import time
from datetime import datetime, timedelta

# Known Solana token mint → CoinGecko ID mapping
MINT_TO_COINGECKO = {
    "So11111111111111111111111111111111111111112": ("solana", 9),
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v": ("usd-coin", 6),
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB": ("tether", 6),
    "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn": ("jito-staked-sol", 9),
    "mSoLzYCxHdYgdzU16gzaiJ5Gk2Ur7VFinzRzDaXiXLk": ("msol", 9),
    "bSo13r4TkiE4KumL71LsHTPpL2euBYLFx6h9HP3piy1": ("blazestake-staked-sol", 9),
    "jupSoLaHXQiZZTSfEWMTRRgpnyFm8f6sZdosWBjx93v": ("jupiter-staked-sol", 9),
    "27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4": ("jupiter-perpetuals-liquidity-provider-token", 6),
    "USDSwr9ApdHk5bvJKMjRYHPniMZswFLPBjBT3HGER2o": ("usds", 6),
    "2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo": ("paypal-usd", 6),
    "2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH": ("agora-dollar", 6),
    # Stablecoins that are ~$1
    "HzwqbKZw8HxMN6bF2yFHNzR6SC9d3USSvHbKELLzUngg": (None, 6),  # PYUSD pool token
    "CASHx9KJnvSvnmhJLUdKncTKRwLcYRCqsNLxp1tQFeXz": (None, 6),  # CASH
}

# Stablecoin mints (assume $1.00)
STABLECOIN_MINTS = {
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",  # USDC
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",  # USDT
    "USDSwr9ApdHk5bvJKMjRYHPniMZswFLPBjBT3HGER2o",  # USDS
    "2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo",  # PYUSD
    "2u1tszSeqZ3qBWF3uNGPFc8TzMk2tdiwknnRMWGWjGWH",  # AUSD
    "HzwqbKZw8HxMN6bF2yFHNzR6SC9d3USSvHbKELLzUngg",  # PYUSD v2
    "CASHx9KJnvSvnmhJLUdKncTKRwLcYRCqsNLxp1tQFeXz",  # CASH
}

def ch_query(url, db, user, password, query):
    """Execute a ClickHouse query and return the result."""
    auth = f"user={user}&password={password}&database={db}"
    result = subprocess.run(
        ["/usr/bin/curl", "-s", f"{url}/?{auth}", "-d", query],
        capture_output=True, text=True, timeout=30
    )
    return result.stdout.strip()

def get_coingecko_price(coin_id, date_str):
    """Get price from CoinGecko for a specific date. Free, no API key."""
    # date_str format: dd-mm-yyyy
    url = f"https://api.coingecko.com/api/v3/coins/{coin_id}/history?date={date_str}&localization=false"
    result = subprocess.run(
        ["/usr/bin/curl", "-s", url],
        capture_output=True, text=True, timeout=15
    )
    try:
        data = json.loads(result.stdout)
        return data.get("market_data", {}).get("current_price", {}).get("usd")
    except:
        return None

def main():
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--clickhouse-url", default="http://localhost:8123")
    parser.add_argument("--clickhouse-db", default="liquidation_indexer")
    parser.add_argument("--clickhouse-user", default="default")
    parser.add_argument("--clickhouse-password", default="dev")
    args = parser.parse_args()

    ch = lambda q: ch_query(args.clickhouse_url, args.clickhouse_db, args.clickhouse_user, args.clickhouse_password, q)

    # Get unique (date, mint) pairs that need prices
    print("Finding unique mints and dates needing prices...")
    result = ch("""
        SELECT DISTINCT
            toDate(block_time) as day,
            replaceAll(debt_mint, '\\0', '') as mint
        FROM liquidations
        WHERE debt_price IS NULL AND venue = 'kamino'
        UNION DISTINCT
        SELECT DISTINCT
            toDate(block_time) as day,
            replaceAll(collateral_mint, '\\0', '') as mint
        FROM liquidations
        WHERE collateral_price IS NULL AND venue = 'kamino'
        FORMAT JSONEachRow
    """)

    if not result:
        print("No rows need enrichment")
        return

    pairs = []
    for line in result.strip().split("\n"):
        if line:
            row = json.loads(line)
            pairs.append((row["day"], row["mint"]))

    print(f"Need prices for {len(pairs)} (date, mint) pairs")

    # Fetch prices
    price_cache = {}  # (date, mint) -> price
    coingecko_calls = 0

    for day, mint in pairs:
        if mint in STABLECOIN_MINTS:
            price_cache[(day, mint)] = 1.0
            continue

        info = MINT_TO_COINGECKO.get(mint)
        if not info or not info[0]:
            # Unknown mint — skip
            continue

        coin_id, decimals = info
        cache_key = (day, coin_id)
        if cache_key in price_cache:
            price_cache[(day, mint)] = price_cache[cache_key]
            continue

        # CoinGecko rate limit: 10-30 calls/minute on free tier
        date_obj = datetime.strptime(day, "%Y-%m-%d")
        date_str = date_obj.strftime("%d-%m-%Y")

        print(f"  Fetching {coin_id} price for {day}...")
        price = get_coingecko_price(coin_id, date_str)
        if price:
            price_cache[(day, mint)] = price
            price_cache[cache_key] = price
            print(f"    ${price}")
        else:
            print(f"    Not found")

        coingecko_calls += 1
        if coingecko_calls % 10 == 0:
            time.sleep(2)  # Rate limit

    print(f"\nFetched {len(price_cache)} prices. Updating ClickHouse...")

    # Update ClickHouse — batch by date for efficiency
    dates = sorted(set(day for day, _ in price_cache.keys()))

    for day in dates:
        # Get all mints for this day
        day_prices = {mint: price for (d, mint), price in price_cache.items() if d == day}

        for mint, price in day_prices.items():
            info = MINT_TO_COINGECKO.get(mint)
            decimals = info[1] if info else 6

            # Update debt_price where debt_mint matches
            mint_padded = mint  # ClickHouse stores with null padding
            ch(f"""
                ALTER TABLE liquidations UPDATE
                    debt_price = {price},
                    repay_amount_usd = toDecimal64(repay_amount / {10**decimals} * {price}, 6)
                WHERE toDate(block_time) = '{day}'
                  AND debt_mint LIKE '{mint}%'
                  AND debt_price IS NULL
            """)

            # Update collateral_price
            ch(f"""
                ALTER TABLE liquidations UPDATE
                    collateral_price = {price}
                WHERE toDate(block_time) = '{day}'
                  AND collateral_mint LIKE '{mint}%'
                  AND collateral_price IS NULL
            """)

        print(f"  Updated {day}: {len(day_prices)} token prices")

    # Compute liquidator_profit_usd where both prices are set
    print("\nComputing liquidator_profit_usd...")
    ch("""
        ALTER TABLE liquidations UPDATE
            obligation_borrowed_usd = repay_amount_usd
        WHERE repay_amount_usd IS NOT NULL AND obligation_borrowed_usd IS NULL
    """)

    # Final stats
    result = ch("""
        SELECT
            count() as total,
            countIf(debt_price IS NOT NULL) as has_debt_price,
            countIf(collateral_price IS NOT NULL) as has_col_price,
            countIf(repay_amount_usd IS NOT NULL) as has_usd
        FROM liquidations
        WHERE venue = 'kamino'
        FORMAT PrettyCompact
    """)
    print(f"\n{result}")

if __name__ == "__main__":
    main()
