//! Kill Switch (S9.8, FR-86, AD-11, EXPERIENCE §9.9).
//!
//! One flag stops every automatic send. The tray, the app header and the global
//! shortcut (Cmd/Ctrl+Shift+K) all flip it through [`KillSwitchService::set`],
//! which is the only writer:
//!
//! * the flag flips in memory first, so every check point that already exists —
//!   [`crate::apply::outbox::OutboxService`] reads it when an item is queued,
//!   released and again just before the channel submits (S9.6 AC2, S9.7) —
//!   stops on the next call, without waiting for a screen;
//! * the state is persisted in the `setting` table, so a restart comes back
//!   engaged and the header shows the quiet chip until the user turns it off;
//! * every live Outbox item moves to the review queue (`needs_review`, note
//!   `kill_switch`): nothing is lost, the user sends it by hand if they want;
//! * in-flight work is cancelled: the tracked agent runs get their
//!   [`CancelToken`] cancelled (the runtime kills the process tree), and the
//!   one browser session the ATS channel drives is closed — which stops an
//!   in-flight fill before the user's final submit;
//! * the change is audited as an intent + outcome pair (AD-12, FR-89).
//!
//! `engaged()` is the [`KillSwitchPort`]: cheap enough for every pre-submit
//! check, because it is one atomic load.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use boba_domain::apply::outbox::OutboxState;
use boba_domain::audit::AuditOutcome;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::agent::{AgentPort, AgentTask, CancelToken, RunHandle};
use crate::ports::browser::BrowserPort;
use crate::ports::clock::ClockPort;
use crate::ports::kill_switch::KillSwitchPort;
use crate::ports::repo::outbox_hold::{OutboxHoldRepo, OutboxHoldRepoError};
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};

/// The `setting` row holding the flag.
pub const KILL_SWITCH_KEY: &str = "apply.kill_switch";
/// The `setting` row holding when the user engaged it (for the chip).
pub const SINCE_KEY: &str = "apply.kill_switch.since";
/// The audit action every change writes (AD-12).
pub const AUDIT_ACTION: &str = "apply.kill_switch";
/// The note an Outbox item carries when the switch moved it out of the path.
pub const NOTE: &str = "kill_switch";

/// The stored engaged value.
pub const ENGAGED: &str = "engaged";
/// The stored off value.
pub const OFF: &str = "off";

/// How many Outbox rows one engage pass reads (the same panel cap as S9.7).
pub const LIST_LIMIT: usize = 100;

/// How many run handles the registry remembers. A run is harmless to cancel
/// twice, so the list is a rolling window rather than a map with removal: it
/// holds every run a switch could still stop, and never grows with the session.
pub const MAX_TRACKED_RUNS: usize = 64;

/// Whether the switch is on, and since when (AD-1: never an undefined state).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct KillSwitchView {
    pub engaged: bool,
    /// RFC 3339, when the user engaged it; `None` while it is off.
    pub since: Option<String>,
}

/// What one change did, for the command result and the audit detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Engagement {
    pub view: KillSwitchView,
    /// Outbox items moved to the review queue by this call (`u32`: specta does
    /// not export machine-sized integers).
    pub parked: u32,
    /// Run handles asked to stop by this call. A run that already finished is
    /// counted too: its token no longer reaches a process.
    pub cancelled_runs: u32,
    /// Whether a browser session was open and was closed by this call.
    pub browser_closed: bool,
}

/// Why the switch could not be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum KillSwitchError {
    #[error(transparent)]
    System(#[from] SystemRepoError),
    #[error(transparent)]
    Outbox(#[from] OutboxHoldRepoError),
    #[error("the kill switch clock cannot be read")]
    Clock,
}

impl KillSwitchError {
    /// The i18n key the command layer reports (CH7).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::System(_) | Self::Outbox(_) => "errors.apply.store",
            Self::Clock => "errors.apply.kill_switch",
        }
    }
}

/// Tracks the agent runs in flight so one switch stops them (FR-86).
///
/// The composition root wraps the mounted `AgentPort` in [`CancellingAgent`],
/// so every run — the user's own tailoring, the daily search's extraction —
/// registers its handle here without a call site learning about the switch.
#[derive(Debug, Default)]
pub struct RunCancellations {
    runs: Mutex<Vec<(String, CancelToken)>>,
}

impl RunCancellations {
    /// Remember one run's cancel token; the same run id never doubles up.
    pub fn track(&self, run_id: impl Into<String>, token: CancelToken) {
        let run_id = run_id.into();
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        if runs.iter().any(|(seen, _)| seen == &run_id) {
            return;
        }
        runs.push((run_id, token));
        if runs.len() > MAX_TRACKED_RUNS {
            runs.remove(0);
        }
    }

    /// Ask every remembered run to stop; returns how many were still live.
    ///
    /// A run that already finished is cancelled harmlessly: its token no longer
    /// reaches a process.
    pub fn cancel_all(&self) -> usize {
        let runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        let mut live = 0;
        for (_, token) in runs.iter() {
            if !token.is_cancelled() {
                live += 1;
            }
            token.cancel();
        }
        live
    }

    /// How many run ids are remembered.
    pub fn len(&self) -> usize {
        self.runs.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Whether no run was ever tracked (or every entry aged out).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The `AgentPort` the composition root mounts: it delegates every task and
/// remembers the handle, so [`RunCancellations::cancel_all`] can reach it.
pub struct CancellingAgent {
    inner: Arc<dyn AgentPort>,
    runs: Arc<RunCancellations>,
}

impl CancellingAgent {
    pub fn new(inner: Arc<dyn AgentPort>, runs: Arc<RunCancellations>) -> Self {
        Self { inner, runs }
    }
}

impl AgentPort for CancellingAgent {
    fn run(&self, task: AgentTask) -> RunHandle {
        let handle = self.inner.run(task);
        self.runs
            .track(handle.run_id.clone(), handle.cancel.clone());
        handle
    }
}

/// The Kill Switch use case and its [`KillSwitchPort`] adapter.
pub struct KillSwitchService {
    engaged: AtomicBool,
    since: Mutex<Option<String>>,
    system: Arc<dyn SystemRepo>,
    outbox: Arc<dyn OutboxHoldRepo>,
    clock: Arc<dyn ClockPort>,
    /// The one browser session the ATS channel drives (S9.4). A launch without
    /// it still stops every send; it just has no window to close.
    browser: Option<Arc<dyn BrowserPort>>,
    runs: Arc<RunCancellations>,
}

impl KillSwitchService {
    /// Build the service from the mounted ports. The persisted flag is read
    /// once here (AC2: it survives restarts); a store that cannot answer starts
    /// the switch off, the state it has on a fresh install.
    pub fn new(
        system: Arc<dyn SystemRepo>,
        outbox: Arc<dyn OutboxHoldRepo>,
        clock: Arc<dyn ClockPort>,
        browser: Option<Arc<dyn BrowserPort>>,
        runs: Arc<RunCancellations>,
    ) -> Self {
        let stored = persisted(system.as_ref());
        Self {
            engaged: AtomicBool::new(stored.engaged),
            since: Mutex::new(stored.since),
            system,
            outbox,
            clock,
            browser,
            runs,
        }
    }

    /// The state the header chip shows.
    pub fn view(&self) -> KillSwitchView {
        KillSwitchView {
            engaged: self.engaged(),
            since: self.since.lock().unwrap_or_else(|p| p.into_inner()).clone(),
        }
    }

    /// Engage or release the switch. Idempotent: asking for the state it is
    /// already in is a no-op, so the tray, the header and the shortcut never
    /// audited the same change twice.
    ///
    /// Engaging flips the flag **first** (every check point stops from that
    /// instant), then persists it, parks the live Outbox items, cancels the
    /// tracked runs and closes the browser. A persist failure leaves the flag
    /// on: stopping automation is the safe direction, and the next launch
    /// simply starts off.
    pub fn set(&self, engaged: bool) -> Result<Engagement, KillSwitchError> {
        if self.engaged() == engaged {
            return Ok(Engagement {
                view: self.view(),
                parked: 0,
                cancelled_runs: 0,
                browser_closed: false,
            });
        }
        let now = self.clock.now_rfc3339();
        let intent = self.system.audit_append_intent(&NewIntent {
            action: AUDIT_ACTION.to_string(),
            correlation_id: format!("kill_switch_{}_{}", value_of(engaged), stamp(&now)),
            actor: "user".to_string(),
            target_kind: "setting".to_string(),
            target_id: Some(KILL_SWITCH_KEY.to_string()),
            detail_json: serde_json::json!({ "engaged": engaged }).to_string(),
            payload_blake3: None,
            created_at: now.clone(),
        })?;

        self.engaged.store(engaged, Ordering::SeqCst);
        *self.since.lock().unwrap_or_else(|p| p.into_inner()) = engaged.then(|| now.clone());
        if let Err(error) = self.persist(engaged, &now) {
            // The change is live in memory (the flag flips before the write,
            // because stopping is the safe direction), so the audit says it did
            // not survive the restart instead of leaving an open intent.
            let _ = self.system.audit_append_outcome(
                &intent,
                AuditOutcome::Failed,
                &serde_json::json!({ "engaged": engaged, "persisted": false }).to_string(),
                &now,
            );
            return Err(error.into());
        }

        let (parked, cancelled_runs, browser_closed) = if engaged {
            (
                count(self.park_live(&now)),
                count(self.runs.cancel_all()),
                self.browser
                    .as_ref()
                    .is_some_and(|browser| browser.close().is_ok()),
            )
        } else {
            (0, 0, false)
        };

        self.system.audit_append_outcome(
            &intent,
            AuditOutcome::Ok,
            &serde_json::json!({
                "engaged": engaged,
                "parked": parked,
                "cancelled_runs": cancelled_runs,
                "browser_closed": browser_closed,
            })
            .to_string(),
            &now,
        )?;

        Ok(Engagement {
            view: self.view(),
            parked,
            cancelled_runs,
            browser_closed,
        })
    }

    /// Write (or clear) the persisted state.
    fn persist(&self, engaged: bool, now: &str) -> Result<(), SystemRepoError> {
        self.system
            .setting_put(KILL_SWITCH_KEY, value_of(engaged), now)?;
        if engaged {
            self.system.setting_put(SINCE_KEY, now, now)
        } else {
            // A missing stamp is the same as no stamp (the chip reads `None`).
            self.system.setting_delete(SINCE_KEY).map(|_| ())
        }
    }

    /// Move every live item out of the automatic path, best effort: the flag
    /// already stopped the sends, and a row that cannot move stays where it is
    /// for the next pass. Returns how many moved.
    fn park_live(&self, now: &str) -> usize {
        let Ok(items) = self.outbox.outbox_list(LIST_LIMIT) else {
            return 0;
        };
        let mut parked = 0;
        for item in items.iter().filter(|item| item.state.is_live()) {
            if self
                .outbox
                .outbox_set_state(&item.id, OutboxState::NeedsReview, Some(NOTE), now)
                .is_ok()
            {
                parked += 1;
            }
        }
        parked
    }
}

impl KillSwitchPort for KillSwitchService {
    fn engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }
}

/// The state a launch starts with, read from the `setting` rows.
fn persisted(system: &dyn SystemRepo) -> KillSwitchView {
    let engaged = matches!(
        system
            .setting_get(KILL_SWITCH_KEY)
            .ok()
            .flatten()
            .as_deref(),
        Some(ENGAGED)
    );
    let since = if engaged {
        system.setting_get(SINCE_KEY).ok().flatten()
    } else {
        None
    };
    KillSwitchView { engaged, since }
}

/// The stored value of one state.
fn value_of(engaged: bool) -> &'static str {
    if engaged { ENGAGED } else { OFF }
}

/// A count as the wire type (`u32`: specta forbids machine-sized integers).
fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// A correlation id safe for a key: a timestamp's `:` and `+` are not.
fn stamp(now: &str) -> String {
    now.replace([':', '+', '.'], "-")
}

#[cfg(test)]
mod tests;
