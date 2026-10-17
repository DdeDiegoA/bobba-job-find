//! What one daily search run reports (S5.2, FR-46, NFR-9, NFR-14).
//!
//! The report is the run's own account of itself: how every Source did, what
//! ingestion made of what they returned, what scoring found, and how many agent
//! tasks the run spent. It is persisted as one JSON row (the command layer owns
//! that) and read back by the run-report screen.
//!
//! The four ingestion totals are **derived** from the per-Source entries, never
//! stored beside them: a total that disagrees with its parts is a bug the type
//! cannot have.

use serde::{Deserialize, Serialize};

/// How one Source's turn in a run ended (FR-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOutcome {
    /// The Source answered; its postings went through ingestion.
    Ok,
    /// The Source failed; the run went on with the others (AC1).
    Failed,
}

impl SourceOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
        }
    }
}

/// One Source's line in the report (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceReport {
    /// Registry id, e.g. `remotive` or `rss:jobs-example-com-feed-xml`.
    pub source_id: String,
    /// The name the Source declares.
    pub name: String,
    pub outcome: SourceOutcome,
    /// Postings the Source returned this run.
    pub fetched: u32,
    /// Postings that became a new Offer.
    pub new: u32,
    /// Postings that merged into an Offer the store already had (AD-19).
    pub duplicates: u32,
    /// The failure, diagnostic text for the `source_run` row; never UI prose.
    pub error: Option<String>,
}

/// The ingestion totals of a run, derived from its Sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunTotals {
    /// Sources the run queried, failed ones included.
    pub queried: u32,
    pub fetched: u32,
    pub new: u32,
    pub duplicates: u32,
}

/// Everything a finished run reports (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// The id the report is stored and shown under.
    pub run_id: String,
    /// The run this one retried the failed Sources of, when it was a retry.
    pub retry_of: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: i64,
    pub sources: Vec<SourceReport>,
    /// Offers the Gates VETOed (FR-54).
    pub vetoed: u32,
    /// Offers that got a ScoreCard this run.
    pub ranked: u32,
    /// Agent tasks the run spent (the run budget, S2.5).
    pub agent_tasks_used: u32,
    /// Diagnostics the report screen shows as details, never as the outcome.
    pub errors: Vec<String>,
}

impl RunReport {
    /// The four ingestion totals, from the Sources themselves.
    pub fn totals(&self) -> RunTotals {
        let mut totals = RunTotals {
            queried: self.sources.len() as u32,
            ..RunTotals::default()
        };
        for source in &self.sources {
            totals.fetched += source.fetched;
            totals.new += source.new;
            totals.duplicates += source.duplicates;
        }
        totals
    }

    /// The Sources whose turn failed, in report order: what the partial-failure
    /// banner lists with Retry (AC3).
    pub fn failed_sources(&self) -> Vec<&SourceReport> {
        self.sources
            .iter()
            .filter(|source| source.outcome == SourceOutcome::Failed)
            .collect()
    }

    /// Whether the run finished with at least one Source failed (AC3).
    pub fn is_partial(&self) -> bool {
        self.sources
            .iter()
            .any(|source| source.outcome == SourceOutcome::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(
        id: &str,
        outcome: SourceOutcome,
        fetched: u32,
        new: u32,
        duplicates: u32,
    ) -> SourceReport {
        SourceReport {
            source_id: id.to_string(),
            name: id.to_string(),
            outcome,
            fetched,
            new,
            duplicates,
            error: (outcome == SourceOutcome::Failed).then(|| "HTTP 503".to_string()),
        }
    }

    /// AC2: the header totals are the sum of the Sources, not a second count
    /// that could drift.
    #[test]
    fn the_totals_are_the_sum_of_the_sources() {
        let report = RunReport {
            run_id: "01J".to_string(),
            retry_of: None,
            started_at: "2026-10-17T08:00:00Z".to_string(),
            finished_at: "2026-10-17T08:01:00Z".to_string(),
            duration_ms: 60_000,
            sources: vec![
                source("remotive", SourceOutcome::Ok, 12, 9, 3),
                source("himalayas", SourceOutcome::Ok, 5, 2, 3),
                source("hn", SourceOutcome::Failed, 0, 0, 0),
            ],
            vetoed: 1,
            ranked: 10,
            agent_tasks_used: 4,
            errors: Vec::new(),
        };

        assert_eq!(
            report.totals(),
            RunTotals {
                queried: 3,
                fetched: 17,
                new: 11,
                duplicates: 6,
            }
        );
    }

    /// AC3: only the Sources that failed are listed for Retry.
    #[test]
    fn a_partial_run_lists_exactly_its_failed_sources() {
        let report = RunReport {
            run_id: "01J".to_string(),
            retry_of: None,
            started_at: "2026-10-17T08:00:00Z".to_string(),
            finished_at: "2026-10-17T08:01:00Z".to_string(),
            duration_ms: 60_000,
            sources: vec![
                source("remotive", SourceOutcome::Ok, 12, 9, 3),
                source("hn", SourceOutcome::Failed, 0, 0, 0),
                source("wwr", SourceOutcome::Failed, 0, 0, 0),
            ],
            vetoed: 0,
            ranked: 0,
            agent_tasks_used: 0,
            errors: Vec::new(),
        };

        assert!(report.is_partial());
        let failed: Vec<&str> = report
            .failed_sources()
            .iter()
            .map(|source| source.source_id.as_str())
            .collect();
        assert_eq!(failed, ["hn", "wwr"]);
    }

    /// The report survives its own JSON round-trip: that is how it is stored.
    #[test]
    fn the_report_round_trips_through_its_stored_json() {
        let report = RunReport {
            run_id: "01J".to_string(),
            retry_of: Some("01H".to_string()),
            started_at: "2026-10-17T08:00:00Z".to_string(),
            finished_at: "2026-10-17T08:01:00Z".to_string(),
            duration_ms: 60_000,
            sources: vec![source("remotive", SourceOutcome::Ok, 12, 9, 3)],
            vetoed: 2,
            ranked: 7,
            agent_tasks_used: 5,
            errors: vec!["lifecycle sweep failed".to_string()],
        };

        let json = serde_json::to_string(&report).expect("stored");
        let back: RunReport = serde_json::from_str(&json).expect("read");
        assert_eq!(back, report);
    }
}
