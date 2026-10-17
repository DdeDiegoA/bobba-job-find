//! Tracker use cases (S9.1, FR-93/94, AD-16): the board, one application's
//! detail and the one way a status may change (AC4).
//!
//! The status machine itself lives in `boba-domain::application`; this layer
//! turns it into the shapes the screen needs and is the only writer of
//! `application.status` + `status_event`. Every move — a drag, an approved
//! Status Proposal, an apply outcome — travels the same path, so the history can
//! never be bypassed (AD-16).
//!
//! ponytail: the module lands in `applications/` because S9.1 owns it; the
//! owning *area* is still apply (AD-16), the store adapter is the same
//! `ApplyRepo`.

pub mod testing;

use std::sync::Arc;

use boba_domain::application::{
    AppStatus, Application, PackageItem, Stage, StatusEvent, StatusSource, new_status_event_id,
};
use boba_domain::posting::Offer;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::clock::ClockPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};

/// The domain rule the status machine refuses a move with (re-exported so a
/// caller — the command layer — needs no direct `boba-domain` dependency).
pub use boba_domain::application::RuleError;

/// A tracker failure (AD-3: the UI translates `message_key`, never raw text).
#[derive(Debug, Clone, thiserror::Error)]
pub enum TrackerError {
    #[error(transparent)]
    Repo(#[from] ApplyRepoError),
    /// The move changes nothing (already in that status).
    #[error(transparent)]
    Rule(#[from] RuleError),
    /// The UI sent a status that is not in the vocabulary.
    #[error("unknown application status: {0}")]
    UnknownStatus(String),
    /// No such application.
    #[error("unknown application: {0}")]
    UnknownApplication(String),
}

impl TrackerError {
    /// The i18n key the UI shows.
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(ApplyRepoError::Store(_)) => "errors.tracker.store",
            Self::Repo(ApplyRepoError::UnknownApplication(_)) => {
                "errors.tracker.unknown_application"
            }
            Self::Repo(ApplyRepoError::Conflict { .. }) => "errors.tracker.stale",
            Self::Rule(RuleError::SameStatus) => "errors.tracker.same_status",
            Self::UnknownStatus(_) => "errors.tracker.unknown_status",
            Self::UnknownApplication(_) => "errors.tracker.unknown_application",
        }
    }
}

/// The Offer as it looked when the application was created (FR-94). Stored as
/// JSON on the row, so the card keeps showing what was applied to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct OfferSnapshot {
    pub title: String,
    pub company: String,
    #[serde(default)]
    pub apply_url: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub deadline: Option<String>,
}

impl From<&Offer> for OfferSnapshot {
    fn from(offer: &Offer) -> Self {
        Self {
            title: offer.title.clone(),
            company: offer.company.clone(),
            apply_url: Some(offer.apply.url.clone()),
            lang: Some(offer.lang.clone()),
            deadline: offer.deadline.clone(),
        }
    }
}

impl OfferSnapshot {
    /// The snapshot as it is stored on the row.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Read a stored snapshot. A row written by an older build (or corrupted)
    /// shows an empty card instead of taking the whole board down.
    fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }
}

/// One card on the board (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ApplicationCard {
    pub id: String,
    /// The stored status (`saved`, `drafted`, …).
    pub status: String,
    /// The column it sits in.
    pub stage: String,
    pub title: String,
    pub company: String,
    pub next_action: Option<String>,
}

/// One column (AC1). The four terminal columns collapse in the UI (FR-93).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BoardColumn {
    pub stage: String,
    /// The status a card gets when it is dropped here (AC2): a drop names a
    /// column, the column names the status.
    pub status: String,
    pub terminal: bool,
    pub count: u32,
    pub cards: Vec<ApplicationCard>,
}

/// The whole Tracker in one read (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BoardView {
    pub columns: Vec<BoardColumn>,
}

/// One document of the Package the application carries (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PackageItemView {
    pub document_id: String,
    /// `cv` or `cover_letter`.
    pub kind: String,
    pub lang: String,
    /// `u32` because specta refuses to export 64-bit integers (AD-3).
    pub version: u32,
    /// A document is only attachable once verified (AD-14).
    pub verified: bool,
}

/// One move in the timeline (AC3). The Stage is the label the UI shows (FR-94:
/// "Stage changes"), resolved here so no screen repeats the status → column map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StatusEventView {
    pub id: String,
    pub from_status: Option<String>,
    pub from_stage: Option<String>,
    pub to_status: String,
    pub to_stage: String,
    /// `user`, `status_proposal`, `apply_outcome` or `import`.
    pub source: String,
    pub reason: Option<String>,
    pub created_at: String,
}

/// Everything the card detail shows (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ApplicationDetail {
    pub id: String,
    pub posting_id: String,
    pub status: String,
    pub stage: String,
    pub title: String,
    pub company: String,
    pub apply_url: Option<String>,
    pub lang: Option<String>,
    pub deadline: Option<String>,
    pub notes: Option<String>,
    pub next_action: Option<String>,
    pub package: Vec<PackageItemView>,
    pub timeline: Vec<StatusEventView>,
    pub created_at: String,
    pub updated_at: String,
}

/// A move the UI asks for (AC2/AC4). `source` names who caused it, so an
/// approved Status Proposal or an apply outcome travels the same path as a drag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TransitionInput {
    pub application_id: String,
    /// The status the card showed when the user moved it (the race guard).
    pub from_status: String,
    pub to_status: String,
    /// `user`, `status_proposal`, `apply_outcome` or `import`.
    pub source: String,
    pub reason: Option<String>,
}

/// The Tracker use cases (AC1–AC4) over the apply area's repo.
pub struct TrackerService {
    repo: Arc<dyn ApplyRepo>,
    clock: Arc<dyn ClockPort>,
}

impl TrackerService {
    pub fn new(repo: Arc<dyn ApplyRepo>, clock: Arc<dyn ClockPort>) -> Self {
        Self { repo, clock }
    }

    /// The board: every column in order, with its cards (AC1).
    pub fn board(&self) -> Result<BoardView, TrackerError> {
        let applications = self.repo.applications()?;
        let columns = Stage::ALL
            .into_iter()
            .map(|stage| {
                let cards: Vec<ApplicationCard> = applications
                    .iter()
                    .filter(|application| application.status.stage() == stage)
                    .map(card)
                    .collect();
                BoardColumn {
                    stage: stage.as_str().to_string(),
                    status: stage.status().as_str().to_string(),
                    terminal: stage.is_terminal(),
                    count: cards.len() as u32,
                    cards,
                }
            })
            .collect();
        Ok(BoardView { columns })
    }

    /// One card in full: snapshot, package, timeline, next action, notes (AC3).
    pub fn detail(&self, application_id: &str) -> Result<ApplicationDetail, TrackerError> {
        let row = self.application(application_id)?;
        let snapshot = OfferSnapshot::parse(&row.offer_snapshot_json);
        let package = self
            .repo
            .package(application_id)?
            .into_iter()
            .map(package_view)
            .collect();
        let timeline = self
            .repo
            .status_events(application_id)?
            .into_iter()
            .map(event_view)
            .collect();
        Ok(ApplicationDetail {
            id: row.id,
            posting_id: row.posting_id,
            status: row.status.as_str().to_string(),
            stage: row.status.stage().as_str().to_string(),
            title: snapshot.title,
            company: snapshot.company,
            apply_url: snapshot.apply_url,
            lang: snapshot.lang,
            deadline: snapshot.deadline,
            notes: row.notes,
            next_action: row.next_action,
            package,
            timeline,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// The only way a status may change (AC4): validate, write one
    /// `status_event`, return the updated card.
    ///
    /// A row's *birth* is the one event this cannot write: the application must
    /// exist first. That first event (`from_status: None`) is written with the
    /// row by the owner that creates it — Save in the Inbox (S6.6) — through
    /// `ApplyRepo::application_create`, in the same transaction.
    pub fn transition(&self, input: TransitionInput) -> Result<ApplicationDetail, TrackerError> {
        let from = AppStatus::parse(&input.from_status)
            .ok_or_else(|| TrackerError::UnknownStatus(input.from_status.clone()))?;
        let to = AppStatus::parse(&input.to_status)
            .ok_or_else(|| TrackerError::UnknownStatus(input.to_status.clone()))?;
        let source = StatusSource::parse(&input.source)
            .ok_or_else(|| TrackerError::UnknownStatus(input.source.clone()))?;
        boba_domain::application::can_transition(from, to)?;

        // The application must exist before the (append-only) history grows.
        self.application(&input.application_id)?;

        let event = StatusEvent {
            id: new_status_event_id(),
            application_id: input.application_id.clone(),
            from_status: Some(from),
            to_status: to,
            source,
            reason: input.reason,
            created_at: self.clock.now_rfc3339(),
        };
        self.repo.append_status_event(&event)?;
        self.detail(&input.application_id)
    }

    fn application(&self, id: &str) -> Result<Application, TrackerError> {
        self.repo
            .application_get(id)?
            .ok_or_else(|| TrackerError::UnknownApplication(id.to_string()))
    }
}

fn package_view(item: PackageItem) -> PackageItemView {
    PackageItemView {
        document_id: item.document_id,
        kind: item.kind,
        lang: item.lang,
        // The store assigns this version; it never approaches u32::MAX.
        version: u32::try_from(item.version).unwrap_or_default(),
        verified: item.verified_at.is_some(),
    }
}

fn card(application: &Application) -> ApplicationCard {
    let snapshot = OfferSnapshot::parse(&application.offer_snapshot_json);
    ApplicationCard {
        id: application.id.clone(),
        status: application.status.as_str().to_string(),
        stage: application.status.stage().as_str().to_string(),
        title: snapshot.title,
        company: snapshot.company,
        next_action: application.next_action.clone(),
    }
}

fn event_view(event: StatusEvent) -> StatusEventView {
    StatusEventView {
        id: event.id,
        from_status: event.from_status.map(|s| s.as_str().to_string()),
        from_stage: event.from_status.map(|s| s.stage().as_str().to_string()),
        to_status: event.to_status.as_str().to_string(),
        to_stage: event.to_status.stage().as_str().to_string(),
        source: event.source.as_str().to_string(),
        reason: event.reason,
        created_at: event.created_at,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use boba_domain::application::{Application, PackageItem, StatusSource};

    use super::testing::FakeApplyRepo;
    use super::*;
    use crate::system::FixedClock;

    const SNAPSHOT: &str = r#"{"title":"Backend Engineer","company":"Boba Labs",
        "apply_url":"https://jobs.example.com/abc-1","lang":"en"}"#;

    fn application(id: &str, status: AppStatus) -> Application {
        Application {
            id: id.to_string(),
            posting_id: "pst_1".to_string(),
            status,
            offer_snapshot_json: SNAPSHOT.to_string(),
            notes: Some("referred by Ana".to_string()),
            next_action: Some("follow up on Friday".to_string()),
            created_at: "2026-10-16T09:00:00Z".to_string(),
            updated_at: "2026-10-16T09:00:00Z".to_string(),
        }
    }

    /// A service over a fake repo holding `app_01` (Saved) and `app_02` (Applied).
    fn service() -> (TrackerService, FakeApplyRepo) {
        let repo = FakeApplyRepo::default();
        repo.seed_application(application("app_01", AppStatus::Saved));
        repo.seed_application(application("app_02", AppStatus::Applied));
        let service = TrackerService::new(
            Arc::new(repo.clone()),
            Arc::new(FixedClock::new("2026-10-17T10:00:00Z")),
        );
        (service, repo)
    }

    fn input(from: AppStatus, to: AppStatus) -> TransitionInput {
        TransitionInput {
            application_id: "app_01".to_string(),
            from_status: from.as_str().to_string(),
            to_status: to.as_str().to_string(),
            source: "user".to_string(),
            reason: None,
        }
    }

    // AC1: the columns are the nine stages, in order, with the cards in them.
    #[test]
    fn the_board_is_the_nine_columns_with_their_cards() {
        let (service, repo) = service();
        repo.seed_application(application("app_03", AppStatus::Drafted));
        repo.seed_application(application("app_04", AppStatus::NoResponse));

        let board = service.board().unwrap();
        let stages: Vec<&str> = board.columns.iter().map(|c| c.stage.as_str()).collect();
        assert_eq!(
            stages,
            vec![
                "saved",
                "preparing",
                "applied",
                "interviewing",
                "offer",
                "hired",
                "rejected",
                "no_response",
                "withdrawn",
            ]
        );

        let saved = &board.columns[0];
        assert_eq!(saved.count, 1);
        assert_eq!(saved.cards[0].id, "app_01");
        assert_eq!(saved.cards[0].title, "Backend Engineer");
        assert_eq!(saved.cards[0].company, "Boba Labs");
        assert_eq!(saved.cards[0].status, "saved");
        assert_eq!(saved.status, "saved");
        // A drop in Preparing writes `drafted`: the UI maps nothing itself.
        assert_eq!(board.columns[1].status, "drafted");
        assert_eq!(board.columns[8].status, "withdrawn");
        assert_eq!(board.columns[1].cards[0].id, "app_03");
        assert_eq!(board.columns[2].cards[0].id, "app_02");
        assert_eq!(board.columns[7].cards[0].id, "app_04");
        assert_eq!(board.columns[8].count, 0);
    }

    // AC1: upstream's `offer_declined` is the user walking away → Withdrawn.
    #[test]
    fn a_declined_offer_shows_up_in_the_withdrawn_column() {
        let (service, repo) = service();
        repo.seed_application(application("app_05", AppStatus::OfferDeclined));

        let board = service.board().unwrap();
        let withdrawn = board.columns.last().unwrap();
        assert_eq!(withdrawn.cards[0].id, "app_05");
        assert_eq!(withdrawn.cards[0].status, "offer_declined");
    }

    // AC1: the four terminal columns collapse (FR-93).
    #[test]
    fn only_the_four_terminal_columns_are_collapsible() {
        let (service, _) = service();
        let board = service.board().unwrap();
        let terminal: Vec<&str> = board
            .columns
            .iter()
            .filter(|column| column.terminal)
            .map(|column| column.stage.as_str())
            .collect();
        assert_eq!(
            terminal,
            vec!["hired", "rejected", "no_response", "withdrawn"]
        );
    }

    // AC2/AC4: a move writes exactly one event, with the injected clock.
    #[test]
    fn a_move_logs_one_status_event_from_the_injected_clock() {
        let (service, repo) = service();

        let detail = service
            .transition(input(AppStatus::Saved, AppStatus::Interview))
            .unwrap();

        assert_eq!(detail.status, "interview");
        assert_eq!(detail.stage, "interviewing");
        let events = repo.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].application_id, "app_01");
        assert_eq!(events[0].from_status, Some(AppStatus::Saved));
        assert_eq!(events[0].to_status, AppStatus::Interview);
        assert_eq!(events[0].source, StatusSource::User);
        assert_eq!(events[0].created_at, "2026-10-17T10:00:00Z");
        assert!(events[0].id.starts_with("evt_"));
        assert_eq!(detail.timeline.len(), 1);
        assert_eq!(detail.timeline[0].to_status, "interview");
        assert_eq!(detail.timeline[0].from_stage.as_deref(), Some("saved"));
        assert_eq!(detail.timeline[0].to_stage, "interviewing");
    }

    // AC4: an approved proposal or an apply outcome travels the same path.
    #[test]
    fn a_proposal_approval_records_its_own_source() {
        let (service, repo) = service();
        let mut proposal = input(AppStatus::Applied, AppStatus::Interview);
        proposal.application_id = "app_02".to_string();
        proposal.source = "status_proposal".to_string();
        proposal.reason = Some("\"we'd like to schedule a call\"".to_string());

        service.transition(proposal).unwrap();

        let events = repo.events();
        assert_eq!(events[0].source, StatusSource::StatusProposal);
        assert_eq!(
            events[0].reason.as_deref(),
            Some("\"we'd like to schedule a call\"")
        );
    }

    // AC4: an invalid move is rejected with an i18n error and writes nothing.
    #[test]
    fn a_move_to_where_the_card_already_is_writes_nothing() {
        let (service, repo) = service();
        let error = service
            .transition(input(AppStatus::Saved, AppStatus::Saved))
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.tracker.same_status");
        assert!(repo.events().is_empty());
        assert_eq!(repo.stored_status("app_01"), Some(AppStatus::Saved));
    }

    #[test]
    fn a_status_the_domain_does_not_know_is_rejected() {
        let (service, repo) = service();
        let mut unknown = input(AppStatus::Saved, AppStatus::Applied);
        unknown.to_status = "needs_check".to_string();
        let error = service.transition(unknown).unwrap_err();
        assert_eq!(error.message_key(), "errors.tracker.unknown_status");
        assert!(repo.events().is_empty());
    }

    // AC4: a move that lost the race re-reads instead of overwriting.
    #[test]
    fn a_move_that_lost_the_race_reports_stale() {
        let (service, repo) = service();
        service
            .transition(input(AppStatus::Saved, AppStatus::Applied))
            .unwrap();
        // Another window still shows the card as Saved and drops it on Offer.
        let error = service
            .transition(input(AppStatus::Saved, AppStatus::Offer))
            .unwrap_err();
        assert_eq!(error.message_key(), "errors.tracker.stale");
        assert_eq!(repo.events().len(), 1);
        assert_eq!(repo.stored_status("app_01"), Some(AppStatus::Applied));
    }

    #[test]
    fn moving_an_unknown_application_is_not_found() {
        let (service, _) = service();
        let mut missing = input(AppStatus::Saved, AppStatus::Applied);
        missing.application_id = "app_99".to_string();
        let error = service.transition(missing).unwrap_err();
        assert_eq!(error.message_key(), "errors.tracker.unknown_application");
    }

    // AC3: the card detail is the snapshot, the package, the timeline, notes.
    #[test]
    fn the_detail_shows_snapshot_package_timeline_and_notes() {
        let (service, repo) = service();
        repo.seed_package(
            "app_01",
            PackageItem {
                document_id: "doc_1".to_string(),
                kind: "cv".to_string(),
                lang: "en".to_string(),
                version: 2,
                verified_at: Some("2026-10-16T09:30:00Z".to_string()),
            },
        );
        repo.seed_package(
            "app_01",
            PackageItem {
                document_id: "doc_2".to_string(),
                kind: "cover_letter".to_string(),
                lang: "en".to_string(),
                version: 1,
                verified_at: None,
            },
        );

        let detail = service.detail("app_01").unwrap();

        assert_eq!(detail.title, "Backend Engineer");
        assert_eq!(detail.company, "Boba Labs");
        assert_eq!(
            detail.apply_url.as_deref(),
            Some("https://jobs.example.com/abc-1")
        );
        assert_eq!(detail.lang.as_deref(), Some("en"));
        assert_eq!(detail.notes.as_deref(), Some("referred by Ana"));
        assert_eq!(detail.next_action.as_deref(), Some("follow up on Friday"));
        assert_eq!(detail.package.len(), 2);
        assert_eq!(detail.package[0].kind, "cv");
        assert_eq!(detail.package[0].version, 2);
        assert!(detail.package[0].verified);
        assert!(!detail.package[1].verified);
        assert!(detail.timeline.is_empty());
    }

    #[test]
    fn the_detail_of_an_unknown_application_is_not_found() {
        let (service, _) = service();
        let error = service.detail("app_99").unwrap_err();
        assert_eq!(error.message_key(), "errors.tracker.unknown_application");
    }

    // AD-3: the UI translates a key, never a raw string.
    #[test]
    fn every_error_carries_an_i18n_key() {
        for error in [
            TrackerError::Repo(ApplyRepoError::Store("disk".to_string())),
            TrackerError::Repo(ApplyRepoError::UnknownApplication("app_01".to_string())),
            TrackerError::Repo(ApplyRepoError::Conflict {
                expected: "saved".to_string(),
                found: "applied".to_string(),
            }),
            TrackerError::Rule(RuleError::SameStatus),
            TrackerError::UnknownStatus("needs_check".to_string()),
        ] {
            assert!(error.message_key().starts_with("errors.tracker."));
        }
    }
}
