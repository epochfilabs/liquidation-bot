#!/usr/bin/env python3
"""
Fetch real mainnet liquidation transaction fixtures for testing.
Searches for liquidation transactions across 4 Solana lending protocols.
"""
import json
import subprocess
import os
import time

RPC = "https://asymmetr-solanam-0245.mainnet.rpcpool.com/f976db24-7f7d-4345-9284-2783b152e483"
BASE = "/Users/liam/dev/epochfilabs/liquidation-bot/tests/fixtures"

VENUES = {
    "kamino": {
        "program": "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD",
        "keywords": ["liquidat"],
    },
    "marginfi": {
        "program": "MFv2hWf31Z9kbCa1snEPYctwafyhdvnV7FZnsebVacA",
        "keywords": ["liquidat"],
    },
    "save": {
        "program": "So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo",
        "keywords": ["liquidat"],
    },
    "jupiter-lend": {
        "program": "jupr81YtYssSyPt8jbnGuiWon5f6x9TcDEFxYe3Bdzi",
        "keywords": ["liquidat"],
    },
}


def rpc_call(method, params):
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    result = subprocess.run(
        ["curl", "-s", "-X", "POST", RPC, "-H", "Content-Type: application/json", "-d", payload],
        capture_output=True, text=True, timeout=30
    )
    return json.loads(result.stdout)


def is_liquidation(tx_result, keywords):
    if not tx_result:
        return False
    meta = tx_result.get("meta", {})
    if not meta:
        return False
    logs = meta.get("logMessages", [])
    log_text = " ".join(logs).lower()
    return any(kw.lower() in log_text for kw in keywords)


def process_venue(name, info):
    out_dir = os.path.join(BASE, name)
    os.makedirs(out_dir, exist_ok=True)
    found = 0
    before = None

    for batch in range(15):  # up to 750 txs
        if found >= 3:
            break
        params = [info["program"], {"limit": 50}]
        if before:
            params[1]["before"] = before

        data = rpc_call("getSignaturesForAddress", params)
        sigs_data = data.get("result", [])
        if not sigs_data:
            break

        sigs = [x["signature"] for x in sigs_data if x.get("err") is None]
        before = sigs_data[-1]["signature"]

        print(f"[{name}] batch {batch+1}: checking {len(sigs)} sigs (found {found}/3)")

        for sig in sigs:
            if found >= 3:
                break
            try:
                tx_data = rpc_call("getTransaction", [sig, {"encoding": "json", "maxSupportedTransactionVersion": 0}])
                tx_result = tx_data.get("result")
                if tx_result and is_liquidation(tx_result, info["keywords"]):
                    found += 1
                    fp = os.path.join(out_dir, f"{sig[:12]}.json")
                    with open(fp, "w") as f:
                        json.dump(tx_result, f, indent=2)
                    print(f"  FOUND #{found}: {sig[:40]}...")
                time.sleep(0.05)
            except Exception as e:
                print(f"  error: {e}")
                time.sleep(1)

    print(f"[{name}] total found: {found}")
    return found


if __name__ == "__main__":
    for name, info in VENUES.items():
        process_venue(name, info)
    print("Done!")
