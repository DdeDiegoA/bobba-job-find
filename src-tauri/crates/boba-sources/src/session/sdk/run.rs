//! The gate and page budget of one Session Board run (S11.1b, FR-30, AD-9,
//! AD-10, AD-12).
//!
//! S11.1a left residual R1: nothing consumed the opt-in, the pause or the cap
//! before a session run. [`SessionRun`] is that consumer, shared by every
//! Session Board adapter:
//!
//! * [`SessionRun::open`] refuses a Source without a live opt-in, a Source a
//!   challenge paused, and a Source whose daily page budget is spent — no run
//!   starts without them;
//! * [`SessionRun::before_load`] audits the page load as `session.run` through
//!   the [`RunControl`] the core provides, then hands out the human-like delay
//!   from [`DEFAULT_PACING`] and spends the page load, refusing past the cap;
//! * the core persists the new daily total with each audit, so the cap is a
//!   daily total, not a per-run one.
//!
//! The gate itself is [`check`], so a caller that only needs "may this Source
//! run at all" (the run report, a health check) asks the same question.

use std::sync::Arc;

use super::{BoardView, ChallengeKind, DEFAULT_PACING, Pacing};

/// What the core did with one page load (S11.1b R-a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSpend {
    /// The core paid for the load; this is the day's new total.
    Spent { loads_today: u32 },
    /// The cap is already spent; nothing may load.
    CapReached { cap: u32 },
}

/// What a run needs from the core around every page load: the board, the daily
/// counter, the audit row and the challenge pause. The app implements it over
/// [`super::SessionBoardService`]; a test implements it in memory.
pub trait RunControl: Send + Sync {
    /// The board as the core sees it (opt-in, pause, enabled) plus the page
    /// loads already spent today, in the user's local day.
    fn board(&self, source_id: &str) -> Result<(BoardView, u32), String>;

    /// Spend one page from today's budget, atomically, and audit it
    /// (`session.run`). The core owns the counter, so two overlapping runs
    /// share one budget.
    fn page_loaded(&self, source_id: &str, url: &str, cap: u32) -> Result<LoadSpend, String>;

    /// Pause a Source after a challenge: audit, persist and notify (AD-10).
    fn pause(&self, source_id: &str, kind: ChallengeKind) -> Result<(), String>;
}

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
    /// The core could not answer, so the load never happens (fail closed).
    #[error("the session core refused the load: {0}")]
    Control(String),
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
#[derive(Clone)]
pub struct SessionRun {
    control: Arc<dyn RunControl>,
    source_id: String,
    cap: u32,
    used_today: u32,
    remaining: u32,
    loads: u32,
    pacing: Pacing,
}

impl std::fmt::Debug for SessionRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRun")
            .field("source_id", &self.source_id)
            .field("cap", &self.cap)
            .field("used_today", &self.used_today)
            .field("loads", &self.loads)
            .finish_non_exhaustive()
    }
}

impl SessionRun {
    /// Open a run for `source_id` against the core.
    ///
    /// Refuses without a live opt-in, with a recorded pause, and when the cap
    /// is spent — before the adapter reads a single page (R1).
    pub fn open(control: Arc<dyn RunControl>, source_id: &str, cap: u32) -> Result<Self, RunError> {
        let (board, used_today) = control.board(source_id).map_err(RunError::Control)?;
        if board.source_id != source_id {
            return Err(RunError::Control(format!(
                "the core answered for '{}', not '{source_id}'",
                board.source_id
            )));
        }
        check(&board)?;
        if cap == 0 || used_today >= cap {
            return Err(RunError::CapReached {
                source_id: source_id.to_string(),
                cap,
            });
        }
        Ok(Self {
            control,
            source_id: source_id.to_string(),
            cap,
            used_today,
            remaining: cap - used_today,
            loads: 0,
            pacing: DEFAULT_PACING,
        })
    }

    /// Spend one page from the day's budget, then return the human-like delay
    /// before it.
    ///
    /// `url` is the page about to load, `sample` any changing number (a
    /// counter, a timer): it only jitters the delay, never lowers it below the
    /// pacing floor. The core's conditional spend is the authority on the cap,
    /// so two overlapping runs cannot both pass it (S11.1b R-a). A refused
    /// audit refuses the load: nothing touches the board unaudited.
    pub fn before_load(&mut self, url: &str, sample: u64) -> Result<u64, RunError> {
        if self.remaining == 0 {
            return Err(RunError::CapReached {
                source_id: self.source_id.clone(),
                cap: self.cap,
            });
        }
        let delay = self.pacing.delay_ms(sample);
        match self
            .control
            .page_loaded(&self.source_id, url, self.cap)
            .map_err(RunError::Control)?
        {
            LoadSpend::CapReached { cap } => Err(RunError::CapReached {
                source_id: self.source_id.clone(),
                cap,
            }),
            LoadSpend::Spent { loads_today } => {
                self.loads += 1;
                self.remaining = self.cap.saturating_sub(loads_today);
                Ok(delay)
            }
        }
    }

    /// Page loads this run has spent.
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
    use std::sync::Mutex;

    use super::*;
    use crate::session::sdk::PauseRecord;

    const NOW: &str = "2026-10-17T06:00:00.000Z";

    #[derive(Default)]
    struct Core {
        board: Mutex<Option<BoardView>>,
        used_today: Mutex<u32>,
        loads: Mutex<Vec<(String, u32)>>,
        paused: Mutex<Vec<ChallengeKind>>,
        fail: Mutex<bool>,
    }

    impl Core {
        fn with(board: BoardView, used_today: u32) -> Arc<Self> {
            Arc::new(Self {
                board: Mutex::new(Some(board)),
                used_today: Mutex::new(used_today),
                ..Self::default()
            })
        }
    }

    impl RunControl for Core {
        fn board(&self, _source_id: &str) -> Result<(BoardView, u32), String> {
            let board = self.board.lock().unwrap().clone().ok_or("no board")?;
            Ok((board, *self.used_today.lock().unwrap()))
        }

        fn page_loaded(&self, source_id: &str, _url: &str, cap: u32) -> Result<LoadSpend, String> {
            if *self.fail.lock().unwrap() {
                return Err("audit store down".into());
            }
            let mut used = self.used_today.lock().unwrap();
            if *used >= cap {
                return Ok(LoadSpend::CapReached { cap });
            }
            *used += 1;
            self.loads
                .lock()
                .unwrap()
                .push((source_id.to_string(), *used));
            Ok(LoadSpend::Spent { loads_today: *used })
        }

        fn pause(&self, _source_id: &str, kind: ChallengeKind) -> Result<(), String> {
            self.paused.lock().unwrap().push(kind);
            Ok(())
        }
    }

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
        let core = Core::with(view.clone(), 0);
        assert_eq!(
            SessionRun::open(core.clone(), "linkedin", 60).unwrap_err(),
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
            SessionRun::open(Core::with(view, 0), "linkedin", 60).unwrap_err(),
            RunError::Paused {
                source_id: "linkedin".into(),
                kind: ChallengeKind::Captcha,
            }
        );
    }

    #[test]
    fn a_board_for_another_source_is_refused() {
        let core = Core::with(board(), 0);
        assert!(matches!(
            SessionRun::open(core, "indeed", 60),
            Err(RunError::Control(_))
        ));
    }

    #[test]
    fn a_cap_already_spent_refuses_to_open() {
        assert_eq!(
            SessionRun::open(Core::with(board(), 60), "linkedin", 60).unwrap_err(),
            RunError::CapReached {
                source_id: "linkedin".into(),
                cap: 60
            }
        );
        assert!(
            SessionRun::open(Core::with(board(), 0), "linkedin", 0).is_err(),
            "no budget, no run"
        );
    }

    #[test]
    fn a_run_never_loads_more_than_the_daily_cap() {
        let core = Core::with(board(), 1);
        let mut run = SessionRun::open(core.clone(), "linkedin", 3).unwrap();
        assert_eq!(run.remaining(), 2);
        run.before_load("https://www.linkedin.com/jobs/search/", 0)
            .unwrap();
        run.before_load("https://www.linkedin.com/jobs/view/1/", 1)
            .unwrap();
        assert_eq!(
            run.before_load("https://www.linkedin.com/jobs/view/2/", 2)
                .unwrap_err(),
            RunError::CapReached {
                source_id: "linkedin".into(),
                cap: 3
            }
        );
        assert_eq!(run.loads(), 2, "a refused load spends nothing");
    }

    #[test]
    fn two_overlapping_runs_share_one_daily_budget() {
        // R-a: both runs open against the same counter; the core spends every
        // page, so together they still stop at the cap.
        let core = Core::with(board(), 0);
        let mut first = SessionRun::open(core.clone(), "linkedin", 3).unwrap();
        let mut second = SessionRun::open(core.clone(), "linkedin", 3).unwrap();
        first
            .before_load("https://www.linkedin.com/jobs/search/", 0)
            .unwrap();
        second
            .before_load("https://www.linkedin.com/jobs/search/", 1)
            .unwrap();
        first
            .before_load("https://www.linkedin.com/jobs/view/1/", 2)
            .unwrap();
        assert_eq!(
            second
                .before_load("https://www.linkedin.com/jobs/view/2/", 3)
                .unwrap_err(),
            RunError::CapReached {
                source_id: "linkedin".into(),
                cap: 3
            }
        );
        assert_eq!(*core.used_today.lock().unwrap(), 3, "one shared budget");
    }

    #[test]
    fn every_load_is_audited_with_the_daily_total() {
        let core = Core::with(board(), 5);
        let mut run = SessionRun::open(core.clone(), "linkedin", 60).unwrap();
        run.before_load("https://www.linkedin.com/jobs/search/?a", 0)
            .unwrap();
        run.before_load("https://www.linkedin.com/jobs/search/?b", 1)
            .unwrap();
        assert_eq!(
            *core.loads.lock().unwrap(),
            vec![("linkedin".to_string(), 6), ("linkedin".to_string(), 7)],
            "the audit carries the running daily total"
        );
    }

    #[test]
    fn an_unavailable_audit_refuses_the_load() {
        let core = Core::with(board(), 0);
        let mut run = SessionRun::open(core.clone(), "linkedin", 60).unwrap();
        *core.fail.lock().unwrap() = true;
        assert!(matches!(
            run.before_load("https://www.linkedin.com/jobs/search/", 0),
            Err(RunError::Control(_))
        ));
        assert_eq!(run.loads(), 0, "nothing loaded unaudited");
        assert_eq!(run.remaining(), 60);
    }

    #[test]
    fn every_load_is_paced_like_a_person() {
        let core = Core::with(board(), 0);
        let mut run = SessionRun::open(core, "linkedin", 10).unwrap();
        let floor = DEFAULT_PACING.min_interval_ms - DEFAULT_PACING.min_interval_ms * 40 / 100;
        let delays: Vec<u64> = (0..10)
            .map(|n| {
                run.before_load("https://www.linkedin.com/jobs/search/", n)
                    .unwrap()
            })
            .collect();
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
