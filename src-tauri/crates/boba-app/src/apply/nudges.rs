//! Follow-up nudge notifications (S9.5, FR-95).
//!
//! A card still in Applied has gone quiet after ten days and needs a No
//! Response decision after thirty. The Tracker shows both as an in-app panel
//! (the UI half of the story); this sweep is how Boba says it while the user is
//! elsewhere: one native notification per threshold, per application, ever.
//!
//! Quiet hours (S5.3, FR-107) hold the notice, not consume it: a sweep that
//! runs inside the window sends nothing and marks nothing, so the nudge lands
//! the first time the user is awake.

use std::sync::Arc;

use boba_domain::application::AppStatus;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::applications::OfferSnapshot;
use crate::digest::EventKind;
use crate::digest::settings::{NotificationSettings, SETTINGS_KEY};
use crate::ports::clock::ClockPort;
use crate::ports::notifier::{Notification, NotifierPort};
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::system::{SystemRepo, SystemRepoError};

/// Days in Applied before Boba checks in (FR-95).
pub const CHECK_IN_DAYS: i64 = 10;
/// Days in Applied before Boba suggests No Response (FR-95).
pub const NO_RESPONSE_DAYS: i64 = 30;

/// The `setting` prefix that makes a nudge land once per application and
/// threshold: `nudge.sent.<application_id>.check_in`.
pub const SENT_KEY_PREFIX: &str = "nudge.sent.";

/// Which nudge a card earned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeKind {
    CheckIn,
    NoResponse,
}

impl NudgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NudgeKind::CheckIn => "check_in",
            NudgeKind::NoResponse => "no_response",
        }
    }
}

/// One nudge a sweep sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgedApplication {
    pub application_id: String,
    pub title: String,
    pub company: String,
    pub kind: NudgeKind,
    pub days: u32,
}

/// Why a nudge sweep failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum NudgesError {
    #[error(transparent)]
    Repo(#[from] ApplyRepoError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
}

/// The nudge sweep: reads the board, decides which cards earned a notice, and
/// asks the notifier for one.
pub struct NudgesService {
    apply: Arc<dyn ApplyRepo>,
    system: Arc<dyn SystemRepo>,
    clock: Arc<dyn ClockPort>,
    notifier: Option<Arc<dyn NotifierPort>>,
}

impl NudgesService {
    pub fn new(
        apply: Arc<dyn ApplyRepo>,
        system: Arc<dyn SystemRepo>,
        clock: Arc<dyn ClockPort>,
        notifier: Option<Arc<dyn NotifierPort>>,
    ) -> Self {
        Self {
            apply,
            system,
            clock,
            notifier,
        }
    }

    /// One sweep. `local_hour` is the user's hour of day (0–23), passed by the
    /// caller so quiet hours are testable without the machine's time zone.
    pub fn sweep(&self, local_hour: u8) -> Result<Vec<NudgedApplication>, NudgesError> {
        let now_text = self.clock.now_rfc3339();
        let Ok(now) = OffsetDateTime::parse(&now_text, &Rfc3339) else {
            return Ok(Vec::new());
        };
        let settings =
            NotificationSettings::from_json(self.system.setting_get(SETTINGS_KEY)?.as_deref());
        // S5.3: the user's switches first, then their quiet hours. Both hold
        // the nudge without consuming it, so a sweep after the window still
        // says what the night one kept back.
        if !settings.allows(EventKind::Digest) {
            return Ok(Vec::new());
        }
        if settings.quiet_hours.covers(local_hour) && !settings.quiet_hours.notify_during {
            return Ok(Vec::new());
        }

        let mut sent = Vec::new();
        for application in self.apply.applications()? {
            if application.status != AppStatus::Applied {
                continue;
            }
            let Some(applied_at) = self.applied_at(&application.id)? else {
                continue;
            };
            let days = (now - applied_at).whole_days().max(0);
            let kind = if days >= NO_RESPONSE_DAYS {
                NudgeKind::NoResponse
            } else if days >= CHECK_IN_DAYS {
                NudgeKind::CheckIn
            } else {
                continue;
            };
            let key = format!("{SENT_KEY_PREFIX}{}.{}", application.id, kind.as_str());
            if self.system.setting_get(&key)?.is_some() {
                continue;
            }
            let snapshot: OfferSnapshot =
                serde_json::from_str(&application.offer_snapshot_json).unwrap_or_default();
            if let Some(notifier) = &self.notifier {
                let _ = notifier.notify(Notification::FollowUpNudge {
                    title: snapshot.title.clone(),
                    company: snapshot.company.clone(),
                    days: u32::try_from(days).unwrap_or(0),
                    no_response: kind == NudgeKind::NoResponse,
                });
            }
            // The marker lands after the attempt, never before it: a nudge is
            // said once, and a host without a notification plugin must not
            // make the sweep retry the same line every minute.
            self.system.setting_put(&key, &now_text, &now_text)?;
            sent.push(NudgedApplication {
                application_id: application.id,
                title: snapshot.title,
                company: snapshot.company,
                kind,
                days: u32::try_from(days).unwrap_or(0),
            });
        }
        Ok(sent)
    }

    /// When the application last entered Applied, from its own history.
    fn applied_at(&self, application_id: &str) -> Result<Option<OffsetDateTime>, NudgesError> {
        let mut latest: Option<OffsetDateTime> = None;
        for event in self.apply.status_events(application_id)? {
            if event.to_status != AppStatus::Applied {
                continue;
            }
            let Ok(at) = OffsetDateTime::parse(&event.created_at, &Rfc3339) else {
                continue;
            };
            if latest.is_none_or(|seen| at > seen) {
                latest = Some(at);
            }
        }
        Ok(latest)
    }
}

#[cfg(test)]
mod tests;
