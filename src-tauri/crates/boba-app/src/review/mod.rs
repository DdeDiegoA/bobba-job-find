//! Reviewer pass and revision (S7.4, FR-22/FR-72).
//!
//! Generation runs three steps per application: `tailor_application` drafts, a
//! **fresh-context** `review_application` run critiques the drafts, and the
//! tailor runs once more with the critique (architecture §5.5). This module owns
//! what the core must guarantee about steps two and three:
//!
//! * **the critique** — what the review run writes with `save_review` is parsed
//!   fail-closed, ranked, and stored on the drafts it judged, so the studio can
//!   show the reviewer's top findings and still show them after a reload
//!   ([`save_critique`], [`Review::top_findings`]);
//! * **the inputs** — the review run is handed the job and the drafts and
//!   nothing else (fresh context, FR-22), and the revision run is handed the
//!   same job, the drafts it must revise, and the findings it must fix;
//! * **the loop** — a revision follows only while there is something to fix and
//!   the iteration cap is not spent, so a review can never loop.
//!
//! The critique is stored **as document data**: it lands on the `document` rows
//! it judged ([`DocumentsRepo::set_review`]), which are the rows the studio reads
//! the drafts from, so this slice adds no table of critiques. It carries its own
//! `untrusted_origin` marker (migration `0014`), because it stays agent text even
//! when the user edits the draft underneath it (ADR-010).
//!
//! The revision itself is an ordinary next draft: `tailoring::save_drafts` writes
//! it as a new document version of the same application, untrusted like the first
//! drafts. Running the three steps end to end is S7.7's.
//!
//! Findings are plain data, never markup: the studio renders their text as
//! text (ADR-010).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use crate::tailoring::Drafts;

/// How many of the reviewer's findings the studio shows (FR-72: the *top*
/// findings, not the whole list).
pub const STUDIO_FINDINGS: usize = 3;

/// How many times one application may be revised. The tailoring flow is capped
/// so a review that keeps finding something can never loop (architecture §5.5).
pub const MAX_REVISIONS: u32 = 1;

/// The longest text one finding may carry, in characters. A message or a
/// suggestion is a sentence or two; anything longer is a runaway answer that
/// would bloat the studio payload and the revision prompt, so it is refused
/// rather than stored. Far above a real finding, so nothing honest is lost.
pub const MAX_FINDING_TEXT: usize = 500;

/// How serious a finding is; the studio ranks by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    High,
    Medium,
    Low,
}

impl Severity {
    /// Every severity, most serious first. This order **is** the ranking.
    pub const ALL: [Severity; 3] = [Severity::High, Severity::Medium, Severity::Low];

    /// The value the agent writes.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
        }
    }

    /// Read the agent's value back. `None` for anything else: an unknown
    /// severity would be ranked nowhere, so it is refused rather than guessed.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

/// Which of the two drafts a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FindingTarget {
    Cv,
    CoverLetter,
}

impl FindingTarget {
    pub const fn as_str(self) -> &'static str {
        match self {
            FindingTarget::Cv => "cv",
            FindingTarget::CoverLetter => "cover_letter",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "cv" => Some(FindingTarget::Cv),
            "cover_letter" => Some(FindingTarget::CoverLetter),
            _ => None,
        }
    }
}

/// One thing the fresh-context reviewer wants changed. `message` says what is
/// wrong, `suggestion` what to do instead; both are the reviewer's words and are
/// shown as data only (ADR-010). A finding is not a claim: it never adds a fact
/// to a draft, it asks for a change that the revision run then has to make from
/// the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Finding {
    pub severity: Severity,
    pub target: FindingTarget,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// The critique one review run wrote for one application's drafts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub application_id: String,
    /// The reviewer's findings, in the order it wrote them; [`Self::top_findings`]
    /// is what ranks them.
    pub findings: Vec<Finding>,
}

impl Review {
    /// Parse the arguments of a `save_review` call.
    ///
    /// Everything is checked before any of it is used: a payload the studio
    /// cannot rank or render is refused whole, so half a critique is never shown
    /// (the run's output goes back to the agent, which retries, and fails
    /// visibly if it cannot fix it). Unknown fields are refused too — an
    /// invented field is a payload we do not understand, not one to skip past.
    pub fn parse(args: &serde_json::Value) -> Result<Self, ReviewError> {
        let object = args
            .as_object()
            .ok_or_else(|| malformed("the arguments must be an object"))?;
        for key in object.keys() {
            if key != "app_id" && key != "findings" {
                return Err(malformed(format!("unknown field `{key}`")));
            }
        }
        let application_id = object
            .get("app_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| malformed("`app_id` must be a non-empty string"))?
            .to_string();
        let raw = object
            .get("findings")
            .and_then(|value| value.as_array())
            .ok_or_else(|| malformed("`findings` must be an array"))?;
        let findings = raw
            .iter()
            .enumerate()
            .map(|(index, finding)| parse_finding(index, finding))
            .collect::<Result<Vec<Finding>, ReviewError>>()?;
        Ok(Self {
            application_id,
            findings,
        })
    }

    /// The findings the studio shows, most serious first, at most `limit` of
    /// them. Findings of the same severity keep the reviewer's order, so the
    /// panel is stable between reads.
    pub fn top_findings(&self, limit: usize) -> Vec<Finding> {
        let mut top = Vec::new();
        for severity in Severity::ALL {
            for finding in self.findings.iter().filter(|f| f.severity == severity) {
                if top.len() == limit {
                    return top;
                }
                top.push(finding.clone());
            }
        }
        top
    }

    /// Whether anything is worth revising: a reviewer that found nothing is a
    /// draft that goes straight to the studio.
    pub fn needs_revision(&self) -> bool {
        !self.findings.is_empty()
    }
}

fn parse_finding(index: usize, value: &serde_json::Value) -> Result<Finding, ReviewError> {
    let at = |field: &str| format!("`findings[{index}].{field}`");
    let object = value
        .as_object()
        .ok_or_else(|| malformed(format!("{} must be an object", at(""))))?;
    for key in object.keys() {
        if !["severity", "target", "message", "suggestion"].contains(&key.as_str()) {
            return Err(malformed(format!(
                "{} is not a field a finding has",
                at(key)
            )));
        }
    }
    let severity = object
        .get("severity")
        .and_then(|value| value.as_str())
        .and_then(Severity::parse)
        .ok_or_else(|| {
            malformed(format!(
                "{} must be `high`, `medium` or `low`",
                at("severity")
            ))
        })?;
    let target = object
        .get("target")
        .and_then(|value| value.as_str())
        .and_then(FindingTarget::parse)
        .ok_or_else(|| malformed(format!("{} must be `cv` or `cover_letter`", at("target"))))?;
    let message = text(object.get("message"), &at("message"))?;
    // `null` is how a JSON writer says "no suggestion": the skill tells the agent
    // to omit the field, and an agent that sends `null` means the same thing.
    let suggestion = match object.get("suggestion") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(text(Some(value), &at("suggestion"))?),
    };
    Ok(Finding {
        severity,
        target,
        message,
        suggestion,
    })
}

/// A finding's own words: a non-empty string, and no longer than
/// [`MAX_FINDING_TEXT`].
fn text(value: Option<&serde_json::Value>, at: &str) -> Result<String, ReviewError> {
    let text = value
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| malformed(format!("{at} must be a non-empty string")))?;
    if text.chars().count() > MAX_FINDING_TEXT {
        return Err(malformed(format!(
            "{at} is longer than {MAX_FINDING_TEXT} characters"
        )));
    }
    Ok(text.to_string())
}

/// Why a critique could not be stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReviewError {
    /// The agent's payload is not a critique we can show or rank. Refused whole,
    /// so the run retries instead of half-showing one.
    #[error("this critique is not usable: {0}")]
    Critique(String),
    /// The application has no drafts, so there was nothing to review.
    #[error("`{0}` has no drafts to review")]
    NothingToReview(String),
    /// The store refused the write.
    #[error("document store error: {0}")]
    Repo(#[from] DocumentsRepoError),
}

/// The input of the fresh-context review run (architecture §5.5 step 3, FR-22):
/// the job and the drafts to critique, and nothing else. A review that shared
/// the drafting run's context would not be fresh, so what it gets is this
/// payload and its own tool calls.
pub fn review_input(application_id: &str, posting_id: &str, drafts: &Drafts) -> String {
    job(application_id, posting_id, drafts).to_string()
}

/// The input of the revision run (architecture §5.5 step 4): the same job, the
/// drafts to revise, and **every** finding to fix — not only the studio's top
/// ones. The core composes it, so the revision sees the core's payload and the
/// profile, never a previous run's conversation.
pub fn revision_input(
    application_id: &str,
    posting_id: &str,
    drafts: &Drafts,
    review: &Review,
) -> String {
    let mut input = job(application_id, posting_id, drafts);
    input["review_findings"] =
        serde_json::to_value(&review.findings).expect("findings are plain serializable data");
    input.to_string()
}

/// The job both the review and the revision run start from: which application,
/// which Offer, and the drafts as they stand.
fn job(application_id: &str, posting_id: &str, drafts: &Drafts) -> serde_json::Value {
    serde_json::json!({
        "app_id": application_id,
        "posting_id": posting_id,
        "drafts": {"cv": drafts.cv, "cover_letter": drafts.cover},
    })
}

/// What the core does with a critique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Run `tailor_application` once more, with these findings.
    Revise,
    /// Nothing left to fix, or no revision left to spend: the drafts and the
    /// findings go to the studio as they are.
    Done,
}

/// One application's review→revise loop (architecture §5.5 step 3–4). One pass
/// per generation run of one application: the caller builds it when generation
/// starts and drops it when the run ends.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReviewPass {
    revisions: u32,
}

impl ReviewPass {
    pub fn new() -> Self {
        Self::default()
    }

    /// Revisions this pass has already asked for.
    pub fn revisions(&self) -> u32 {
        self.revisions
    }

    /// Whether to revise. A critique with findings is revised once; after that
    /// the loop is capped and the critique is shown instead, however serious the
    /// next findings are.
    pub fn step(&self, review: &Review) -> Step {
        if review.needs_revision() && self.revisions < MAX_REVISIONS {
            Step::Revise
        } else {
            Step::Done
        }
    }

    /// Record that the revision the pass asked for has been stored.
    pub fn revised(&mut self) {
        self.revisions += 1;
    }
}

/// A payload the core cannot use, with the field that broke it.
fn malformed(why: impl Into<String>) -> ReviewError {
    ReviewError::Critique(why.into())
}

/// Store the critique a review run wrote with `save_review` (S7.4, FR-22/FR-72).
///
/// The findings are attached to the drafts the reviewer judged, as document
/// data: those rows already carry `untrusted_origin` (S7.2, ADR-010), so this
/// slice needs no table of critiques. An application with no drafts is refused —
/// a critique of nothing is worse than a failed run — and a payload the core
/// cannot use is refused **whole**, before anything is written, so the run
/// retries instead of the studio half-showing a critique.
///
/// `application_id` is the application the **core** started this run for, and
/// the payload's own `app_id` is a checked echo of it: a review run started for
/// one application cannot annotate another, and the drafts that are annotated
/// are the run's own.
pub fn save_critique(
    repo: &dyn DocumentsRepo,
    application_id: &str,
    args: &serde_json::Value,
    updated_at: &str,
) -> Result<Review, ReviewError> {
    let review = Review::parse(args)?;
    if review.application_id != application_id {
        return Err(malformed(format!(
            "`app_id` names `{}`, but this run reviews `{application_id}`",
            review.application_id
        )));
    }
    // The parsed findings, not the agent's raw payload: only fields the core
    // understands are stored.
    let findings =
        serde_json::to_string(&review.findings).expect("findings are plain serializable data");
    if repo.set_review(application_id, &findings, updated_at)? == 0 {
        return Err(ReviewError::NothingToReview(application_id.to_string()));
    }
    Ok(review)
}

#[cfg(test)]
mod tests {
    use boba_domain::documents::{
        Claim, Contact, CoverLetterDocument, CvDocument, Lang, SCHEMA_VERSION, Section, TemplateId,
    };
    use serde_json::json;

    use super::*;
    use crate::ports::repo::documents::fake::FakeDocumentsRepo;
    use crate::tailoring::{self, DraftTemplates, Drafts};

    fn templates() -> DraftTemplates {
        DraftTemplates {
            cv: TemplateId::new("cv-classic"),
            cover: TemplateId::new("cover-classic"),
        }
    }

    /// Store both drafts of one application, as a tailoring run leaves them.
    fn drafts_for(repo: &FakeDocumentsRepo, application_id: &str) {
        tailoring::save_drafts(
            repo,
            application_id,
            &Lang::for_offer("en", None),
            &templates(),
            &drafts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("the drafts are stored");
    }

    /// The reason a payload was refused, for assertions.
    fn why(error: &ReviewError) -> String {
        match error {
            ReviewError::Critique(why) => why.clone(),
            other => panic!("expected a refused payload, got {other}"),
        }
    }

    /// A drafted CV and cover letter, as the tailor run leaves them.
    fn drafts() -> Drafts {
        Drafts {
            cv: CvDocument {
                schema_version: SCHEMA_VERSION,
                lang: Lang::En,
                contact: Contact {
                    full_name: "Ada Lovelace".into(),
                    headline: None,
                    email: Some("ada@example.com".into()),
                    phone: None,
                    location: None,
                    links: vec![],
                },
                summary: Some(Claim {
                    text: "Six years of backend work.".into(),
                    fact_ids: vec!["fact_3".into()],
                }),
                sections: vec![Section {
                    heading: "Experience".into(),
                    items: vec![Claim {
                        text: "Shipped an API".into(),
                        fact_ids: vec!["fact_1".into()],
                    }],
                }],
                declared_gaps: vec!["Kubernetes".into()],
            },
            cover: CoverLetterDocument {
                schema_version: SCHEMA_VERSION,
                lang: Lang::En,
                contact: Contact {
                    full_name: "Ada Lovelace".into(),
                    headline: None,
                    email: None,
                    phone: None,
                    location: None,
                    links: vec![],
                },
                company: Some("Acme".into()),
                recipient: None,
                date: None,
                salutation: None,
                paragraphs: vec![Claim {
                    text: "I would like to apply.".into(),
                    fact_ids: vec!["fact_2".into()],
                }],
                closing: None,
                declared_gaps: vec![],
            },
        }
    }

    // AC1, step 2: the critique the review run writes is parsed as written, in
    // the reviewer's own order.
    #[test]
    fn a_valid_critique_parses_and_keeps_the_reviewers_order() {
        let review = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [
                {
                    "severity": "medium",
                    "target": "cv",
                    "message": "The summary repeats the first bullet.",
                    "suggestion": "Keep the strongest one.",
                },
                {
                    "severity": "high",
                    "target": "cover_letter",
                    "message": "The closing promises relocating in March, which no fact supports.",
                },
                {"severity": "low", "target": "cv", "message": "Two typos."},
            ],
        }))
        .expect("a valid critique");

        assert_eq!(review.application_id, "app_1");
        assert_eq!(review.findings.len(), 3);
        assert_eq!(review.findings[0].severity, Severity::Medium);
        assert_eq!(review.findings[0].target, FindingTarget::Cv);
        assert_eq!(
            review.findings[0].suggestion.as_deref(),
            Some("Keep the strongest one.")
        );
        assert_eq!(review.findings[1].severity, Severity::High);
        assert_eq!(review.findings[1].target, FindingTarget::CoverLetter);
        assert_eq!(review.findings[1].suggestion, None);
        assert_eq!(review.findings[2].severity, Severity::Low);
        assert!(review.needs_revision());
    }

    /// A reviewer that found nothing is a valid critique, and nothing to revise.
    #[test]
    fn a_critique_with_no_findings_is_valid_and_needs_no_revision() {
        let review =
            Review::parse(&json!({"app_id": "app_1", "findings": []})).expect("an empty critique");
        assert!(review.findings.is_empty());
        assert!(!review.needs_revision());
    }

    /// Fail closed: nothing partial is shown in the studio, and an unknown value
    /// is refused rather than ranked somewhere arbitrary.
    #[test]
    fn a_payload_the_studio_cannot_rank_is_refused() {
        let cases = [
            json!("app_1"),
            json!({"findings": []}),
            json!({"app_id": "", "findings": []}),
            json!({"app_id": "app_1"}),
            json!({"app_id": "app_1", "findings": {}}),
            json!({"app_id": "app_1", "findings": ["fix it"]}),
            json!({"app_id": "app_1", "note": "hello", "findings": []}),
            json!({"app_id": "app_1", "findings": [{"severity": "urgent", "target": "cv", "message": "x"}]}),
            json!({"app_id": "app_1", "findings": [{"severity": "high", "target": "resume", "message": "x"}]}),
            json!({"app_id": "app_1", "findings": [{"severity": "high", "target": "cv"}]}),
            json!({"app_id": "app_1", "findings": [{"severity": "high", "target": "cv", "message": "   "}]}),
            json!({"app_id": "app_1", "findings": [{"severity": "high", "target": "cv", "message": "x", "suggestion": 7}]}),
            json!({"app_id": "app_1", "findings": [{"severity": "high", "target": "cv", "message": "x", "fixed": true}]}),
        ];
        for (index, args) in cases.iter().enumerate() {
            let error =
                Review::parse(args).expect_err(&format!("case {index} must be refused: {args}"));
            assert!(
                error
                    .to_string()
                    .starts_with("this critique is not usable:")
            );
        }
        // The refusals name what to fix, so the agent can.
        let unknown_severity = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [{"severity": "urgent", "target": "cv", "message": "x"}],
        }))
        .expect_err("unknown severity");
        assert!(
            why(&unknown_severity).contains("findings[0].severity"),
            "{unknown_severity}"
        );
        let missing_app = Review::parse(&json!({"findings": []})).expect_err("no app id");
        assert!(why(&missing_app).contains("app_id"), "{missing_app}");
    }

    /// AC1: the *top* findings are what the studio shows — the most serious
    /// ones, and only as many as the panel has room for.
    #[test]
    fn the_studio_gets_the_most_serious_findings_first() {
        let review = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [
                {"severity": "low", "target": "cv", "message": "typo one"},
                {"severity": "high", "target": "cv", "message": "unsupported claim"},
                {"severity": "medium", "target": "cover_letter", "message": "too long"},
                {"severity": "high", "target": "cover_letter", "message": "invented fact"},
                {"severity": "low", "target": "cv", "message": "typo two"},
            ],
        }))
        .expect("a valid critique");

        let top = review.top_findings(STUDIO_FINDINGS);
        assert_eq!(top.len(), STUDIO_FINDINGS);
        let messages: Vec<&str> = top.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(
            messages,
            vec!["unsupported claim", "invented fact", "too long"],
            "serious first, and the reviewer's own order within a severity"
        );
        assert_eq!(top[0].severity, Severity::High);
        assert_eq!(top[2].severity, Severity::Medium);

        // A panel with room for all of them gets all of them, still ranked;
        // the review itself is untouched.
        let all: Vec<Severity> = review.top_findings(9).iter().map(|f| f.severity).collect();
        assert_eq!(
            all,
            vec![
                Severity::High,
                Severity::High,
                Severity::Medium,
                Severity::Low,
                Severity::Low
            ]
        );
        assert_eq!(review.findings[0].message, "typo one");
        assert!(review.top_findings(0).is_empty());
        assert!(review.top_findings(STUDIO_FINDINGS).len() <= STUDIO_FINDINGS);
    }

    /// AC1: a revision follows the critique — once. A second critique of the
    /// revision is shown to the user, not revised again.
    #[test]
    fn a_revision_follows_a_critique_and_the_loop_is_capped() {
        let critique = |message: &str| {
            Review::parse(&json!({
                "app_id": "app_1",
                "findings": [{"severity": "high", "target": "cv", "message": message}],
            }))
            .expect("a valid critique")
        };

        let mut pass = ReviewPass::new();
        assert_eq!(pass.step(&critique("fix this")), Step::Revise);
        assert_eq!(pass.revisions(), 0, "nothing revised yet");

        pass.revised();
        assert_eq!(pass.revisions(), 1);
        assert_eq!(
            pass.step(&critique("still wrong")),
            Step::Done,
            "the cap is spent: the studio shows the critique instead of looping"
        );
        assert_eq!(MAX_REVISIONS, 1);
    }

    /// A critique that found nothing revises nothing and spends nothing.
    #[test]
    fn a_clean_critique_goes_straight_to_the_studio() {
        let review =
            Review::parse(&json!({"app_id": "app_1", "findings": []})).expect("an empty critique");
        let pass = ReviewPass::new();
        assert_eq!(pass.step(&review), Step::Done);
        assert_eq!(pass.revisions(), 0);
        // And a critique that *does* have findings still gets its revision:
        // the empty one asked for nothing, so it spent nothing.
        let finding = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [{"severity": "low", "target": "cv", "message": "typo"}],
        }))
        .expect("a valid critique");
        assert_eq!(
            pass.step(&finding),
            Step::Revise,
            "an empty critique must not spend the budget"
        );
    }

    /// AC1: generation runs draft → review → revise. The review run is handed
    /// the job and the drafts and **nothing else** — no drafting transcript, no
    /// findings — so the critique comes from a clean context (FR-22).
    #[test]
    fn the_review_run_gets_the_job_and_the_drafts_and_nothing_else() {
        let input: serde_json::Value =
            serde_json::from_str(&review_input("app_1", "pst_1", &drafts())).expect("JSON");

        assert_eq!(input["app_id"], "app_1");
        assert_eq!(input["posting_id"], "pst_1");
        assert_eq!(
            input["drafts"]["cv"]["contact"]["full_name"],
            "Ada Lovelace"
        );
        assert_eq!(
            input["drafts"]["cv"]["sections"][0]["items"][0]["fact_ids"][0],
            "fact_1"
        );
        assert_eq!(input["drafts"]["cover_letter"]["company"], "Acme");
        assert_eq!(
            input.as_object().expect("an object").len(),
            3,
            "the job and the drafts, and nothing else: {input}"
        );
    }

    /// §5.5 step 4: the revision run is a tailoring run again, and it gets the drafts it must
    /// revise plus every finding it must fix — not just the studio's top ones.
    #[test]
    fn the_revision_run_gets_the_drafts_and_the_findings() {
        let review = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [
                {"severity": "high", "target": "cv", "message": "unsupported claim", "suggestion": "drop it"},
                {"severity": "low", "target": "cover_letter", "message": "typo"},
            ],
        }))
        .expect("a valid critique");

        let input: serde_json::Value =
            serde_json::from_str(&revision_input("app_1", "pst_1", &drafts(), &review))
                .expect("JSON");

        assert_eq!(input["app_id"], "app_1");
        assert_eq!(input["posting_id"], "pst_1");
        assert_eq!(
            input["drafts"]["cv"]["summary"]["text"],
            "Six years of backend work."
        );
        assert_eq!(
            input["review_findings"].as_array().expect("findings").len(),
            2
        );
        assert_eq!(input["review_findings"][0]["severity"], "high");
        assert_eq!(input["review_findings"][0]["target"], "cv");
        assert_eq!(input["review_findings"][0]["message"], "unsupported claim");
        assert_eq!(input["review_findings"][0]["suggestion"], "drop it");
        assert_eq!(
            input["review_findings"][1]["message"], "typo",
            "the reviewer's order reaches the revision run"
        );
        assert_eq!(
            input.as_object().expect("an object").len(),
            4,
            "no drafting transcript either: {input}"
        );
    }

    /// The slice's storage decision, end to end (slice note, ADR-010): what the
    /// review produces is the revision, and a revision is stored as new document
    /// versions of the same application through `tailoring::save_drafts` — rows
    /// that already carry `untrusted_origin`. No critique table, no new column.
    #[test]
    fn a_revision_is_stored_as_an_untrusted_document_version() {
        let repo = FakeDocumentsRepo::default();
        let language = Lang::for_offer("es", None);
        let drafts = drafts();
        let revised = Drafts {
            cv: boba_domain::documents::CvDocument {
                summary: Some(Claim {
                    text: "Backend developer, six years.".into(),
                    fact_ids: vec!["fact_3".into()],
                }),
                ..drafts.cv.clone()
            },
            cover: drafts.cover.clone(),
        };

        tailoring::save_drafts(
            &repo,
            "app_1",
            &language,
            &templates(),
            &drafts,
            "2026-10-16T09:00:00Z",
        )
        .expect("the first drafts are stored");

        let review = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [{"severity": "high", "target": "cv", "message": "the summary repeats a bullet"}],
        }))
        .expect("a valid critique");
        let mut pass = ReviewPass::new();
        assert_eq!(pass.step(&review), Step::Revise);

        let saved = tailoring::save_drafts(
            &repo,
            "app_1",
            &language,
            &templates(),
            &revised,
            "2026-10-16T09:05:00Z",
        )
        .expect("the revision is stored");
        pass.revised();

        let rows = repo.rows();
        assert_eq!(rows.len(), 4, "both drafts, then both revisions");
        assert!(
            rows.iter().all(|row| row.untrusted_origin),
            "ADR-010: agent text written after reading the posting"
        );
        assert_eq!(rows[2].id, saved.cv_id);
        assert_eq!(rows[2].application_id, "app_1");
        assert_eq!(
            rows[2].lang,
            Lang::Es,
            "the revision keeps the run's language"
        );
        assert!(
            rows[2].model_json.contains("Backend developer, six years."),
            "the stored revision is the revised draft: {}",
            rows[2].model_json
        );
        assert_eq!(pass.revisions(), 1);
    }

    /// The skill tells the agent to omit `suggestion` when it cannot name one; a
    /// JSON `null` says the same thing and must not throw the critique away.
    #[test]
    fn a_null_suggestion_is_the_same_as_no_suggestion() {
        let review = Review::parse(&json!({
            "app_id": "app_1",
            "findings": [
                {"severity": "low", "target": "cv", "message": "x", "suggestion": null},
            ],
        }))
        .expect("null is absent, not invalid");
        assert_eq!(review.findings[0].suggestion, None);
    }

    /// A finding's text reaches the studio payload and comes back in the
    /// revision prompt, so it is bounded: a runaway blob is refused, not stored.
    #[test]
    fn finding_text_is_bounded() {
        let long = "a".repeat(MAX_FINDING_TEXT);
        assert!(
            Review::parse(&json!({
                "app_id": "app_1",
                "findings": [{"severity": "low", "target": "cv", "message": long}],
            }))
            .is_ok(),
            "a finding of exactly the cap is accepted"
        );

        let too_long = "a".repeat(MAX_FINDING_TEXT + 1);
        for field in ["message", "suggestion"] {
            let mut finding = json!({"severity": "low", "target": "cv", "message": "x"});
            finding[field] = json!(too_long);
            let error = Review::parse(&json!({"app_id": "app_1", "findings": [finding]}))
                .expect_err(&format!("an over-long {field} is refused"));
            assert!(why(&error).contains(field), "{error}");
        }

        // The cap counts characters, so a message in any language is measured
        // the same way.
        let accented = "á".repeat(MAX_FINDING_TEXT);
        assert!(
            Review::parse(&json!({
                "app_id": "app_1",
                "findings": [{"severity": "low", "target": "cv", "message": accented}],
            }))
            .is_ok()
        );
    }

    /// `save_review`, at the use-case layer: the critique lands on the drafts
    /// the reviewer judged, as document data the studio can read after a reload.
    #[test]
    fn saving_a_critique_stores_it_on_the_drafts_it_judged() {
        let repo = FakeDocumentsRepo::default();
        drafts_for(&repo, "app_1");
        drafts_for(&repo, "app_2");

        let review = save_critique(
            &repo,
            "app_1",
            &json!({
                "app_id": "app_1",
                "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
            }),
            "2026-10-16T10:00:00Z",
        )
        .expect("the critique is stored");

        assert_eq!(review.application_id, "app_1");
        let stored = repo.reviews();
        assert_eq!(stored.len(), 1, "one critique, on one application");
        assert_eq!(stored[0].0, "app_1");
        let findings: serde_json::Value =
            serde_json::from_str(&stored[0].1).expect("the stored critique is JSON");
        assert_eq!(findings[0]["message"], "unsupported claim");
        assert_eq!(findings[0]["severity"], "high");
        assert_eq!(findings[0]["target"], "cv");
    }

    /// A critique of an application with no drafts is refused: there is nothing
    /// the reviewer could have read, and storing it would be a critique of
    /// nothing.
    /// A critique of an application with no drafts is refused: there is nothing
    /// the reviewer could have read, and storing it would be a critique of
    /// nothing.
    #[test]
    fn a_critique_naming_another_application_is_refused() {
        let repo = FakeDocumentsRepo::default();
        drafts_for(&repo, "app_1");
        drafts_for(&repo, "app_2");

        // The application is the core's, not the payload's: a review run started
        // for one application cannot annotate another (the app_id-binding rule).
        let error = save_critique(
            &repo,
            "app_1",
            &json!({
                "app_id": "app_2",
                "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
            }),
            "t",
        )
        .expect_err("a run may not annotate another application");
        assert!(why(&error).contains("app_2"), "{error}");
        assert!(repo.reviews().is_empty(), "nothing was written");

        // And the run's own application is annotated as usual.
        let review = save_critique(
            &repo,
            "app_1",
            &json!({
                "app_id": "app_1",
                "findings": [{"severity": "high", "target": "cv", "message": "unsupported claim"}],
            }),
            "t",
        )
        .expect("the critique is stored");
        assert_eq!(review.application_id, "app_1");
        assert_eq!(repo.reviews().len(), 1);
    }

    #[test]
    fn a_critique_of_an_application_with_no_drafts_is_refused() {
        let repo = FakeDocumentsRepo::default();
        let error = save_critique(
            &repo,
            "app_unknown",
            &json!({
                "app_id": "app_unknown",
                "findings": [{"severity": "low", "target": "cv", "message": "typo"}],
            }),
            "t",
        )
        .expect_err("no drafts to review");
        assert!(
            matches!(&error, ReviewError::NothingToReview(app) if app == "app_unknown"),
            "{error}"
        );
        assert!(repo.reviews().is_empty(), "nothing was written");
    }

    /// A malformed critique is refused whole: the store is never called, so no
    /// half a critique reaches the studio.
    #[test]
    fn a_malformed_critique_is_refused_whole() {
        let repo = FakeDocumentsRepo::default();
        drafts_for(&repo, "app_1");
        let error = save_critique(
            &repo,
            "app_1",
            &json!({
                "app_id": "app_1",
                "findings": [{"severity": "urgent", "target": "cv", "message": "x"}],
            }),
            "t",
        )
        .expect_err("unknown severity");
        assert!(matches!(error, ReviewError::Critique(_)), "{error}");
        assert!(repo.reviews().is_empty(), "nothing was written");
    }

    /// A store failure reaches the caller instead of looking like a stored
    /// critique.
    #[test]
    fn a_failed_review_write_reaches_the_caller() {
        let repo = FakeDocumentsRepo::failing();
        let error = save_critique(
            &repo,
            "app_1",
            &json!({"app_id": "app_1", "findings": []}),
            "t",
        )
        .expect_err("the store refused the write");
        assert!(matches!(error, ReviewError::Repo(_)), "{error}");
    }

    /// The two enums round-trip exactly the values the agent writes.
    #[test]
    fn severities_and_targets_round_trip() {
        for severity in Severity::ALL {
            assert_eq!(Severity::parse(severity.as_str()), Some(severity));
        }
        assert_eq!(Severity::parse("URGENT"), None);
        assert_eq!(Severity::ALL[0], Severity::High, "most serious first");
        for target in [FindingTarget::Cv, FindingTarget::CoverLetter] {
            assert_eq!(FindingTarget::parse(target.as_str()), Some(target));
        }
        assert_eq!(FindingTarget::parse("letter"), None);
    }
}
