//! Cross-protocol liquidation pipeline.
//!
//! [`executor`] is the entry point called by `main`. It picks the cheapest
//! flash-loan provider for the debt mint, asks the active protocol to build
//! its liquidation instruction, assembles the full transaction, and submits
//! it as a Jito bundle. [`profitability`] is a Kamino-specific profit
//! estimator kept for tests and future use.

pub mod executor;
pub mod profitability;
