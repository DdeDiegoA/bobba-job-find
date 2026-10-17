//! Dry run and Outbox hold (S9.7, FR-84/FR-85, AD-11).
//!
//! Two guarantees, both enforced here and never on a screen:
//!
//! * **Dry run** (FR-84) — a rule that the user never activated reports its
//!   first matches as `would_send` and sends nothing. Only
//!   [`OutboxService::activate_rule`] turns the reported items into live ones.
//! * **Hold** (FR-85) — once a rule is active, an automatic send waits in the
//!   Outbox for the configured hold (default 30 min, minimum 5; a Session Board
//!   send waits at least an hour, FR-91). During the wait the user can edit or
//!   cancel it. When the hold ends, [`OutboxService::release`] runs `decide()`
//!   **again** (S9.6 AC2), records the send in the cap ledger and only then hands
//!   the item to the channel; a refusal moves it to the review queue with the
//!   reasons the policy gave.
//!
//! The rows live behind [`OutboxHoldRepo`]; the policy and the rules come from
//! [`AutoRulesService`], so this module never re-implements a cap.

use std::sync::Arc;

use boba_domain::apply::outbox::{
    self, DryRunState, DryRunViolation, HoldViolation, NewOutboxItem, OutboxItem, OutboxState,
};
use boba_domain::apply::policy::{PolicyCandidate, RuleChannel};
use serde::{Deserialize, Serialize};
use specta::Type;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::apply::rules::{AutoRulesService, DecisionView, RuleView, RulesError};
use crate::ports::clock::ClockPort;
use crate::ports::kill_switch::KillSwitchPort;
use crate::ports::notifier::{Notification, NotifierPort};
use crate::ports::repo::outbox_hold::{OutboxHoldRepo, OutboxHoldRepoError};
use crate::ports::repo::system::{SystemRepo, SystemRepoError};

/// How many stored items one snapshot returns (the list is a panel, not a log).
pub const LIST_LIMIT: usize = 100;
/// The longest edit this build stores (a body, not a document).
pub const EDIT_MAX_CHARS: usize = 20_000;

/// The hold setting this area owns.
pub const HOLD_MINUTES_KEY: &str = "apply.outbox.hold_minutes";

/// The Outbox's own settings, defaults resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OutboxSettings {
    /// The configured hold, before the per-channel floor.
    pub hold_minutes: u16,
}

impl Default for OutboxSettings {
    fn default() -> Self {
        Self {
            hold_minutes: outbox::DEFAULT_HOLD_MINUTES,
        }
    }
}

/// One rule's dry run, as the screen shows it (FR-84).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DryRunView {
    pub rule_id: String,
    pub rule_name: String,
    pub required: u8,
    pub seen: u8,
    /// Whether the user activated the rule; only then does it send.
    pub active: bool,
    /// Whether the dry run has reported every match it promises.
    pub complete: bool,
}

/// One Outbox item, as the screen and the tray show it (FR-85).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OutboxItemView {
    pub id: String,
    pub application_id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub title: String,
    pub company: String,
    /// `email`, `ats_form`, `sanctioned_api` or `session_board`.
    pub channel: String,
    /// `would_send`, `held`, `releasing`, `sent`, `cancelled` or `needs_review`.
    pub state: String,
    pub hold_minutes: u16,
    pub release_at: String,
    /// Whole minutes until the hold ends; 0 once it has (or for a dry run).
    pub minutes_left: u16,
    pub subject_override: Option<String>,
    pub body_override: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
}

/// Everything the Today Outbox panel and the tray read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OutboxView {
    pub settings: OutboxSettings,
    pub items: Vec<OutboxItemView>,
    pub rules: Vec<DryRunView>,
}

/// What [`OutboxService::enqueue`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Enqueued {
    /// The rule reported it without sending: the dry run (FR-84).
    DryRun { item: OutboxItemView },
    /// The item waits out its hold (FR-85).
    Held { item: OutboxItemView },
    /// The policy kept the decision: nothing was queued (S9.6 AC2).
    NotAuto { decision: DecisionView },
}

/// What [`OutboxService::release`] decided when a hold ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Release {
    /// The policy still says Auto: the item is claimed (`releasing`) and the
    /// caller decides whether the send is certain. [`OutboxService::record_send`]
    /// writes the cap entry only once it is.
    Sending {
        item: Box<OutboxItem>,
        /// The rule whose decision allowed this send (the cap row names it).
        rule_id: String,
    },
    /// The policy no longer says Auto: the item moved to the review queue.
    Review(Box<OutboxItem>),
    /// The previous automatic send to this Source was too recent (FR-91): the
    /// item stays held and the next sweep tries again.
    Deferred {
        item: Box<OutboxItem>,
        wait_minutes: u32,
    },
}

/// What the pre-submit check allows (S9.6 AC2: enforcement before submit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    /// The policy still says Auto and the Kill Switch is off: send it.
    Go,
    /// The send stopped; the item already moved to the review queue.
    Stop,
}

/// How one release ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// The channel confirmed the send (FR-88's evidence lives in the attempt).
    Sent,
    /// The send did not happen or was not proven: the user reviews it by hand.
    NeedsReview,
}

/// Why an Outbox use case failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum OutboxError {
    #[error(transparent)]
    Repo(#[from] OutboxHoldRepoError),
    #[error(transparent)]
    Rules(#[from] RulesError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    /// A hold outside FR-85's bounds.
    #[error("invalid outbox hold: {0}")]
    Hold(#[from] HoldViolation),
    /// A dry-run length outside FR-84's bounds.
    #[error("invalid dry run: {0}")]
    DryRun(#[from] DryRunViolation),
    /// The item does not exist.
    #[error("unknown outbox item '{0}'")]
    UnknownItem(String),
    /// The item already left the hold; it cannot be edited or cancelled.
    #[error("outbox item '{0}' is no longer waiting")]
    NotWaiting(String),
    /// The application already has an open item.
    #[error("application '{0}' is already queued")]
    AlreadyQueued(String),
    /// The user engaged the Kill Switch: nothing automatic leaves (FR-86).
    #[error("the kill switch is engaged")]
    KillSwitch,
    /// An edit this build refuses.
    #[error("invalid outbox edit")]
    Edit,
    /// An Auto decision that names a rule which is not stored.
    #[error("unknown auto rule '{0}'")]
    UnknownRule(String),
    /// The item names a channel this build does not know.
    #[error("unknown apply channel '{0}'")]
    UnknownChannel(String),
    /// A stored timestamp cannot be read.
    #[error("the outbox clock cannot be read")]
    Clock,
}

impl OutboxError {
    /// The i18n key the command layer reports (CH7).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) | Self::System(_) => "errors.apply.store",
            Self::Rules(error) => error.message_key(),
            Self::Hold(_) => "errors.apply.outbox_hold",
            Self::DryRun(_) => "errors.apply.outbox_dry_run",
            Self::UnknownItem(_) => "errors.apply.outbox_not_found",
            Self::NotWaiting(_) => "errors.apply.outbox_not_live",
            Self::AlreadyQueued(_) => "errors.apply.outbox_queued",
            Self::KillSwitch => "errors.apply.outbox_kill_switch",
            Self::Edit => "errors.apply.outbox_edit",
            Self::UnknownRule(_) => "errors.apply.unknown_rule",
            Self::UnknownChannel(_) => "errors.apply.rule_invalid",
            Self::Clock => "errors.apply.outbox_clock",
        }
    }
}

/// The Outbox use cases. The ports arrive as `Arc<dyn …>`, the shape every use
/// case in this crate uses, so one instance can sit behind a command.
pub struct OutboxService {
    items: Arc<dyn OutboxHoldRepo>,
    rules: Arc<AutoRulesService>,
    system: Arc<dyn SystemRepo>,
    clock: Arc<dyn ClockPort>,
    /// The native notification FR-85 asks for when an item enters the hold. A
    /// host without notifications keeps working: the record is the Outbox.
    notifier: Option<Arc<dyn NotifierPort>>,
    /// The Kill Switch (S9.8): the check point that stops every automatic send.
    /// Unmounted means not engaged, so the feature ships without S9.8.
    kill_switch: Option<Arc<dyn KillSwitchPort>>,
}

impl OutboxService {
    pub fn new(
        items: Arc<dyn OutboxHoldRepo>,
        rules: Arc<AutoRulesService>,
        system: Arc<dyn SystemRepo>,
        clock: Arc<dyn ClockPort>,
        notifier: Option<Arc<dyn NotifierPort>>,
        kill_switch: Option<Arc<dyn KillSwitchPort>>,
    ) -> Self {
        Self {
            items,
            rules,
            system,
            clock,
            notifier,
            kill_switch,
        }
    }

    /// The Outbox settings, defaults resolved (AD-1: never an undefined hold).
    pub fn settings(&self) -> Result<OutboxSettings, OutboxError> {
        let minutes = self
            .system
            .setting_get(HOLD_MINUTES_KEY)?
            .and_then(|raw| raw.trim().parse::<u16>().ok())
            .filter(|minutes| outbox::validate_hold_minutes(*minutes).is_ok())
            .unwrap_or(outbox::DEFAULT_HOLD_MINUTES);
        Ok(OutboxSettings {
            hold_minutes: minutes,
        })
    }

    /// Set the configured hold; core validates it, never the UI (FR-85).
    pub fn set_hold(&self, minutes: u16) -> Result<OutboxSettings, OutboxError> {
        outbox::validate_hold_minutes(minutes)?;
        let now = self.clock.now_rfc3339();
        self.system
            .setting_put(HOLD_MINUTES_KEY, &minutes.to_string(), &now)?;
        Ok(OutboxSettings {
            hold_minutes: minutes,
        })
    }

    /// What Today and the tray show: the items and every rule's dry run.
    pub fn snapshot(&self) -> Result<OutboxView, OutboxError> {
        let settings = self.settings()?;
        let rules = self.rules.snapshot()?.rules;
        let now = self.now()?;
        let stored = self.items.outbox_list(LIST_LIMIT)?;
        let dry_rows = self.items.dry_run_list()?;

        let items = stored
            .iter()
            .map(|item| self.view(item, &rules, &now))
            .collect();
        let rules_view = rules
            .iter()
            .map(|rule| {
                let state = dry_rows
                    .iter()
                    .find(|row| row.rule_id == rule.id)
                    .cloned()
                    .unwrap_or_else(|| DryRunState::new(&rule.id));
                dry_run_view(&state, &rule.name)
            })
            .collect();
        Ok(OutboxView {
            settings,
            items,
            rules: rules_view,
        })
    }

    /// Queue one automatic send (FR-84/FR-85). The policy runs first: a
    /// `Review`/`Block` decision queues nothing. An Auto decision queues a
    /// `would_send` item while the rule's dry run runs, and a `held` item after
    /// the user activated it.
    ///
    /// `spacing_sample` is the caller's random sample for the S9.6 gap, passed
    /// through so this use case stays deterministic under test.
    pub fn enqueue(
        &self,
        application_id: &str,
        candidate: &PolicyCandidate,
        title: &str,
        company: &str,
        spacing_sample: u64,
    ) -> Result<Enqueued, OutboxError> {
        // FR-86: with the Kill Switch on, nothing new enters the automatic path.
        if self
            .kill_switch
            .as_ref()
            .is_some_and(|switch| switch.engaged())
        {
            return Err(OutboxError::KillSwitch);
        }
        if self.items.outbox_live_for(application_id)?.is_some() {
            return Err(OutboxError::AlreadyQueued(application_id.to_string()));
        }
        let decision = self.rules.decide(candidate, spacing_sample)?;
        let Some(rule_id) = decision.rule_id.clone() else {
            return Ok(Enqueued::NotAuto { decision });
        };
        let rules = self.rules.snapshot()?.rules;
        if !rules.iter().any(|rule| rule.id == rule_id) {
            return Err(OutboxError::UnknownRule(rule_id));
        }
        let mut dry = self
            .items
            .dry_run_get(&rule_id)?
            .unwrap_or_else(|| DryRunState::new(&rule_id));

        let now = self.now()?;
        let (state, hold_minutes, release_at) = if dry.is_active() {
            let hold = outbox::effective_hold_minutes(
                candidate.channel.as_str(),
                self.settings()?.hold_minutes,
            );
            (
                OutboxState::Held,
                hold,
                format_time(now + time::Duration::minutes(i64::from(hold))),
            )
        } else {
            (OutboxState::WouldSend, 0, format_time(now))
        };

        let item = self.items.outbox_insert(
            &NewOutboxItem {
                application_id: application_id.to_string(),
                rule_id: rule_id.clone(),
                source_id: candidate.source_id.clone(),
                channel: candidate.channel.as_str().to_string(),
                company_key: candidate.company_key.clone(),
                title: title.to_string(),
                company: company.to_string(),
                state,
                hold_minutes,
                release_at,
            },
            &format_time(now),
        )?;

        if !dry.is_active() {
            dry.seen = dry.seen.saturating_add(1).min(dry.required);
            self.items.dry_run_upsert(&dry, &format_time(now))?;
        }

        // FR-85 notifies the user when a send enters the hold. A dry run's
        // "would send" is a report on a screen, not an interruption.
        if state == OutboxState::Held
            && let Some(notifier) = &self.notifier
        {
            let _ = notifier.notify(Notification::OutboxHold);
        }

        let view = self.view(&item, &rules, &now);
        Ok(match state {
            OutboxState::Held => Enqueued::Held { item: view },
            _ => Enqueued::DryRun { item: view },
        })
    }

    /// Cancel one item during its wait (FR-85). Returns the item as stored.
    pub fn cancel(&self, id: &str) -> Result<OutboxItemView, OutboxError> {
        let item = self.waiting(id)?;
        let now = self.now()?;
        let stored = self.items.outbox_set_state(
            &item.id,
            OutboxState::Cancelled,
            Some("user"),
            &format_time(now),
        )?;
        self.view_of(stored, &now)
    }

    /// Edit the subject or the body during the hold (FR-85). The send uses the
    /// edit; an empty string clears one, and both are refused past the cap.
    pub fn edit(
        &self,
        id: &str,
        subject: Option<String>,
        body: Option<String>,
    ) -> Result<OutboxItemView, OutboxError> {
        let item = self.waiting(id)?;
        let subject = clean(subject)?;
        let body = clean(body)?;
        if subject.is_none() && body.is_none() {
            // Clearing both is allowed; it returns the item to its draft.
            let now = self.now()?;
            let stored =
                self.items
                    .outbox_set_overrides(&item.id, None, None, &format_time(now))?;
            return self.view_of(stored, &now);
        }
        let now = self.now()?;
        let stored = self.items.outbox_set_overrides(
            &item.id,
            subject.as_deref(),
            body.as_deref(),
            &format_time(now),
        )?;
        self.view_of(stored, &now)
    }

    /// Move a live item out of the automatic path, saying why (a channel this
    /// build cannot send automatically, or a mailbox that is gone). Nothing is
    /// sent and no cap entry is written; the user reviews it by hand.
    pub fn park(&self, id: &str, note: &str) -> Result<OutboxItemView, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        if !item.state.is_live() {
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        let now = self.now()?;
        let stored = self.items.outbox_set_state(
            &item.id,
            OutboxState::NeedsReview,
            Some(note),
            &format_time(now),
        )?;
        self.view_of(stored, &now)
    }

    /// One item, in the shape the screen shows it (the release sweep's answer).
    pub fn item_view(&self, id: &str) -> Result<OutboxItemView, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        let now = self.now()?;
        self.view_of(item, &now)
    }

    /// Finish a rule's dry run and turn it into a live rule (FR-84 AC1). The
    /// items it reported move to the review queue; nothing is sent for them.
    pub fn activate_rule(&self, rule_id: &str) -> Result<DryRunView, OutboxError> {
        let rule = self
            .rules
            .snapshot()?
            .rules
            .into_iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| OutboxError::UnknownRule(rule_id.to_string()))?;
        let now = self.now()?;
        let mut state = self
            .items
            .dry_run_get(rule_id)?
            .unwrap_or_else(|| DryRunState::new(rule_id));
        state.activated_at = Some(format_time(now));
        self.items.dry_run_upsert(&state, &format_time(now))?;

        for item in self.items.outbox_list(LIST_LIMIT)? {
            if item.rule_id == rule_id && item.state == OutboxState::WouldSend {
                self.items.outbox_set_state(
                    &item.id,
                    OutboxState::NeedsReview,
                    Some("dry_run_ended"),
                    &format_time(now),
                )?;
            }
        }
        Ok(dry_run_view(&state, &rule.name))
    }

    /// Set how many matches one rule's dry run reports (FR-84, minimum 1).
    pub fn set_dry_run(&self, rule_id: &str, matches: u8) -> Result<DryRunView, OutboxError> {
        outbox::validate_dry_run_matches(matches)?;
        let rule = self
            .rules
            .snapshot()?
            .rules
            .into_iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| OutboxError::UnknownRule(rule_id.to_string()))?;
        let now = self.now()?;
        let mut state = self
            .items
            .dry_run_get(rule_id)?
            .unwrap_or_else(|| DryRunState::new(rule_id));
        state.required = matches;
        self.items.dry_run_upsert(&state, &format_time(now))?;
        Ok(dry_run_view(&state, &rule.name))
    }

    /// The held items whose time has come, oldest first (the release sweep).
    pub fn due(&self) -> Result<Vec<OutboxItem>, OutboxError> {
        let now = self.now()?;
        Ok(self.items.outbox_due(&format_time(now))?)
    }

    /// Release one item whose hold ended (FR-85, S9.6 AC2).
    ///
    /// The policy runs **again** against the facts the caller just read: a
    /// rule turned off, a cap reached meanwhile or a VETO cancels the send and
    /// moves the item to the review queue with the reasons. Only an Auto answer
    /// records the cap entry and hands the item to the channel.
    pub fn release(
        &self,
        id: &str,
        candidate: &PolicyCandidate,
        spacing_sample: u64,
    ) -> Result<Release, OutboxError> {
        let item = self.waiting(id)?;
        if item.state != OutboxState::Held {
            // A dry run's "would send" is a report, never a send.
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        // FR-86: the Kill Switch stops every automatic send, checked here and
        // again just before the channel submits.
        if self
            .kill_switch
            .as_ref()
            .is_some_and(|switch| switch.engaged())
        {
            return Err(OutboxError::KillSwitch);
        }
        let decision = self.rules.decide(candidate, spacing_sample)?;
        let now = self.now()?;
        let Some(rule_id) = decision.rule_id.clone() else {
            // A timing reason resolves on its own, so the item keeps its place
            // in the hold instead of bothering the user. Anything else (a cap,
            // a VETO, a disabled rule) means "not automatically", so it goes to
            // the review queue.
            if let Some(wait_minutes) = retry_after_minutes(&decision) {
                return Ok(Release::Deferred {
                    item: Box::new(item),
                    wait_minutes,
                });
            }
            let note = review_note(&decision);
            let stored = self.items.outbox_set_state(
                &item.id,
                OutboxState::NeedsReview,
                Some(&note),
                &format_time(now),
            )?;
            return Ok(Release::Review(Box::new(stored)));
        };
        // FR-91's anti-ban spacing: one Source never takes two automatic sends
        // back to back. The item stays held for the next sweep.
        let gap = outbox::min_source_gap_minutes(&item.channel);
        if let Some(last) = self.rules.last_auto_for_source(&item.source_id)? {
            let last = OffsetDateTime::parse(&last, &Rfc3339).map_err(|_| OutboxError::Clock)?;
            let elapsed = (now - last).whole_minutes().max(0);
            let elapsed = u32::try_from(elapsed).unwrap_or(u32::MAX);
            if elapsed < gap {
                return Ok(Release::Deferred {
                    item: Box::new(item),
                    wait_minutes: gap - elapsed,
                });
            }
        }
        // The claim is one conditional write, so two sweeps (the tick and the
        // screen) can never release the same item twice.
        if !self.items.outbox_claim(&item.id, &format_time(now))? {
            return Err(OutboxError::NotWaiting(item.id));
        }
        // No cap entry yet: the caller still has to prove a mailbox exists and
        // pass the pre-submit check. `record_send` writes it only then, so a
        // send that never leaves does not consume a cap slot (review B-3).
        let stored = self
            .items
            .outbox_get(&item.id)?
            .ok_or_else(|| OutboxError::UnknownItem(item.id.clone()))?;
        Ok(Release::Sending {
            item: Box::new(stored),
            rule_id,
        })
    }

    /// Write the cap entry for a send that is certain (S9.6 AC2, review B-3).
    ///
    /// It runs **after** the mailbox check and the pre-submit check, and
    /// **before** the channel starts, so the entry counts a send that really is
    /// on its way. A ledger write that fails gives the item back to the hold
    /// instead of stranding it in `releasing`.
    pub fn record_send(&self, id: &str, rule_id: &str) -> Result<OutboxItem, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        if item.state != OutboxState::Releasing {
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        let channel = RuleChannel::parse(&item.channel)
            .ok_or_else(|| OutboxError::UnknownChannel(item.channel.clone()))?;
        let now = format_time(self.now()?);
        if let Err(error) = self.rules.record_auto_send(
            rule_id,
            &item.application_id,
            &item.source_id,
            channel,
            &item.company_key,
        ) {
            let _ = self
                .items
                .outbox_set_state(&item.id, OutboxState::Held, None, &now);
            return Err(OutboxError::Rules(error));
        }
        self.items
            .outbox_get(&item.id)?
            .ok_or_else(|| OutboxError::UnknownItem(item.id.clone()))
    }

    /// The check that runs **just before the channel submits** (S9.6 AC2): the
    /// Kill Switch and `decide()` run again against the facts the caller just
    /// read. A stop parks the item for the user, so nothing leaves afterwards.
    pub fn confirm_before_submit(
        &self,
        id: &str,
        candidate: &PolicyCandidate,
        spacing_sample: u64,
    ) -> Result<Submit, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        if item.state != OutboxState::Releasing {
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        if self
            .kill_switch
            .as_ref()
            .is_some_and(|switch| switch.engaged())
        {
            self.park(&item.id, "kill_switch")?;
            return Ok(Submit::Stop);
        }
        let decision = self.rules.decide(candidate, spacing_sample)?;
        if decision.rule_id.is_none() {
            self.park(&item.id, &review_note(&decision))?;
            return Ok(Submit::Stop);
        }
        Ok(Submit::Go)
    }

    /// Record how one release ended (FR-85). A `needs_review` outcome keeps the
    /// reason for the Today panel; the attempt row keeps the send evidence.
    pub fn finish(&self, id: &str, finish: Finish) -> Result<OutboxItemView, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        if item.state != OutboxState::Releasing {
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        let now = self.now()?;
        let (state, note) = match finish {
            Finish::Sent => (OutboxState::Sent, None),
            Finish::NeedsReview => (OutboxState::NeedsReview, Some("send_failed")),
        };
        let stored = self
            .items
            .outbox_set_state(&item.id, state, note, &format_time(now))?;
        self.view_of(stored, &now)
    }

    /// Close the items a crash left mid-release (called once at startup). A
    /// `releasing` row at launch means the process died between the ledger entry
    /// and the result: the item goes to the review queue, never to a silent
    /// second send. Returns how many items were reconciled.
    pub fn reconcile(&self) -> Result<usize, OutboxError> {
        let now = format_time(self.now()?);
        Ok(reconcile_stuck(self.items.as_ref(), &now)?)
    }

    // ---- internals ---------------------------------------------------------

    fn now(&self) -> Result<OffsetDateTime, OutboxError> {
        OffsetDateTime::parse(&self.clock.now_rfc3339(), &Rfc3339).map_err(|_| OutboxError::Clock)
    }

    /// One item that is still waiting; anything else is not editable.
    fn waiting(&self, id: &str) -> Result<OutboxItem, OutboxError> {
        let item = self
            .items
            .outbox_get(id)?
            .ok_or_else(|| OutboxError::UnknownItem(id.to_string()))?;
        if !matches!(item.state, OutboxState::Held | OutboxState::WouldSend) {
            return Err(OutboxError::NotWaiting(id.to_string()));
        }
        Ok(item)
    }

    fn view_of(
        &self,
        item: OutboxItem,
        now: &OffsetDateTime,
    ) -> Result<OutboxItemView, OutboxError> {
        let rules = self.rules.snapshot()?.rules;
        Ok(self.view(&item, &rules, now))
    }

    fn view(&self, item: &OutboxItem, rules: &[RuleView], now: &OffsetDateTime) -> OutboxItemView {
        let rule_name = rules
            .iter()
            .find(|rule| rule.id == item.rule_id)
            .map(|rule| rule.name.clone())
            .unwrap_or_else(|| item.rule_id.clone());
        OutboxItemView {
            id: item.id.clone(),
            application_id: item.application_id.clone(),
            rule_id: item.rule_id.clone(),
            rule_name,
            title: item.title.clone(),
            company: item.company.clone(),
            channel: item.channel.clone(),
            state: item.state.as_str().to_string(),
            hold_minutes: item.hold_minutes,
            release_at: item.release_at.clone(),
            minutes_left: minutes_left(&item.release_at, now),
            subject_override: item.subject_override.clone(),
            body_override: item.body_override.clone(),
            note: item.note.clone(),
            created_at: item.created_at.clone(),
        }
    }
}

/// Close the items a crash left mid-release (S9.7; the startup path needs only
/// the repo and the clock, so the composition root can call it before any
/// service is built). Returns how many items were reconciled.
pub fn reconcile_stuck(
    items: &dyn OutboxHoldRepo,
    now: &str,
) -> Result<usize, OutboxHoldRepoError> {
    let stuck = items.outbox_releasing()?;
    for item in &stuck {
        items.outbox_set_state(&item.id, OutboxState::NeedsReview, Some("interrupted"), now)?;
    }
    Ok(stuck.len())
}

/// The wait a timing-only refusal asks for, `None` when the refusal is not a
/// timing one (S9.7, FR-85). Spacing carries its wait; active hours only say
/// "not now", so the next sweep (a minute later) re-asks.
fn retry_after_minutes(decision: &DecisionView) -> Option<u32> {
    let mut wait = 1;
    if decision.reasons.is_empty() {
        return None;
    }
    for reason in &decision.reasons {
        match reason.code.as_str() {
            "spacing_not_elapsed" => {
                wait = wait.max(
                    reason
                        .params
                        .get("wait_minutes")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(1),
                );
            }
            "outside_active_hours" => {}
            _ => return None,
        }
    }
    Some(wait)
}

/// The reason list one refused release carries, as the item's note.
fn review_note(decision: &DecisionView) -> String {
    let codes: Vec<&str> = decision
        .reasons
        .iter()
        .map(|reason| reason.code.as_str())
        .collect();
    format!("review:{}", codes.join(","))
}

/// The minutes until `release_at`; 0 once it has come, and a value this build
/// cannot read also reads 0 (the hold has no meaning it can prove).
fn minutes_left(release_at: &str, now: &OffsetDateTime) -> u16 {
    let Ok(at) = OffsetDateTime::parse(release_at, &Rfc3339) else {
        return 0;
    };
    let minutes = (at - *now).whole_minutes().max(0);
    u16::try_from(minutes).unwrap_or(u16::MAX)
}

/// The dry-run view for one rule.
fn dry_run_view(state: &DryRunState, rule_name: &str) -> DryRunView {
    DryRunView {
        rule_id: state.rule_id.clone(),
        rule_name: rule_name.to_string(),
        required: state.required,
        seen: state.seen,
        active: state.is_active(),
        complete: state.is_complete(),
    }
}

/// One edit value: trimmed, empty becomes `None`, and the cap is enforced here.
fn clean(value: Option<String>) -> Result<Option<String>, OutboxError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.chars().count() > EDIT_MAX_CHARS {
        return Err(OutboxError::Edit);
    }
    Ok(Some(value.to_string()))
}

fn format_time(value: OffsetDateTime) -> String {
    value
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use boba_domain::apply::policy::{AutoAttemptEntry, AutoRule};
    use boba_domain::scoring::gates::{Confidence, Verdict as GateVerdict};
    use boba_domain::scoring::rank::Verdict;

    use crate::ports::notifier::NotifierError;
    use crate::ports::repo::auto_rules::{AutoRulesRepo, AutoRulesRepoError};
    use crate::system::{FixedClock, testing::FakeSystemRepo};

    const NOW: &str = "2026-10-17T09:00:00Z";

    // ---- fakes ------------------------------------------------------------

    #[derive(Default)]
    struct FakeOutbox {
        items: Mutex<Vec<OutboxItem>>,
        dry: Mutex<Vec<DryRunState>>,
        next: Mutex<usize>,
    }

    impl FakeOutbox {
        fn items(&self) -> Vec<OutboxItem> {
            self.items.lock().unwrap().clone()
        }

        fn count(&self) -> usize {
            self.items.lock().unwrap().len()
        }

        fn dry(&self, rule_id: &str) -> Option<DryRunState> {
            self.dry
                .lock()
                .unwrap()
                .iter()
                .find(|state| state.rule_id == rule_id)
                .cloned()
        }

        fn seed_dry(&self, state: DryRunState) {
            self.dry.lock().unwrap().push(state);
        }

        fn seed_item(&self, item: OutboxItem) {
            self.items.lock().unwrap().push(item);
        }
    }

    fn live(state: OutboxState) -> bool {
        state.is_live()
    }

    impl OutboxHoldRepo for FakeOutbox {
        fn outbox_insert(
            &self,
            item: &NewOutboxItem,
            now: &str,
        ) -> Result<OutboxItem, OutboxHoldRepoError> {
            let mut next = self.next.lock().unwrap();
            *next += 1;
            let stored = OutboxItem {
                id: format!("out_{}", *next),
                application_id: item.application_id.clone(),
                rule_id: item.rule_id.clone(),
                source_id: item.source_id.clone(),
                channel: item.channel.clone(),
                company_key: item.company_key.clone(),
                title: item.title.clone(),
                company: item.company.clone(),
                state: item.state,
                hold_minutes: item.hold_minutes,
                release_at: item.release_at.clone(),
                subject_override: None,
                body_override: None,
                note: None,
                created_at: now.to_string(),
                updated_at: now.to_string(),
            };
            self.items.lock().unwrap().push(stored.clone());
            Ok(stored)
        }

        fn outbox_get(&self, id: &str) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
            Ok(self
                .items
                .lock()
                .unwrap()
                .iter()
                .find(|item| item.id == id)
                .cloned())
        }

        fn outbox_live_for(
            &self,
            application_id: &str,
        ) -> Result<Option<OutboxItem>, OutboxHoldRepoError> {
            Ok(self
                .items
                .lock()
                .unwrap()
                .iter()
                .find(|item| item.application_id == application_id && live(item.state))
                .cloned())
        }

        fn outbox_list(&self, limit: usize) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
            let mut items = self.items();
            items.reverse();
            items.truncate(limit);
            Ok(items)
        }

        fn outbox_due(&self, now: &str) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
            Ok(self
                .items()
                .into_iter()
                .filter(|item| item.state == OutboxState::Held && item.release_at.as_str() <= now)
                .collect())
        }

        fn outbox_releasing(&self) -> Result<Vec<OutboxItem>, OutboxHoldRepoError> {
            Ok(self
                .items()
                .into_iter()
                .filter(|item| item.state == OutboxState::Releasing)
                .collect())
        }

        fn outbox_claim(&self, id: &str, now: &str) -> Result<bool, OutboxHoldRepoError> {
            let mut items = self.items.lock().unwrap();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| OutboxHoldRepoError::new(format!("unknown outbox item {id}")))?;
            if item.state != OutboxState::Held {
                return Ok(false);
            }
            item.state = OutboxState::Releasing;
            item.updated_at = now.to_string();
            Ok(true)
        }

        fn outbox_set_state(
            &self,
            id: &str,
            state: OutboxState,
            note: Option<&str>,
            now: &str,
        ) -> Result<OutboxItem, OutboxHoldRepoError> {
            let mut items = self.items.lock().unwrap();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| OutboxHoldRepoError::new(format!("unknown outbox item {id}")))?;
            item.state = state;
            if note.is_some() {
                item.note = note.map(str::to_string);
            }
            item.updated_at = now.to_string();
            Ok(item.clone())
        }

        fn outbox_set_overrides(
            &self,
            id: &str,
            subject: Option<&str>,
            body: Option<&str>,
            now: &str,
        ) -> Result<OutboxItem, OutboxHoldRepoError> {
            let mut items = self.items.lock().unwrap();
            let item = items
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| OutboxHoldRepoError::new(format!("unknown outbox item {id}")))?;
            item.subject_override = subject.map(str::to_string);
            item.body_override = body.map(str::to_string);
            item.updated_at = now.to_string();
            Ok(item.clone())
        }

        fn dry_run_get(&self, rule_id: &str) -> Result<Option<DryRunState>, OutboxHoldRepoError> {
            Ok(self.dry(rule_id))
        }

        fn dry_run_list(&self) -> Result<Vec<DryRunState>, OutboxHoldRepoError> {
            Ok(self.dry.lock().unwrap().clone())
        }

        fn dry_run_upsert(
            &self,
            state: &DryRunState,
            _now: &str,
        ) -> Result<(), OutboxHoldRepoError> {
            let mut dry = self.dry.lock().unwrap();
            match dry.iter_mut().find(|row| row.rule_id == state.rule_id) {
                Some(row) => *row = state.clone(),
                None => dry.push(state.clone()),
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeAutoRules {
        rules: Mutex<Vec<AutoRule>>,
        ledger: Mutex<Vec<AutoAttemptEntry>>,
    }

    impl FakeAutoRules {
        fn with_rules(rules: Vec<AutoRule>) -> Self {
            Self {
                rules: Mutex::new(rules),
                ledger: Mutex::new(Vec::new()),
            }
        }

        fn ledger(&self) -> Vec<AutoAttemptEntry> {
            self.ledger.lock().unwrap().clone()
        }

        fn seed_ledger(&self, entry: AutoAttemptEntry) {
            self.ledger.lock().unwrap().push(entry);
        }
    }

    /// A Kill Switch the test toggles (S9.8's port, stubbed here).
    #[derive(Default)]
    struct FakeKillSwitch {
        engaged: std::sync::atomic::AtomicBool,
    }

    impl FakeKillSwitch {
        fn engage(&self) {
            self.engaged
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl KillSwitchPort for FakeKillSwitch {
        fn engaged(&self) -> bool {
            self.engaged.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl AutoRulesRepo for FakeAutoRules {
        fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
            Ok(self.rules.lock().unwrap().clone())
        }

        fn rule_upsert(&self, rule: &AutoRule) -> Result<(), AutoRulesRepoError> {
            let mut rules = self.rules.lock().unwrap();
            match rules.iter_mut().find(|stored| stored.id == rule.id) {
                Some(stored) => *stored = rule.clone(),
                None => rules.push(rule.clone()),
            }
            Ok(())
        }

        fn rule_delete(&self, id: &str) -> Result<bool, AutoRulesRepoError> {
            let before = self.rules.lock().unwrap().len();
            self.rules.lock().unwrap().retain(|rule| rule.id != id);
            Ok(self.rules.lock().unwrap().len() != before)
        }

        fn ledger_insert(&self, entry: &AutoAttemptEntry) -> Result<(), AutoRulesRepoError> {
            self.ledger.lock().unwrap().push(entry.clone());
            Ok(())
        }

        fn ledger_global_since(&self, since: &str) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.created_at.as_str() >= since)
                .count() as u32)
        }

        fn ledger_rule_since(&self, rule_id: &str, since: &str) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.rule_id == rule_id && entry.created_at.as_str() >= since)
                .count() as u32)
        }

        fn ledger_company_since(
            &self,
            company_key: &str,
            since: &str,
        ) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| {
                    entry.company_key == company_key && entry.created_at.as_str() >= since
                })
                .count() as u32)
        }

        fn ledger_last_at(&self) -> Result<Option<String>, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .map(|entry| entry.created_at.clone())
                .max())
        }

        fn ledger_last_at_for_source(
            &self,
            source_id: &str,
        ) -> Result<Option<String>, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.source_id == source_id)
                .map(|entry| entry.created_at.clone())
                .max())
        }

        fn ledger_source_since(
            &self,
            source_id: &str,
            channel: &str,
            since: &str,
        ) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| {
                    entry.source_id == source_id
                        && entry.channel.as_str() == channel
                        && entry.created_at.as_str() >= since
                })
                .count() as u32)
        }
    }

    #[derive(Default)]
    struct CountingNotifier {
        sent: Mutex<Vec<Notification>>,
    }

    impl NotifierPort for CountingNotifier {
        fn notify(&self, notification: Notification) -> Result<(), NotifierError> {
            self.sent.lock().unwrap().push(notification);
            Ok(())
        }
    }

    // ---- fixtures ---------------------------------------------------------

    fn rule(id: &str) -> AutoRule {
        AutoRule::defaults(id, "Email × remote", NOW)
    }

    fn candidate() -> PolicyCandidate {
        PolicyCandidate {
            source_id: "remotive".into(),
            channel: RuleChannel::Email,
            company_key: "acme".into(),
            rank: Some(85.0),
            fit: Some(82.0),
            verdict: Verdict::Good,
            confidence: Confidence::Medium,
            gates: GateVerdict::Pass,
            fraud_badge: false,
            documents_verified: true,
            language_variant_confirmed: true,
            duplicate: false,
            agent_used: false,
            cli_tool_lockdown: true,
            source_auto_opt_in: true,
            session_auto: None,
            local_minute: 10 * 60,
            utc_offset_minutes: 0,
        }
    }

    /// The Auto settings the rules service reads: on, confirmed, defaults.
    fn system() -> FakeSystemRepo {
        let mut system = FakeSystemRepo::default();
        system.seed_setting("apply.auto.enabled", "true");
        system.seed_setting("apply.auto.confirmed_at", NOW);
        system.seed_setting("apply.auto.global_daily_cap", "10");
        system
    }

    fn activated(rule_id: &str) -> DryRunState {
        DryRunState {
            rule_id: rule_id.to_string(),
            required: 3,
            seen: 3,
            activated_at: Some(NOW.to_string()),
        }
    }

    fn service(
        items: Arc<FakeOutbox>,
        rules: Arc<FakeAutoRules>,
        system: FakeSystemRepo,
        notifier: Option<Arc<CountingNotifier>>,
    ) -> OutboxService {
        let system = Arc::new(system);
        let rules_service =
            AutoRulesService::new(rules, system.clone(), Arc::new(FixedClock::new(NOW)));
        OutboxService::new(
            items,
            Arc::new(rules_service),
            system,
            Arc::new(FixedClock::new(NOW)),
            notifier.map(|notifier| notifier as Arc<dyn NotifierPort>),
            None,
        )
    }

    fn service_with_kill(
        items: Arc<FakeOutbox>,
        rules: Arc<FakeAutoRules>,
        system: FakeSystemRepo,
        kill: Arc<FakeKillSwitch>,
    ) -> OutboxService {
        let system = Arc::new(system);
        let rules_service =
            AutoRulesService::new(rules, system.clone(), Arc::new(FixedClock::new(NOW)));
        OutboxService::new(
            items,
            Arc::new(rules_service),
            system,
            Arc::new(FixedClock::new(NOW)),
            None,
            Some(kill as Arc<dyn KillSwitchPort>),
        )
    }

    // ---- settings ---------------------------------------------------------

    #[test]
    fn a_fresh_install_holds_for_thirty_minutes() {
        let service = service(
            Arc::default(),
            Arc::default(),
            FakeSystemRepo::default(),
            None,
        );
        assert_eq!(
            service.settings().unwrap(),
            OutboxSettings { hold_minutes: 30 }
        );
    }

    #[test]
    fn the_hold_is_validated_by_core_and_never_by_the_ui() {
        let service = service(
            Arc::default(),
            Arc::default(),
            FakeSystemRepo::default(),
            None,
        );
        assert!(matches!(
            service.set_hold(4),
            Err(OutboxError::Hold(HoldViolation::TooShort))
        ));
        assert!(matches!(
            service.set_hold(1441),
            Err(OutboxError::Hold(HoldViolation::TooLong))
        ));
        assert_eq!(service.set_hold(45).unwrap().hold_minutes, 45);
        assert_eq!(service.settings().unwrap().hold_minutes, 45);
    }

    #[test]
    fn a_malformed_stored_hold_falls_back_to_the_default() {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(HOLD_MINUTES_KEY, "two");
        let service = service(Arc::default(), Arc::default(), system, None);
        assert_eq!(service.settings().unwrap().hold_minutes, 30);
    }

    // ---- the dry run ------------------------------------------------------

    #[test]
    fn a_new_rule_reports_its_first_three_matches_and_sends_nothing() {
        let items = Arc::new(FakeOutbox::default());
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let notifier = Arc::new(CountingNotifier::default());
        let service = service(
            items.clone(),
            rules.clone(),
            system(),
            Some(notifier.clone()),
        );

        for application in ["app_01", "app_02", "app_03"] {
            let enqueued = service
                .enqueue(application, &candidate(), "Backend Engineer", "Acme", 0)
                .unwrap();
            let Enqueued::DryRun { item } = enqueued else {
                panic!("a rule that is not activated only reports");
            };
            assert_eq!(item.state, "would_send");
        }

        assert_eq!(items.count(), 3);
        assert!(rules.ledger().is_empty(), "a dry run never records a send");
        assert_eq!(items.dry("rule_1").unwrap().seen, 3);
        assert!(items.dry("rule_1").unwrap().is_complete());
        assert!(
            notifier.sent.lock().unwrap().is_empty(),
            "a would-send item is not notified"
        );
    }

    #[test]
    fn a_dry_run_keeps_reporting_past_its_three_matches_until_activated() {
        let items = Arc::new(FakeOutbox::default());
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);

        for application in ["app_01", "app_02", "app_03", "app_04"] {
            let enqueued = service
                .enqueue(application, &candidate(), "Backend Engineer", "Acme", 0)
                .unwrap();
            assert!(matches!(enqueued, Enqueued::DryRun { .. }));
        }
        assert_eq!(items.dry("rule_1").unwrap().seen, 3, "it stops counting");
    }

    #[test]
    fn activating_a_rule_moves_its_reported_items_to_the_review_queue() {
        let items = Arc::new(FakeOutbox::default());
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap();

        let view = service.activate_rule("rule_1").unwrap();
        assert!(view.active);
        let stored = items.items();
        assert_eq!(stored[0].state, OutboxState::NeedsReview);
        assert_eq!(stored[0].note.as_deref(), Some("dry_run_ended"));
    }

    #[test]
    fn an_activated_rule_queues_a_held_item_and_notifies() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let notifier = Arc::new(CountingNotifier::default());
        let service = service(
            items.clone(),
            rules.clone(),
            system(),
            Some(notifier.clone()),
        );

        let enqueued = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap();
        let Enqueued::Held { item } = enqueued else {
            panic!("an activated rule holds the send");
        };
        assert_eq!(item.state, "held");
        assert_eq!(item.hold_minutes, 30);
        assert_eq!(item.release_at, "2026-10-17T09:30:00Z");
        assert_eq!(item.minutes_left, 30);
        assert_eq!(
            notifier.sent.lock().unwrap().as_slice(),
            &[Notification::OutboxHold]
        );
        assert!(
            items.dry("rule_1").unwrap().seen == 3,
            "an active rule does not count matches"
        );
    }

    #[test]
    fn a_session_board_send_waits_at_least_an_hour() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let mut session_rule = rule("rule_1");
        session_rule.channels = vec![RuleChannel::SessionBoard];
        let rules = Arc::new(FakeAutoRules::with_rules(vec![session_rule]));
        let service = service(items.clone(), rules, system(), None);

        let mut session = candidate();
        session.channel = RuleChannel::SessionBoard;
        let Enqueued::Held { item } = service
            .enqueue("app_01", &session, "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("an activated rule holds the send");
        };
        assert_eq!(item.hold_minutes, 60);
    }

    #[test]
    fn a_review_decision_queues_nothing() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);

        let mut weak = candidate();
        weak.rank = Some(40.0);
        let enqueued = service
            .enqueue("app_01", &weak, "Backend Engineer", "Acme", 0)
            .unwrap();
        let Enqueued::NotAuto { decision } = enqueued else {
            panic!("a low rank never queues");
        };
        assert_eq!(decision.kind, "review");
        assert_eq!(items.count(), 0);
    }

    #[test]
    fn the_same_application_never_queues_twice() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);

        service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap();
        assert!(matches!(
            service.enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0),
            Err(OutboxError::AlreadyQueued(_))
        ));
    }

    // ---- the hold ---------------------------------------------------------

    #[test]
    fn an_item_can_be_cancelled_during_its_hold_and_not_after() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        let cancelled = service.cancel(&item.id).unwrap();
        assert_eq!(cancelled.state, "cancelled");
        assert_eq!(cancelled.note.as_deref(), Some("user"));
        assert!(matches!(
            service.cancel(&item.id),
            Err(OutboxError::NotWaiting(_))
        ));
    }

    #[test]
    fn an_edit_is_kept_for_the_send_and_blank_clears_it() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        let edited = service
            .edit(&item.id, Some("  Hello  ".into()), Some("Body line".into()))
            .unwrap();
        assert_eq!(edited.subject_override.as_deref(), Some("Hello"));
        assert_eq!(edited.body_override.as_deref(), Some("Body line"));

        let cleared = service.edit(&item.id, Some(String::new()), None).unwrap();
        assert_eq!(cleared.subject_override, None);
    }

    #[test]
    fn an_edit_past_the_cap_is_refused() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        let huge = "x".repeat(EDIT_MAX_CHARS + 1);
        assert!(matches!(
            service.edit(&item.id, None, Some(huge)),
            Err(OutboxError::Edit)
        ));
    }

    #[test]
    fn only_a_held_item_whose_time_has_come_is_due() {
        let items = Arc::new(FakeOutbox::default());
        let due = OutboxItem {
            id: "out_due".into(),
            application_id: "app_01".into(),
            rule_id: "rule_1".into(),
            source_id: "remotive".into(),
            channel: "email".into(),
            company_key: "acme".into(),
            title: "Backend Engineer".into(),
            company: "Acme".into(),
            state: OutboxState::Held,
            hold_minutes: 30,
            release_at: "2026-10-17T08:59:00Z".into(),
            subject_override: None,
            body_override: None,
            note: None,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        };
        let mut later = due.clone();
        later.id = "out_later".into();
        later.application_id = "app_02".into();
        later.release_at = "2026-10-17T10:00:00Z".into();
        items.seed_item(due);
        items.seed_item(later);

        let service = service(items, Arc::default(), system(), None);
        let due = service.due().unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "out_due");
    }

    #[test]
    fn a_release_rechecks_the_policy_and_keeps_a_refused_item_for_review() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        // The rule is switched off while the item waits.
        rules
            .rule_upsert(&AutoRule {
                enabled: false,
                ..rule("rule_1")
            })
            .unwrap();

        let released = service.release(&item.id, &candidate(), 0).unwrap();
        let Release::Review(stored) = released else {
            panic!("a disabled rule never sends");
        };
        assert_eq!(stored.state, OutboxState::NeedsReview);
        assert!(stored.note.unwrap().starts_with("review:"));
        assert!(rules.ledger().is_empty(), "no cap entry without a send");
    }

    #[test]
    fn the_cap_entry_waits_until_the_send_is_certain() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        let released = service.release(&item.id, &candidate(), 0).unwrap();
        let Release::Sending {
            item: stored,
            rule_id,
        } = released
        else {
            panic!("the policy still says Auto");
        };
        assert_eq!(stored.state, OutboxState::Releasing);
        assert!(
            rules.ledger().is_empty(),
            "nothing is counted before the send is certain (review B-3)"
        );

        service.record_send(&stored.id, &rule_id).unwrap();
        let ledger = rules.ledger();
        assert_eq!(ledger.len(), 1, "the send is counted before it leaves");
        assert_eq!(ledger[0].rule_id, "rule_1");
        assert_eq!(ledger[0].application_id, "app_01");
        assert_eq!(ledger[0].channel, RuleChannel::Email);
    }

    #[test]
    fn a_send_is_recorded_only_once_and_only_while_releasing() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        // Still `held`: the caller has not claimed it, so nothing is counted.
        assert!(matches!(
            service.record_send(&item.id, "rule_1"),
            Err(OutboxError::NotWaiting(_))
        ));
        assert!(rules.ledger().is_empty());
    }

    #[test]
    fn a_dry_run_item_is_never_released() {
        let items = Arc::new(FakeOutbox::default());
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::DryRun { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("a rule that is not activated only reports");
        };

        assert!(matches!(
            service.release(&item.id, &candidate(), 0),
            Err(OutboxError::NotWaiting(_))
        ));
        assert!(rules.ledger().is_empty());
    }

    // FR-92 (S9.5): a duplicate application never leaves the review queue,
    // whatever the rule says.
    #[test]
    fn a_duplicate_candidate_is_never_sent_automatically() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        let mut duplicate = candidate();
        duplicate.duplicate = true;

        let released = service.release(&item.id, &duplicate, 0).unwrap();

        assert!(
            matches!(released, Release::Review(_)),
            "a duplicate is reviewed, never auto-sent"
        );
    }

    #[test]
    fn a_release_finishes_sent_or_back_in_the_review_queue() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        service.release(&item.id, &candidate(), 0).unwrap();

        let sent = service.finish(&item.id, Finish::Sent).unwrap();
        assert_eq!(sent.state, "sent");
        assert!(matches!(
            service.finish(&item.id, Finish::Sent),
            Err(OutboxError::NotWaiting(_))
        ));
    }

    #[test]
    fn a_send_that_failed_returns_to_the_review_queue() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        service.release(&item.id, &candidate(), 0).unwrap();

        let stored = service.finish(&item.id, Finish::NeedsReview).unwrap();
        assert_eq!(stored.state, "needs_review");
        assert_eq!(stored.note.as_deref(), Some("send_failed"));
    }

    #[test]
    fn a_crash_mid_release_moves_the_item_to_the_review_queue_at_startup() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        service.release(&item.id, &candidate(), 0).unwrap();

        assert_eq!(service.reconcile().unwrap(), 1);
        let stored = items.items();
        assert_eq!(stored[0].state, OutboxState::NeedsReview);
        assert_eq!(stored[0].note.as_deref(), Some("interrupted"));
    }

    #[test]
    fn a_channel_this_build_cannot_send_automatically_is_parked_for_review() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let mut ats = rule("rule_1");
        ats.channels = vec![RuleChannel::AtsForm];
        let rules = Arc::new(FakeAutoRules::with_rules(vec![ats]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let mut form = candidate();
        form.channel = RuleChannel::AtsForm;
        let Enqueued::Held { item } = service
            .enqueue("app_01", &form, "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        let parked = service.park(&item.id, "channel_unavailable").unwrap();
        assert_eq!(parked.state, "needs_review");
        assert_eq!(parked.note.as_deref(), Some("channel_unavailable"));
        assert!(rules.ledger().is_empty(), "no cap entry without a send");
    }

    #[test]
    fn an_engaged_kill_switch_stops_new_items_from_being_queued() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let kill = Arc::new(FakeKillSwitch::default());
        let service = service_with_kill(items.clone(), rules, system(), kill.clone());
        kill.engage();

        assert!(matches!(
            service.enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0),
            Err(OutboxError::KillSwitch)
        ));
        assert_eq!(items.count(), 0);
    }

    #[test]
    fn an_engaged_kill_switch_leaves_the_item_held() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let kill = Arc::new(FakeKillSwitch::default());
        let service = service_with_kill(items.clone(), rules.clone(), system(), kill.clone());
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        kill.engage();
        assert!(matches!(
            service.release(&item.id, &candidate(), 0),
            Err(OutboxError::KillSwitch)
        ));
        assert_eq!(items.items()[0].state, OutboxState::Held);
        assert!(rules.ledger().is_empty());
    }

    #[test]
    fn a_spacing_refusal_keeps_the_item_held_instead_of_bothering_the_user() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        // Any Source sent at 08:58, two minutes before the release: S9.6's
        // cross-rule gap is not over yet.
        rules.seed_ledger(AutoAttemptEntry {
            id: "aut_earlier".into(),
            rule_id: "rule_1".into(),
            application_id: "app_00".into(),
            source_id: "other_board".into(),
            channel: RuleChannel::Email,
            company_key: "other".into(),
            created_at: "2026-10-17T08:58:00Z".into(),
        });

        let released = service.release(&item.id, &candidate(), 0).unwrap();
        let Release::Deferred { item: held, .. } = released else {
            panic!("a spacing refusal defers, it never sends");
        };
        assert_eq!(held.state, OutboxState::Held);
        assert_eq!(rules.ledger().len(), 1, "no new cap entry while deferred");
    }

    #[test]
    fn a_session_board_source_keeps_ten_minutes_between_sends() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let mut session_rule = rule("rule_1");
        session_rule.channels = vec![RuleChannel::SessionBoard];
        let rules = Arc::new(FakeAutoRules::with_rules(vec![session_rule]));
        // Eight minutes since the last send on this Source: past S9.6's 5-minute
        // cross-rule gap, inside FR-91's 10-minute Session Board floor.
        rules.seed_ledger(AutoAttemptEntry {
            id: "aut_earlier".into(),
            rule_id: "rule_1".into(),
            application_id: "app_00".into(),
            source_id: "remotive".into(),
            channel: RuleChannel::SessionBoard,
            company_key: "other".into(),
            created_at: "2026-10-17T08:52:00Z".into(),
        });
        let service = service(items.clone(), rules.clone(), system(), None);
        let mut session = candidate();
        session.channel = RuleChannel::SessionBoard;
        let Enqueued::Held { item } = service
            .enqueue("app_01", &session, "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };

        let released = service.release(&item.id, &session, 0).unwrap();
        let Release::Deferred {
            item: held,
            wait_minutes,
        } = released
        else {
            panic!("the Session Board floor is not over yet");
        };
        assert_eq!(held.state, OutboxState::Held);
        assert_eq!(wait_minutes, 2, "ten minutes minus the eight that passed");
    }

    #[test]
    fn the_pre_submit_check_stops_a_decision_that_changed_mid_release() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules.clone(), system(), None);
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        service.release(&item.id, &candidate(), 0).unwrap();

        // The rule is turned off between the release and the channel's submit.
        rules
            .rule_upsert(&AutoRule {
                enabled: false,
                ..rule("rule_1")
            })
            .unwrap();

        assert_eq!(
            service
                .confirm_before_submit(&item.id, &candidate(), 0)
                .unwrap(),
            Submit::Stop
        );
        assert_eq!(items.items()[0].state, OutboxState::NeedsReview);
        assert!(
            rules.ledger().is_empty(),
            "a stopped submit never counts as a send (review B-3)"
        );
    }

    #[test]
    fn the_pre_submit_check_stops_on_the_kill_switch_too() {
        let items = Arc::new(FakeOutbox::default());
        items.seed_dry(activated("rule_1"));
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let kill = Arc::new(FakeKillSwitch::default());
        let service = service_with_kill(items.clone(), rules, system(), kill.clone());
        let Enqueued::Held { item } = service
            .enqueue("app_01", &candidate(), "Backend Engineer", "Acme", 0)
            .unwrap()
        else {
            panic!("held");
        };
        service.release(&item.id, &candidate(), 0).unwrap();

        kill.engage();
        assert_eq!(
            service
                .confirm_before_submit(&item.id, &candidate(), 0)
                .unwrap(),
            Submit::Stop
        );
        let stored = items.items();
        assert_eq!(stored[0].state, OutboxState::NeedsReview);
        assert_eq!(stored[0].note.as_deref(), Some("kill_switch"));
    }

    // ---- the snapshot -----------------------------------------------------

    #[test]
    fn the_snapshot_lists_every_rule_dry_run_with_its_progress() {
        let items = Arc::new(FakeOutbox::default());
        let mut second = rule("rule_2");
        second.name = "ATS × remote".into();
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1"), second]));
        items.seed_dry(activated("rule_1"));
        let service = service(items.clone(), rules, system(), None);

        let view = service.snapshot().unwrap();
        assert_eq!(view.rules.len(), 2);
        assert!(view.rules[0].active);
        assert!(view.rules[0].complete);
        assert!(!view.rules[1].active);
        assert_eq!(view.rules[1].required, 3, "a rule that never ran reports 3");
        assert_eq!(view.rules[1].rule_name, "ATS × remote");
    }

    #[test]
    fn the_dry_run_length_is_validated_and_kept_per_rule() {
        let items = Arc::new(FakeOutbox::default());
        let rules = Arc::new(FakeAutoRules::with_rules(vec![rule("rule_1")]));
        let service = service(items.clone(), rules, system(), None);

        assert!(matches!(
            service.set_dry_run("rule_1", 0),
            Err(OutboxError::DryRun(DryRunViolation::TooShort))
        ));
        assert!(matches!(
            service.set_dry_run("rule_1", 11),
            Err(OutboxError::DryRun(DryRunViolation::TooLong))
        ));
        let view = service.set_dry_run("rule_1", 1).unwrap();
        assert_eq!(view.required, 1);
        assert_eq!(items.dry("rule_1").unwrap().required, 1);
    }

    #[test]
    fn a_rule_this_build_does_not_store_cannot_be_activated() {
        let service = service(Arc::default(), Arc::default(), system(), None);
        assert!(matches!(
            service.activate_rule("rule_missing"),
            Err(OutboxError::UnknownRule(_))
        ));
    }

    #[test]
    fn an_unknown_item_is_not_found_rather_than_a_silent_write() {
        let service = service(Arc::default(), Arc::default(), system(), None);
        assert!(matches!(
            service.cancel("out_missing"),
            Err(OutboxError::UnknownItem(_))
        ));
        assert!(matches!(
            service.finish("out_missing", Finish::Sent),
            Err(OutboxError::UnknownItem(_))
        ));
    }
}
