/**
 * Snapshot tool: fetches live obligation accounts from Kamino Lend,
 * computes health using the TS SDK, and writes fixture files that
 * the Rust tests consume for cross-validation.
 *
 * Usage:
 *   RPC_URL=https://api.mainnet-beta.solana.com npx ts-node snapshot-obligations.ts
 *
 * Output:
 *   ../tests/fixtures/obligation_<pubkey>.json
 */

import { Connection, PublicKey } from "@solana/web3.js";
import { KaminoMarket } from "@kamino-finance/klend-sdk";
import * as fs from "fs";
import * as path from "path";

const RPC_URL = process.env.RPC_URL || "https://api.mainnet-beta.solana.com";
const KAMINO_MARKET = new PublicKey(
  "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF"
);
const FIXTURES_DIR = path.join(__dirname, "..", "tests", "fixtures");

interface ObligationFixture {
  pubkey: string;
  raw_account_data_base64: string;
  ts_sdk_values: {
    loan_to_value: string;
    unhealthy_ltv: string;
    is_liquidatable: boolean;
    deposited_value: string;
    borrowed_value_bf_adjusted: string;
    unhealthy_borrow_value: string;
    borrowed_assets_market_value: string;
  };
}

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");
  console.log("Loading Kamino market...");
  const market = await KaminoMarket.load(connection, KAMINO_MARKET);
  if (!market) {
    throw new Error("Failed to load Kamino market");
  }

  await market.loadReserves();

  console.log("Fetching obligations...");
  const obligations = await market.getAllObligationsForMarket();

  // Take a sample: some healthy, some near-threshold, some liquidatable
  const samples = obligations.slice(0, 50);

  fs.mkdirSync(FIXTURES_DIR, { recursive: true });

  let fixtureCount = 0;
  for (const obligation of samples) {
    try {
      const ltv = obligation.loanToValue();
      const liquidationLtv = obligation.liquidationLtv();
      const isLiquidatable = ltv.gte(liquidationLtv) && ltv.gt(0);

      // Get the raw account data
      const accountInfo = await connection.getAccountInfo(
        obligation.obligationAddress
      );
      if (!accountInfo) continue;

      const fixture: ObligationFixture = {
        pubkey: obligation.obligationAddress.toBase58(),
        raw_account_data_base64: accountInfo.data.toString("base64"),
        ts_sdk_values: {
          loan_to_value: ltv.toString(),
          unhealthy_ltv: liquidationLtv.toString(),
          is_liquidatable: isLiquidatable,
          deposited_value: obligation.getDepositedValue().toString(),
          borrowed_value_bf_adjusted: obligation
            .getBorrowedMarketValueBFAdjusted()
            .toString(),
          unhealthy_borrow_value: obligation
            .getUnhealthyBorrowValue()
            .toString(),
          borrowed_assets_market_value: obligation
            .getBorrowedMarketValue()
            .toString(),
        },
      };

      const filename = `obligation_${fixture.pubkey.slice(0, 8)}.json`;
      fs.writeFileSync(
        path.join(FIXTURES_DIR, filename),
        JSON.stringify(fixture, null, 2)
      );
      fixtureCount++;
      console.log(
        `  [${fixtureCount}] ${fixture.pubkey} ltv=${ltv.toFixed(4)} liquidatable=${isLiquidatable}`
      );
    } catch (e) {
      // Skip obligations that fail to evaluate
      continue;
    }
  }

  console.log(`\nWrote ${fixtureCount} fixture files to ${FIXTURES_DIR}`);
}

main().catch(console.error);
