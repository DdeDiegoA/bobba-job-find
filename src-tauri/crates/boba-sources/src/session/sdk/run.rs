//! The gate and page budget of one Session Board run (S11.1b, FR-30, AD-9,
//! AD-10).
//!
//! S11.1a left residual R1: nothing consumed the opt-in, the pause or the cap
//! before a session run. [`SessionRun`] is that consumer, shared by every
//! Session Board adapter:
//!
//! * [`SessionRun::open`] refuses a Source without a live opt-in, a Source a
//!   challenge paused, and a Source whose daily page budget is spent — no run
//!   starts without them;
//! * [`SessionRun::before_load`] hands out the human-like delay from
//!   [`DEFAULT_PACING`] and spends one page load, refusing past the cap;
//! * the caller persists [`SessionRun::loads`] after the run, so the cap is a
//!   daily total, not a per-run one.
//!
//! The gate itself is [`check`], so a caller that only needs "may this Source
//! run at all" (a health check, the run report) asks the same question.

use super::{BoardView, ChallengeKind, DEFAULT_PACING, Pacing};

/// Why a Session Board run was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// The Source has no recorded opt-in (AD-9).
    #[error("source '{source_id}' has no recorded opt-in")]
    OptInRequired { source_id: String },
    /// A challenge paused the Source (AD-10).
    #[error("source '{source_id}' is paused after a {kind:?} challenge")]
    Paused {
        source_id: String,
        kind: ChallengeKind,
    },
    /// Today's page budget is spent (FR-30: low volume).
    #[error("source '{source_id}' already used its {cap} page loads today")]
    CapReached { source_id: String, cap: u32 },
}

/// The opt-in and pause gate on its own: a Source runs only with a live
/// opt-in and no pause, and a failure is never softer than that.
pub fn check(board: &BoardView) -> Result<(), RunError> {
    if let Some(pause) = &board.pause {
        return Err(RunError::Paused {
            source_id: board.source_id.clone(),
            kind: pause.kind,
        });
    }
    if !board.opted_in || !board.enabled {
        return Err(RunError::OptInRequired {
            source_id: board.source_id.clone(),
        });
    }
    Ok(())
}

/// One run's gate, pacing and page budget.
#[derive(Debug, Clone)]
pub struct SessionRun {
    source_id: String,
    cap: u32,
    remaining: u32,
    loads: u32,
    pacing: Pacing,
}

impl SessionRun {
    /// Open a run for `board`, given the page loads already spent today.
    ///
    /// Refuses without a live opt-in, with a recorded pause, and when the cap
    /// is spent — before the adapter reads a single page (R1).
    pub fn open(board: &BoardView, cap: u32, used_today: u32) -> Result<Self, RunError> {
        check(board)?;
        if cap == 0 || used_today >= cap {
            return Err(RunError::CapReached {
                source_id: board.source_id.clone(),
                cap,
            });
        }
        Ok(Self {
            source_id: board.source_id.clone(),
            cap,
            remaining: cap.saturating_sub(used_today),
            loads: 0,
            pacing: DEFAULT_PACING,
        })
    }

    /// The human-like delay before the next page load, then spend it.
    ///
    /// `sample` is any changing number (a counter, a timer): it only jitters
    /// the delay, never lowers it below the pacing floor.
    pub fn before_load(&mut self, sample: u64) -> Result<u64, RunError> {
        if self.remaining == 0 {
            return Err(RunError::CapReached {
                source_id: self.source_id.clone(),
                cap: self.cap,
            });
        }
        self.remaining -= 1;
        self.loads += 1;
        Ok(self.pacing.delay_ms(sample))
    }

    /// Page loads this run has spent (persist it against the daily cap).
    pub fn loads(&self) -> u32 {
        self.loads
    }

    /// Page loads this run may still spend.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// The daily cap this run was opened with.
    pub fn cap(&self) -> u32 {
        self.cap
    }

    /// The pacing every load of this run respects.
    pub fn pacing(&self) -> Pacing {
        self.pacing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::sdk::PauseRecord;

    const NOW: &str = "2026-10-17T06:00:00.000Z";

    fn board() -> BoardView {
        BoardView {
            source_id: "linkedin".into(),
            name: "LinkedIn".into(),
            enabled: true,
            opted_in: true,
            accepted_at: Some(NOW.into()),
            accepted_warning_version: Some("v1".into()),
            pause: None,
            weak_ack: false,
        }
    }

    #[test]
    fn a_source_without_an_opt_in_never_runs() {
        let mut view = board();
        view.opted_in = false;
        assert_eq!(
            SessionRun::open(&view, 60, 0).unwrap_err(),
            RunError::OptInRequired {
                source_id: "linkedin".into()
            }
        );
        assert_eq!(
            check(&view).unwrap_err(),
            RunError::OptInRequired {
                source_id: "linkedin".into()
            }
        );
    }

    #[test]
    fn a_paused_source_never_runs() {
        let mut view = board();
        view.pause = Some(PauseRecord::new(ChallengeKind::Captcha, NOW));
        assert_eq!(
            SessionRun::open(&view, 60, 0).unwrap_err(),
            RunError::Paused {
                source_id: "linkedin".into(),
                kind: ChallengeKind::Captcha,
            }
        );
    }

    #[test]
    fn a_cap_already_spent_refuses_to_open() {
        assert_eq!(
            SessionRun::open(&board(), 60, 60).unwrap_err(),
            RunError::CapReached {
                source_id: "linkedin".into(),
                cap: 60
            }
        );
        assert!(
            SessionRun::open(&board(), 0, 0).is_err(),
            "no budget, no run"
        );
    }

    #[test]
    fn a_run_never_loads_more_than_the_daily_cap() {
        let mut run = SessionRun::open(&board(), 3, 1).unwrap();
        assert_eq!(run.remaining(), 2);
        run.before_load(0).unwrap();
        run.before_load(1).unwrap();
        assert_eq!(
            run.before_load(2).unwrap_err(),
            RunError::CapReached {
                source_id: "linkedin".into(),
                cap: 3
            }
        );
        assert_eq!(run.loads(), 2, "a refused load spends nothing");
    }

    #[test]
    fn every_load_is_paced_like_a_person() {
        let mut run = SessionRun::open(&board(), 10, 0).unwrap();
        let floor = DEFAULT_PACING.min_interval_ms - DEFAULT_PACING.min_interval_ms * 40 / 100;
        let delays: Vec<u64> = (0..10).map(|n| run.before_load(n).unwrap()).collect();
        assert!(delays.iter().all(|delay| *delay >= floor));
        assert!(
            delays.iter().any(|delay| *delay != delays[0]),
            "not a metronome"
        );
        assert_eq!(run.loads(), 10);
        assert_eq!(run.remaining(), 0);
        assert_eq!(run.cap(), 10);
    }
}
