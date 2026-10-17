//! Funnel stats (S9.9, FR-97, SM-4/SM-5): Applications by Stage, reply rate
//! and interview rate, broken down by Source, Verdict and language.
//!
//! Everything is computed locally from what the owning areas already store, and
//! read through their query functions (AD-16): `application` + `status_event`
//! from [`ApplyRepo`], `posting` from [`IngestionRepo`], `score_card` from
//! [`ScoringRepo`] and Source names from [`SourcesRepo`]. This module writes
//! nothing.
//!
//! The rates count **applications that were applied to**, not every row on the
//! board: a Saved or Preparing card has not entered the funnel yet.
//!
//! - **Reply**: the history reached Interviewing, Offer, Hired, Rejected or
//!   Offer declined. A rejection is a reply; `No Response` and a withdrawal
//!   from Saved are not.
//! - **Interview**: the history reached Interviewing, Offer, Hired or Offer
//!   declined (an Offer implies the interview).
//!
//! The history (`status_event`) is the source of truth, so a later move never
//! erases an earlier reply.

pub mod audit;

use std::collections::BTreeMap;
use std::sync::Arc;

use boba_domain::application::{AppStatus, Application, Stage, StatusEvent};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};

/// A stats failure (AD-3: the UI translates `message_key`, never raw text).
#[derive(Debug, Clone, thiserror::Error)]
pub enum StatsError {
    #[error(transparent)]
    Apply(#[from] ApplyRepoError),
    #[error(transparent)]
    Ingestion(#[from] IngestionRepoError),
    #[error(transparent)]
    Scoring(#[from] ScoringRepoError),
    #[error(transparent)]
    Sources(#[from] SourcesRepoError),
    #[error(transparent)]
    Outbox(#[from] crate::ports::repo::outbox_hold::OutboxHoldRepoError),
    #[error(transparent)]
    Rules(#[from] crate::ports::repo::auto_rules::AutoRulesRepoError),
    #[error(transparent)]
    System(#[from] crate::ports::repo::system::SystemRepoError),
}

impl StatsError {
    /// The i18n key the UI shows (CH7: `errors.audit_stats.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Apply(_)
            | Self::Ingestion(_)
            | Self::Scoring(_)
            | Self::Sources(_)
            | Self::Outbox(_)
            | Self::Rules(_)
            | Self::System(_) => "errors.audit_stats.store",
        }
    }
}

/// One Stage column and how many Applications currently sit in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StageCount {
    /// The Stage id (`saved`, `preparing`, …), the i18n key is `stage.<id>`.
    pub stage: String,
    pub count: u32,
}

/// One row of a breakdown: a Source, a Verdict band or a language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RateRow {
    /// The grouping id: a `source` id, a Verdict band, a BCP-47 tag, or
    /// `unscored` / `unknown` when the data is missing.
    pub key: String,
    /// A human label when the key is an id (the Source's name).
    pub label: Option<String>,
    /// Applications that were applied to in this group.
    pub applications: u32,
    pub replies: u32,
    pub interviews: u32,
    /// `replies / applications` in 0..1, `0` for an empty group.
    pub reply_rate: f64,
    /// `interviews / applications` in 0..1, `0` for an empty group.
    pub interview_rate: f64,
}

/// The whole funnel view (AC-2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FunnelStats {
    /// Every Application on the board, all stages.
    pub applications: u32,
    /// Applications that were applied to: the denominator of both rates.
    pub applied: u32,
    pub replies: u32,
    pub interviews: u32,
    pub reply_rate: f64,
    pub interview_rate: f64,
    /// Every Stage, in board order, including the ones with no cards.
    pub by_stage: Vec<StageCount>,
    pub by_source: Vec<RateRow>,
    pub by_verdict: Vec<RateRow>,
    pub by_language: Vec<RateRow>,
}

/// FR-97's dashboard over the store.
pub struct StatsService {
    applications: Arc<dyn ApplyRepo>,
    ingestion: Arc<dyn IngestionRepo>,
    scoring: Arc<dyn ScoringRepo>,
    sources: Arc<dyn SourcesRepo>,
}

impl StatsService {
    pub fn new(
        applications: Arc<dyn ApplyRepo>,
        ingestion: Arc<dyn IngestionRepo>,
        scoring: Arc<dyn ScoringRepo>,
        sources: Arc<dyn SourcesRepo>,
    ) -> Self {
        Self {
            applications,
            ingestion,
            scoring,
            sources,
        }
    }

    /// Compute the funnel (AC-2). Local only: no network, no agent.
    pub fn funnel(&self) -> Result<FunnelStats, StatsError> {
        let applications = self.applications.applications()?;
        let source_names: BTreeMap<String, String> = self
            .sources
            .list_sources()?
            .into_iter()
            .map(|source| (source.id, source.name))
            .collect();

        let mut by_stage: BTreeMap<&str, u32> =
            Stage::ALL.iter().map(|stage| (stage.as_str(), 0)).collect();
        for application in &applications {
            *by_stage
                .entry(application.status.stage().as_str())
                .or_insert(0) += 1;
        }

        // The posting ids of the applications that entered the funnel.
        let mut applied: Vec<&Application> = Vec::new();
        let mut events: BTreeMap<&str, Vec<StatusEvent>> = BTreeMap::new();
        for application in &applications {
            let history = self.applications.status_events(&application.id)?;
            if was_applied(application.status, &history) {
                applied.push(application);
                events.insert(application.id.as_str(), history);
            }
        }

        let posting_ids: Vec<String> = applied
            .iter()
            .map(|application| application.posting_id.clone())
            .collect();
        let cards = self.scoring.latest_cards(&posting_ids)?;

        let mut sources = Group::default();
        let mut verdicts = Group::default();
        let mut languages = Group::default();
        let (mut replies, mut interviews) = (0, 0);

        for application in &applied {
            let history = events
                .get(application.id.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let replied = reached(application.status, history, &REPLY_STATUSES);
            let interviewed = reached(application.status, history, &INTERVIEW_STATUSES);
            replies += u32::from(replied);
            interviews += u32::from(interviewed);

            let offer = self.ingestion.get_offer(&application.posting_id)?;
            let source_id = offer
                .as_ref()
                .map(|offer| offer.source_id.clone())
                .unwrap_or_else(|| UNKNOWN.to_string());
            let language = offer
                .as_ref()
                .map(|offer| offer.lang.clone())
                .filter(|lang| !lang.trim().is_empty())
                .unwrap_or_else(|| UNKNOWN.to_string());
            let verdict = cards
                .get(&application.posting_id)
                .map(|card| card.card.verdict.as_str().to_string())
                .unwrap_or_else(|| UNSCORED.to_string());

            sources.add(source_id, replied, interviewed);
            verdicts.add(verdict, replied, interviewed);
            languages.add(language, replied, interviewed);
        }

        let applied_count = applied.len() as u32;
        Ok(FunnelStats {
            applications: applications.len() as u32,
            applied: applied_count,
            replies,
            interviews,
            reply_rate: rate(replies, applied_count),
            interview_rate: rate(interviews, applied_count),
            by_stage: Stage::ALL
                .iter()
                .map(|stage| StageCount {
                    stage: stage.as_str().to_string(),
                    count: by_stage.get(stage.as_str()).copied().unwrap_or(0),
                })
                .collect(),
            by_source: sources.rows(|key| source_names.get(key).cloned(), None),
            by_verdict: verdicts.rows(|_| None, Some(&BAND_ORDER)),
            by_language: languages.rows(|_| None, None),
        })
    }
}

const UNSCORED: &str = "unscored";
const UNKNOWN: &str = "unknown";

const REPLY_STATUSES: [AppStatus; 5] = [
    AppStatus::Interview,
    AppStatus::Offer,
    AppStatus::Hired,
    AppStatus::Rejected,
    AppStatus::OfferDeclined,
];

const INTERVIEW_STATUSES: [AppStatus; 4] = [
    AppStatus::Interview,
    AppStatus::Offer,
    AppStatus::Hired,
    AppStatus::OfferDeclined,
];

/// Statuses that can only follow `applied`: a row in one of them entered the
/// funnel even when an imported history is missing its `applied` event.
const PAST_APPLIED: [AppStatus; 6] = [
    AppStatus::Applied,
    AppStatus::Interview,
    AppStatus::Offer,
    AppStatus::Hired,
    AppStatus::Rejected,
    AppStatus::NoResponse,
];

/// Whether the Application was ever applied to (FR-97's denominator).
fn was_applied(status: AppStatus, history: &[StatusEvent]) -> bool {
    reached(status, history, &PAST_APPLIED)
}

/// Whether the status is in `history`, or is the current status.
fn reached(status: AppStatus, history: &[StatusEvent], targets: &[AppStatus]) -> bool {
    targets.contains(&status)
        || history
            .iter()
            .any(|event| targets.contains(&event.to_status))
}

fn rate(part: u32, whole: u32) -> f64 {
    if whole == 0 {
        0.0
    } else {
        f64::from(part) / f64::from(whole)
    }
}

/// Counts one breakdown's groups before the rates are known.
#[derive(Default)]
struct Group {
    counts: BTreeMap<String, u32>,
    replies: BTreeMap<String, u32>,
    interviews: BTreeMap<String, u32>,
}

impl Group {
    fn add(&mut self, key: String, replied: bool, interviewed: bool) {
        *self.counts.entry(key.clone()).or_insert(0) += 1;
        if replied {
            *self.replies.entry(key.clone()).or_insert(0) += 1;
        }
        if interviewed {
            *self.interviews.entry(key).or_insert(0) += 1;
        }
    }

    /// Rows with stable ordering. With `bands`, the named keys keep that order
    /// (a Verdict band list); otherwise the biggest group comes first, then the
    /// key.
    fn rows(&self, label: impl Fn(&str) -> Option<String>, bands: Option<&[&str]>) -> Vec<RateRow> {
        let mut rows: Vec<RateRow> = self
            .counts
            .iter()
            .map(|(key, count)| {
                let replies = self.replies.get(key).copied().unwrap_or(0);
                let interviews = self.interviews.get(key).copied().unwrap_or(0);
                RateRow {
                    key: key.clone(),
                    label: label(key),
                    applications: *count,
                    replies,
                    interviews,
                    reply_rate: rate(replies, *count),
                    interview_rate: rate(interviews, *count),
                }
            })
            .collect();
        match bands {
            Some(bands) => {
                let slot = |key: &str| {
                    bands
                        .iter()
                        .position(|band| *band == key)
                        .unwrap_or(bands.len())
                };
                rows.sort_by(|a, b| slot(&a.key).cmp(&slot(&b.key)).then(a.key.cmp(&b.key)));
            }
            None => {
                rows.sort_by(|a, b| b.applications.cmp(&a.applications).then(a.key.cmp(&b.key)))
            }
        }
        rows
    }
}

/// Verdict bands in score order; `unscored` (an Offer with no card) last.
const BAND_ORDER: [&str; 6] = ["strong", "good", "moderate", "weak", "poor", UNSCORED];

#[cfg(test)]
pub mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::application::{StatusSource, new_status_event_id};
    use boba_domain::posting::{ApplyHints, Offer, WorkMode};
    use boba_domain::scoring::gates::{Confidence, GateReport};
    use boba_domain::scoring::rank::{CardVersions, ScoreCard, ScoreCardRow, Verdict};
    use boba_domain::source::{AccessClass, ApplyCapability, LegalTier, SourceRecord};

    use crate::applications::testing::FakeApplyRepo;
    use crate::ingest::manual::testing::FakeSourcesRepo;
    use crate::ingest::normalize::testing::FakeIngestionRepo;
    use crate::ports::repo::ingestion::IngestionRepo;
    use crate::ports::repo::scoring::ScoringRepo;
    use crate::ports::repo::sources::SourcesRepo;
    use crate::scoring::testing::FakeScoringRepo;

    const NOW: &str = "2026-10-17T08:00:00.000Z";

    fn offer(id: &str, source_id: &str, lang: &str) -> Offer {
        Offer {
            id: id.to_string(),
            source_id: source_id.to_string(),
            external_id: format!("ext-{id}"),
            title: "Backend Engineer".to_string(),
            company: "Nimbus".to_string(),
            locations: Vec::new(),
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: NOW.to_string(),
            last_seen: NOW.to_string(),
            lang: lang.to_string(),
            description_html: String::new(),
            description_text: String::new(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://jobs.example.com/1".to_string(),
                email: None,
            },
        }
    }

    fn provenance(posting_id: &str, source_id: &str) -> boba_domain::posting::Provenance {
        boba_domain::posting::Provenance {
            posting_id: posting_id.to_string(),
            source_id: source_id.to_string(),
            external_id: format!("ext-{posting_id}"),
            url: "https://jobs.example.com/1".to_string(),
            raw: "{}".to_string(),
            fetched_at: NOW.to_string(),
        }
    }

    fn source(id: &str, name: &str) -> SourceRecord {
        SourceRecord {
            id: id.to_string(),
            name: name.to_string(),
            access_class: AccessClass::PublicJson,
            legal_tier: LegalTier::Safe,
            apply_capability: ApplyCapability::LinkOut,
            attribution: None,
            min_interval_ms: 1_000,
            daily_cap: None,
            enabled: true,
            added_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        }
    }

    fn application(id: &str, posting_id: &str, status: AppStatus) -> Application {
        Application {
            id: id.to_string(),
            posting_id: posting_id.to_string(),
            status,
            offer_snapshot_json: "{}".to_string(),
            notes: None,
            next_action: None,
            created_at: NOW.to_string(),
            updated_at: NOW.to_string(),
        }
    }

    fn event(application_id: &str, from: Option<AppStatus>, to: AppStatus) -> StatusEvent {
        StatusEvent {
            id: new_status_event_id(),
            application_id: application_id.to_string(),
            from_status: from,
            to_status: to,
            source: StatusSource::User,
            reason: None,
            created_at: NOW.to_string(),
        }
    }

    fn card(posting_id: &str, verdict: Verdict) -> ScoreCardRow {
        ScoreCardRow::new(
            format!("sc_{posting_id}"),
            posting_id,
            CardVersions {
                profile_version: 1,
                prefs_version: 1,
                model_version: 1,
            },
            ScoreCard {
                model_version: 1,
                factors: Vec::new(),
                fit: Some(70.0),
                quality: Some(70.0),
                rank: 70.0,
                verdict,
                confidence: Confidence::High,
            },
            GateReport { results: vec![] },
            NOW,
        )
    }

    struct Fixture {
        apply: FakeApplyRepo,
        ingest: FakeIngestionRepo,
        scoring: FakeScoringRepo,
        sources: FakeSourcesRepo,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                apply: FakeApplyRepo::default(),
                ingest: FakeIngestionRepo::default(),
                scoring: FakeScoringRepo::default(),
                sources: FakeSourcesRepo::default(),
            }
        }

        /// The service takes the fakes; seed everything before calling this.
        fn service(self) -> StatsService {
            StatsService::new(
                Arc::new(self.apply),
                Arc::new(self.ingest),
                Arc::new(self.scoring),
                Arc::new(self.sources),
            )
        }
    }

    // AC-2: Applications by Stage covers every board column, zeros included.
    #[test]
    fn applications_by_stage_counts_every_column() {
        let fixture = Fixture::new();
        fixture
            .apply
            .seed_application(application("app_1", "pst_1", AppStatus::Saved));
        fixture
            .apply
            .seed_application(application("app_2", "pst_2", AppStatus::Applied));
        fixture
            .apply
            .seed_application(application("app_3", "pst_3", AppStatus::Interview));

        let stats = fixture.service().funnel().unwrap();
        assert_eq!(stats.applications, 3);
        assert_eq!(stats.by_stage.len(), Stage::ALL.len());
        let saved = stats
            .by_stage
            .iter()
            .find(|row| row.stage == "saved")
            .unwrap();
        assert_eq!(saved.count, 1);
        let interview = stats
            .by_stage
            .iter()
            .find(|row| row.stage == "interviewing")
            .unwrap();
        assert_eq!(interview.count, 1);
        let hired = stats
            .by_stage
            .iter()
            .find(|row| row.stage == "hired")
            .unwrap();
        assert_eq!(hired.count, 0);
    }

    // AC-2: the rates count applied Applications; a rejection is a reply, an
    // interview (past or present) is an interview, and history is kept.
    #[test]
    fn rates_follow_the_history_not_just_the_current_status() {
        let fixture = Fixture::new();
        seed_offer(&fixture, "pst_1", "src_a", "en");

        // app_1: applied, then rejected → a reply, no interview.
        fixture
            .apply
            .seed_application(application("app_1", "pst_1", AppStatus::Rejected));
        fixture
            .apply
            .append_status_event(&event("app_1", None, AppStatus::Applied))
            .unwrap();
        fixture
            .apply
            .append_status_event(&event(
                "app_1",
                Some(AppStatus::Applied),
                AppStatus::Rejected,
            ))
            .unwrap();

        // app_2: applied, interviewed, then withdrawn → still counted as an
        // interview even though the current status says Withdrawn.
        seed_offer(&fixture, "pst_2", "src_a", "en");
        fixture
            .apply
            .seed_application(application("app_2", "pst_2", AppStatus::Withdrawn));
        fixture
            .apply
            .append_status_event(&event("app_2", None, AppStatus::Applied))
            .unwrap();
        fixture
            .apply
            .append_status_event(&event(
                "app_2",
                Some(AppStatus::Applied),
                AppStatus::Interview,
            ))
            .unwrap();
        fixture
            .apply
            .append_status_event(&event(
                "app_2",
                Some(AppStatus::Interview),
                AppStatus::Withdrawn,
            ))
            .unwrap();

        // app_3: saved only → outside the funnel.
        fixture
            .apply
            .seed_application(application("app_3", "pst_3", AppStatus::Saved));

        let stats = fixture.service().funnel().unwrap();
        assert_eq!(stats.applied, 2);
        assert_eq!(stats.replies, 2);
        assert_eq!(stats.interviews, 1);
        assert_eq!(stats.reply_rate, 1.0);
        assert_eq!(stats.interview_rate, 0.5);
    }

    // AC-2: each Source, Verdict and language is its own row, with the Source
    // name on the label.
    #[test]
    fn breakdowns_group_by_source_verdict_and_language() {
        let fixture = Fixture::new();
        fixture
            .sources
            .upsert_source(&source("src_a", "Greenhouse"))
            .unwrap();
        fixture
            .sources
            .upsert_source(&source("src_b", "Remotive"))
            .unwrap();
        seed_offer(&fixture, "pst_1", "src_a", "en");
        seed_offer(&fixture, "pst_2", "src_a", "es");
        seed_offer(&fixture, "pst_3", "src_b", "en");
        fixture
            .scoring
            .insert_card(&card("pst_1", Verdict::Strong))
            .unwrap();
        fixture
            .scoring
            .insert_card(&card("pst_2", Verdict::Moderate))
            .unwrap();
        // pst_3 has no card: it lands in `unscored`, never in a band.

        for (id, posting, status) in [
            ("app_1", "pst_1", AppStatus::Interview),
            ("app_2", "pst_2", AppStatus::Applied),
            ("app_3", "pst_3", AppStatus::Rejected),
        ] {
            fixture
                .apply
                .seed_application(application(id, posting, status));
            fixture
                .apply
                .append_status_event(&event(id, None, AppStatus::Applied))
                .unwrap();
        }
        fixture
            .apply
            .append_status_event(&event(
                "app_1",
                Some(AppStatus::Applied),
                AppStatus::Interview,
            ))
            .unwrap();
        fixture
            .apply
            .append_status_event(&event(
                "app_3",
                Some(AppStatus::Applied),
                AppStatus::Rejected,
            ))
            .unwrap();

        let stats = fixture.service().funnel().unwrap();

        let by_source: Vec<(&str, u32, u32, u32)> = stats
            .by_source
            .iter()
            .map(|row| {
                (
                    row.key.as_str(),
                    row.applications,
                    row.replies,
                    row.interviews,
                )
            })
            .collect();
        assert_eq!(
            by_source,
            vec![("src_a", 2, 1, 1), ("src_b", 1, 1, 0)],
            "groups sort by size, then key"
        );
        assert_eq!(stats.by_source[0].label.as_deref(), Some("Greenhouse"));

        let by_verdict: Vec<(&str, u32)> = stats
            .by_verdict
            .iter()
            .map(|row| (row.key.as_str(), row.applications))
            .collect();
        assert_eq!(
            by_verdict,
            vec![("strong", 1), ("moderate", 1), ("unscored", 1)],
            "bands keep their order, `unscored` last"
        );

        let by_language: Vec<(&str, u32)> = stats
            .by_language
            .iter()
            .map(|row| (row.key.as_str(), row.applications))
            .collect();
        assert_eq!(by_language, vec![("en", 2), ("es", 1)]);
    }

    // A posting the store no longer has (or an unknown Source) still shows a
    // row instead of dropping the Application from the breakdown.
    #[test]
    fn missing_data_falls_into_unknown_buckets() {
        let fixture = Fixture::new();
        fixture
            .apply
            .seed_application(application("app_1", "pst_gone", AppStatus::Applied));
        fixture
            .apply
            .append_status_event(&event("app_1", None, AppStatus::Applied))
            .unwrap();

        let stats = fixture.service().funnel().unwrap();
        assert_eq!(stats.by_source[0].key, "unknown");
        assert_eq!(stats.by_language[0].key, "unknown");
        assert_eq!(stats.by_verdict[0].key, "unscored");
    }

    // An empty board renders zeros rather than an error.
    #[test]
    fn an_empty_store_is_all_zeros() {
        let fixture = Fixture::new();
        let stats = fixture.service().funnel().unwrap();
        assert_eq!(stats.applications, 0);
        assert_eq!(stats.applied, 0);
        assert_eq!(stats.reply_rate, 0.0);
        assert_eq!(stats.interview_rate, 0.0);
        assert!(stats.by_source.is_empty());
        assert_eq!(stats.by_stage.len(), Stage::ALL.len());
    }

    fn seed_offer(fixture: &Fixture, posting_id: &str, source_id: &str, lang: &str) {
        fixture
            .ingest
            .insert_offer(
                &offer(posting_id, source_id, lang),
                &provenance(posting_id, source_id),
            )
            .unwrap();
    }
}
