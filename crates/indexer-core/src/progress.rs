//! Indexer progress tracking.
//!
//! Tracks the last processed epoch/slot per venue and data source.
//! Persisted to the `_indexer_progress` ClickHouse table for resume.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Backfill state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillState {
    Pending,
    InProgress,
    Complete,
    Failed,
}

impl BackfillState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for BackfillState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Progress record for one venue + data source combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressRecord {
    pub venue: String,
    pub source: String,
    pub last_epoch: Option<u32>,
    pub last_slot: u64,
    pub last_signature: Option<String>,
    pub backfill_state: BackfillState,
    pub rows_liquidations: u64,
    pub rows_failed: u64,
    pub rows_total_processed: u64,
    pub error_message: Option<String>,
}

impl ProgressRecord {
    pub fn new(venue: &str, source: &str) -> Self {
        Self {
            venue: venue.to_string(),
            source: source.to_string(),
            last_epoch: None,
            last_slot: 0,
            last_signature: None,
            backfill_state: BackfillState::Pending,
            rows_liquidations: 0,
            rows_failed: 0,
            rows_total_processed: 0,
            error_message: None,
        }
    }

    /// Update progress after processing a batch of transactions.
    pub fn advance(&mut self, slot: u64, signature: &str, liquidations: u64, failed: u64, total: u64) {
        self.last_slot = slot;
        self.last_signature = Some(signature.to_string());
        self.rows_liquidations += liquidations;
        self.rows_failed += failed;
        self.rows_total_processed += total;
    }

    /// Mark an epoch as complete.
    pub fn complete_epoch(&mut self, epoch: u32) {
        self.last_epoch = Some(epoch);
    }

    /// Mark the backfill as failed with an error message.
    pub fn mark_failed(&mut self, error: &str) {
        self.backfill_state = BackfillState::Failed;
        self.error_message = Some(error.to_string());
    }

    /// Mark the backfill as complete.
    pub fn mark_complete(&mut self) {
        self.backfill_state = BackfillState::Complete;
        self.error_message = None;
    }
}

/// In-memory progress tracker for all venues.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    records: std::collections::HashMap<(String, String), ProgressRecord>,
}

impl ProgressTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create a progress record for a venue + source.
    pub fn get_or_create(&mut self, venue: &str, source: &str) -> &mut ProgressRecord {
        let key = (venue.to_string(), source.to_string());
        self.records
            .entry(key)
            .or_insert_with(|| ProgressRecord::new(venue, source))
    }

    /// Get a progress record if it exists.
    pub fn get(&self, venue: &str, source: &str) -> Option<&ProgressRecord> {
        self.records.get(&(venue.to_string(), source.to_string()))
    }

    /// Get the last processed slot for a venue. Returns 0 if no progress recorded.
    pub fn last_slot(&self, venue: &str, source: &str) -> u64 {
        self.get(venue, source).map_or(0, |r| r.last_slot)
    }

    /// All progress records.
    pub fn all_records(&self) -> Vec<&ProgressRecord> {
        self.records.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_tracking() {
        let mut tracker = ProgressTracker::new();
        let rec = tracker.get_or_create("kamino", "old_faithful");
        assert_eq!(rec.last_slot, 0);
        assert_eq!(rec.backfill_state, BackfillState::Pending);

        rec.advance(1000, "abc123", 5, 2, 100);
        assert_eq!(rec.last_slot, 1000);
        assert_eq!(rec.rows_liquidations, 5);
        assert_eq!(rec.rows_total_processed, 100);

        rec.complete_epoch(750);
        assert_eq!(rec.last_epoch, Some(750));

        assert_eq!(tracker.last_slot("kamino", "old_faithful"), 1000);
        assert_eq!(tracker.last_slot("save", "old_faithful"), 0);
    }
}
