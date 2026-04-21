#!/usr/bin/env python3
"""
Fetch all transaction signatures for a program within a slot range via Triton RPC.
Uses getSignaturesForAddress with pagination. Outputs one signature per line.

Usage:
    python3 scripts/fetch_sigs_rpc.py \
        --program KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD \
        --after-slot 395000000 --before-slot 410000000 \
        --output data/kamino_feb_2026.csv

For February 2026 Kamino, estimated slot range: ~395,000,000 - ~410,000,000
"""

import argparse
import json
import os
import subprocess
import sys
import time

def rpc(url, method, params):
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    for attempt in range(3):
        r = subprocess.run(
            ["/usr/bin/curl", "-s", url, "-X", "POST",
             "-H", "Content-Type: application/json", "-d", payload],
            capture_output=True, text=True, timeout=30
        )
        try:
            return json.loads(r.stdout)
        except:
            time.sleep(0.5)
    return {"result": None}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--program", required=True, help="Program ID to search")
    parser.add_argument("--after-slot", type=int, default=0, help="Only include sigs after this slot")
    parser.add_argument("--before-slot", type=int, default=0, help="Only include sigs before this slot")
    parser.add_argument("--output", required=True, help="Output file (one sig per line)")
    parser.add_argument("--include-failed", action="store_true", default=True, help="Include failed txs")
    args = parser.parse_args()

    rpc_url = os.environ.get("SOLANA_RPC_URL")
    if not rpc_url:
        print("Set SOLANA_RPC_URL"); sys.exit(1)

    print(f"Fetching signatures for {args.program}")
    print(f"Slot range: {args.after_slot} to {args.before_slot}")

    all_sigs = []
    before_sig = None
    batch = 0

    while True:
        params = [args.program, {"limit": 1000}]
        if before_sig:
            params[1]["before"] = before_sig

        result = rpc(rpc_url, "getSignaturesForAddress", params)
        sigs = result.get("result", [])

        if not sigs:
            print(f"  Batch {batch}: no more signatures")
            break

        # Filter by slot range
        filtered = []
        too_old = False
        for s in sigs:
            slot = s.get("slot", 0)
            if args.before_slot and slot >= args.before_slot:
                continue
            if args.after_slot and slot < args.after_slot:
                too_old = True
                continue
            if not args.include_failed and s.get("err") is not None:
                continue
            filtered.append(s["signature"])

        all_sigs.extend(filtered)
        before_sig = sigs[-1]["signature"]
        batch += 1

        if batch % 10 == 0:
            print(f"  Batch {batch}: {len(all_sigs)} sigs so far (last slot: {sigs[-1].get('slot', '?')})")

        if too_old:
            print(f"  Reached slot {args.after_slot}, stopping")
            break

        # Small delay to be nice to the RPC
        time.sleep(0.05)

    print(f"\nTotal: {len(all_sigs)} signatures")

    # Write output
    with open(args.output, "w") as f:
        f.write("tx_id\n")  # CSV header
        for sig in all_sigs:
            f.write(f"{sig}\n")

    print(f"Written to {args.output}")

if __name__ == "__main__":
    main()
