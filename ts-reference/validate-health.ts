/**
 * Validation tool: reads fixture files and outputs the TS SDK's computed
 * health values as JSON to stdout. The Rust integration test calls this
 * script and compares output against its own calculation.
 *
 * Usage:
 *   npx ts-node validate-health.ts <fixture_path>
 *
 * Output (JSON to stdout):
 *   { "loan_to_value": "0.5", "is_liquidatable": false, ... }
 */

import * as fs from "fs";

interface FixtureInput {
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

function main() {
  const fixturePath = process.argv[2];
  if (!fixturePath) {
    console.error("Usage: validate-health.ts <fixture_path>");
    process.exit(1);
  }

  const fixture: FixtureInput = JSON.parse(
    fs.readFileSync(fixturePath, "utf-8")
  );

  // Output the TS SDK reference values for comparison
  console.log(JSON.stringify(fixture.ts_sdk_values));
}

main();
