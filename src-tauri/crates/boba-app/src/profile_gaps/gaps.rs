//! The gap model (S3.6, FR-6, FR-12): what the Profile is still missing for
//! the Gates and Tailoring, how an answer becomes a fact draft, and what a
//! skipped question costs (AC1).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use specta::Type;

use boba_domain::profile::{FactDraft, FactKind, Lang, ProfileFact};
use boba_domain::scoring::gates::{Cefr, Confidence, GateKind, GateReport, PermitKind};

use crate::ports::repo::system::SystemRepo;
use crate::profile_gaps::ProfileGapsError;

/// The four things the interview asks about (FR-6): the Profile fields the
/// Gates and Tailoring need that nothing could infer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// Work authorization per country or region (the Eligibility Gate).
    Eligibility,
    /// The CEFR level of a declared language (the Language Gate).
    Cefr,
    /// How much notice the user owes an employer.
    NoticePeriod,
    /// A short sample of the user's own writing (optional, FR-6).
    WritingSample,
}

impl GapKind {
    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            GapKind::Eligibility => "eligibility",
            GapKind::Cefr => "cefr",
            GapKind::NoticePeriod => "notice_period",
            GapKind::WritingSample => "writing_sample",
        }
    }

    /// Parse the stored form, `None` for a key this build does not know (a
    /// skip written by a newer build is ignored, never an error).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "eligibility" => Some(GapKind::Eligibility),
            "cefr" => Some(GapKind::Cefr),
            "notice_period" => Some(GapKind::NoticePeriod),
            "writing_sample" => Some(GapKind::WritingSample),
            _ => None,
        }
    }

    /// AC1: the Gates whose Confidence a skipped question lowers. A field no
    /// Gate reads (the notice period, the optional writing sample) lowers
    /// nothing — Tailoring is the only reader, and it does not decide a Gate.
    pub fn related_gates(self) -> &'static [GateKind] {
        match self {
            GapKind::Eligibility => &[GateKind::Eligibility],
            GapKind::Cefr => &[GateKind::Language],
            GapKind::NoticePeriod | GapKind::WritingSample => &[],
        }
    }

    /// FR-6: the writing sample is the one question that can simply be skipped.
    pub fn optional(self) -> bool {
        matches!(self, GapKind::WritingSample)
    }

    /// The key of a question with no subject, for the kinds that have none.
    pub fn key(self) -> String {
        self.as_str().to_string()
    }
}

/// One question the Profile cannot answer itself (FR-6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Gap {
    pub kind: GapKind,
    /// A language tag for `Cefr` (`"en"`, `"es"`), `None` otherwise.
    pub subject: Option<String>,
    /// The fact whose level is missing, for `Cefr`; the answer revises it.
    pub item_id: Option<String>,
}

impl Gap {
    /// The stable key a skip and a question share (`"cefr:en"`).
    pub fn key(&self) -> String {
        match &self.subject {
            Some(subject) => format!("{}:{subject}", self.kind.as_str()),
            None => self.kind.as_str().to_string(),
        }
    }
}

/// Everything the interview asks, in a stable order: eligibility, one CEFR
/// question per language missing a level, the notice period, the sample.
/// Pure: the same facts always produce the same questions (AD-8).
pub fn detect(facts: &[ProfileFact]) -> Vec<Gap> {
    let mut gaps = Vec::new();
    if !facts
        .iter()
        .any(|fact| fact.kind == FactKind::Eligibility && authorization_of(fact).is_some())
    {
        gaps.push(Gap {
            kind: GapKind::Eligibility,
            subject: None,
            item_id: None,
        });
    }
    for fact in facts.iter().filter(|fact| fact.kind == FactKind::Language) {
        if level_of(fact).is_some() {
            continue;
        }
        gaps.push(Gap {
            kind: GapKind::Cefr,
            subject: language_tag(fact).or_else(|| Some(fact.item_id.clone())),
            item_id: Some(fact.item_id.clone()),
        });
    }
    if !facts.iter().any(has_notice_period) {
        gaps.push(Gap {
            kind: GapKind::NoticePeriod,
            subject: None,
            item_id: None,
        });
    }
    if !facts.iter().any(has_writing_sample) {
        gaps.push(Gap {
            kind: GapKind::WritingSample,
            subject: None,
            item_id: None,
        });
    }
    gaps
}

/// The `setting` row the answered-and-skipped questions live in (one JSON
/// array of gap keys).
pub const SKIPPED_KEY: &str = "profile_gaps.skipped";

/// The skipped gap keys, oldest first. An unreadable row is "nothing
/// skipped": a question comes back rather than an error (AD-21 tolerance).
pub fn skipped(system: &dyn SystemRepo) -> Vec<String> {
    system
        .setting_get(SKIPPED_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// Store the skipped gap keys.
pub fn set_skipped(
    system: &dyn SystemRepo,
    keys: &[String],
    now: &str,
) -> Result<(), ProfileGapsError> {
    let raw = serde_json::to_string(keys).map_err(|_| ProfileGapsError::Store)?;
    system
        .setting_put(SKIPPED_KEY, &raw, now)
        .map_err(|_| ProfileGapsError::Store)
}

/// AC1: a question the user skipped lowers the Confidence of the Gates that
/// needed it, by one step. `Low` stays `Low`; a key this build does not know
/// (another kind, another `cefr:` subject) lowers nothing.
pub fn lower_confidence(report: &mut GateReport, skipped_keys: &[String]) {
    let mut lowered: Vec<GateKind> = Vec::new();
    for key in skipped_keys {
        let Some(kind) = kind_of_key(key) else {
            continue;
        };
        for gate in kind.related_gates() {
            if !lowered.contains(gate) {
                lowered.push(*gate);
            }
        }
    }
    for result in &mut report.results {
        if lowered.contains(&result.gate) {
            result.confidence = lower(result.confidence);
        }
    }
}

fn lower(confidence: Confidence) -> Confidence {
    match confidence {
        Confidence::High => Confidence::Medium,
        Confidence::Medium | Confidence::Low => Confidence::Low,
    }
}

/// The kind of a stored key: `"cefr:en"` and `"cefr"` are both `Cefr`.
fn kind_of_key(key: &str) -> Option<GapKind> {
    let head = key.split(':').next().unwrap_or(key);
    GapKind::parse(head)
}

/// The user's answer to one question, as the step collects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GapAnswer {
    /// Where the user may work, and under what authorization.
    Eligibility { region: String, permit: String },
    /// A CEFR level (`A1`..`C2`).
    Cefr { level: String },
    /// How much notice the user owes (`"1 month"`).
    NoticePeriod { text: String },
    /// The user's own writing, verbatim.
    WritingSample { text: String },
}

/// Turn an answer into the fact draft it states (AC1). The sentence is
/// written in the run's content language only: the other variant stays
/// missing until the translate task drafts it and the user confirms (FR-12).
///
/// `existing` is the fact a `Cefr` answer revises; every other kind adds one.
pub fn answer_draft(
    gap: &Gap,
    answer: &GapAnswer,
    lang: Lang,
    existing: Option<&ProfileFact>,
) -> Result<FactDraft, ProfileGapsError> {
    match (gap.kind, answer) {
        (GapKind::Eligibility, GapAnswer::Eligibility { region, permit }) => {
            let region = region.trim().to_ascii_lowercase();
            let valid_region = region == "eu"
                || (region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()));
            if !valid_region {
                return Err(ProfileGapsError::BadAnswer);
            }
            let permit = parse_permit(permit).ok_or(ProfileGapsError::BadAnswer)?;
            let region_code = region.to_uppercase();
            let mut draft = FactDraft::new(FactKind::Eligibility);
            match lang {
                Lang::En => {
                    draft.text_en = Some(format!(
                        "Authorized to work in {region_code} ({})",
                        permit_label(permit, Lang::En)
                    ));
                }
                Lang::Es => {
                    draft.text_es = Some(format!(
                        "Autorizado/a a trabajar en {region_code} ({})",
                        permit_label(permit, Lang::Es)
                    ));
                }
            }
            draft.detail_json = json!({ "region": region, "permit": permit.as_str() }).to_string();
            Ok(draft)
        }
        (GapKind::Cefr, GapAnswer::Cefr { level }) => {
            let fact = existing.ok_or(ProfileGapsError::BadAnswer)?;
            if fact.kind != FactKind::Language {
                return Err(ProfileGapsError::BadAnswer);
            }
            let level = Cefr::parse(level.trim()).ok_or(ProfileGapsError::BadAnswer)?;
            let mut detail = parse_detail(&fact.detail_json);
            if !detail.is_object() {
                detail = json!({});
            }
            detail["level"] = Value::String(level.as_str().to_uppercase());
            let mut draft = fact.draft();
            draft.detail_json = detail.to_string();
            Ok(draft)
        }
        (GapKind::NoticePeriod, GapAnswer::NoticePeriod { text }) => {
            let text = text.trim();
            if text.is_empty() {
                return Err(ProfileGapsError::BadAnswer);
            }
            let mut draft = FactDraft::new(FactKind::Eligibility);
            match lang {
                Lang::En => draft.text_en = Some(format!("Notice period: {text}")),
                Lang::Es => draft.text_es = Some(format!("Preaviso: {text}")),
            }
            draft.detail_json = json!({ "notice_period": text }).to_string();
            Ok(draft)
        }
        (GapKind::WritingSample, GapAnswer::WritingSample { text }) => {
            if text.trim().is_empty() {
                return Err(ProfileGapsError::BadAnswer);
            }
            let mut draft = FactDraft::new(FactKind::Identity);
            match lang {
                Lang::En => draft.text_en = Some(text.clone()),
                Lang::Es => draft.text_es = Some(text.clone()),
            }
            draft.detail_json = json!({ "writing_sample": true }).to_string();
            Ok(draft)
        }
        _ => Err(ProfileGapsError::BadAnswer),
    }
}

/// Parse the stored permit form; the Gates' own vocabulary.
fn parse_permit(value: &str) -> Option<PermitKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "work_permit" => Some(PermitKind::WorkPermit),
        "residency" => Some(PermitKind::Residency),
        "citizenship" => Some(PermitKind::Citizenship),
        "clearance" => Some(PermitKind::Clearance),
        _ => None,
    }
}

/// The label the fact's sentence uses, in the content language.
fn permit_label(permit: PermitKind, lang: Lang) -> &'static str {
    match (permit, lang) {
        (PermitKind::WorkPermit, Lang::En) => "work permit",
        (PermitKind::Residency, Lang::En) => "residency",
        (PermitKind::Citizenship, Lang::En) => "citizenship",
        (PermitKind::Clearance, Lang::En) => "security clearance",
        (PermitKind::WorkPermit, Lang::Es) => "permiso de trabajo",
        (PermitKind::Residency, Lang::Es) => "residencia",
        (PermitKind::Citizenship, Lang::Es) => "ciudadanía",
        (PermitKind::Clearance, Lang::Es) => "habilitación de seguridad",
    }
}

fn parse_detail(detail_json: &str) -> Value {
    serde_json::from_str(detail_json).unwrap_or_else(|_| json!({}))
}

/// The language tag a `Language` fact declares, when it declares one.
fn language_tag(fact: &ProfileFact) -> Option<String> {
    let detail = parse_detail(&fact.detail_json);
    ["language", "lang", "tag"]
        .iter()
        .find_map(|key| detail.get(*key)?.as_str().map(str::to_string))
}

fn level_of(fact: &ProfileFact) -> Option<Cefr> {
    let detail = parse_detail(&fact.detail_json);
    ["level", "cefr"]
        .iter()
        .find_map(|key| detail.get(*key)?.as_str())
        .and_then(Cefr::parse)
}

/// A declared work authorization, read exactly as the Gates read one
/// (`{"region": "es", "permit": "work_permit"}`).
fn authorization_of(fact: &ProfileFact) -> Option<(String, PermitKind)> {
    if fact.kind != FactKind::Eligibility {
        return None;
    }
    let detail = parse_detail(&fact.detail_json);
    let region = detail.get("region")?.as_str()?.trim().to_string();
    if region.is_empty() {
        return None;
    }
    let permit = parse_permit(detail.get("permit")?.as_str()?)?;
    Some((region, permit))
}

fn has_notice_period(fact: &ProfileFact) -> bool {
    let detail = parse_detail(&fact.detail_json);
    detail
        .get("notice_period")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
}

fn has_writing_sample(fact: &ProfileFact) -> bool {
    let detail = parse_detail(&fact.detail_json);
    match detail.get("writing_sample") {
        Some(Value::Bool(set)) => *set,
        Some(Value::String(text)) => !text.trim().is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::testing::FakeSystemRepo;
    use boba_domain::profile::{Provenance, new_ulid};
    use boba_domain::scoring::gates::{GateReason, GateResult, Verdict};

    fn fact(kind: FactKind, text: &str, detail: &str) -> ProfileFact {
        ProfileFact {
            id: new_ulid(),
            item_id: new_ulid(),
            kind,
            text_en: Some(text.to_string()),
            text_es: None,
            evidence: None,
            detail_json: detail.to_string(),
            supersedes: None,
            profile_version: 1,
            provenance: Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-17T09:00:00.000Z".to_string(),
        }
    }

    fn result(gate: GateKind, confidence: Confidence) -> GateResult {
        GateResult {
            gate,
            verdict: Verdict::Flag,
            reason: GateReason::Unverified,
            evidence: None,
            confidence,
        }
    }

    // AC1: an empty Profile asks about eligibility, notice period and the
    // optional writing sample; no language is declared, so no CEFR question.
    #[test]
    fn an_empty_profile_asks_the_four_field_kinds_it_can_ask() {
        let gaps = detect(&[]);
        let kinds: Vec<&str> = gaps.iter().map(|gap| gap.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["eligibility", "notice_period", "writing_sample"]
        );
    }

    // AC1: an answered field is not asked again.
    #[test]
    fn a_declared_authorization_and_notice_period_are_not_gaps() {
        let facts = vec![
            fact(
                FactKind::Eligibility,
                "Authorized to work in ES (citizenship)",
                r#"{"region":"es","permit":"citizenship","notice_period":"1 month"}"#,
            ),
            fact(
                FactKind::Identity,
                "I write plainly and lead with outcomes.",
                r#"{"writing_sample":true}"#,
            ),
        ];
        let gaps = detect(&facts);
        assert!(gaps.is_empty(), "nothing is missing: {gaps:?}");
    }

    // FR-6/AC1: a language with no CEFR level is one question, about that
    // language, bound to the fact whose level is missing.
    #[test]
    fn a_language_without_a_level_is_a_cefr_gap_for_that_language() {
        let with = fact(
            FactKind::Language,
            "English",
            r#"{"language":"en","level":"C1"}"#,
        );
        let without_en = fact(FactKind::Language, "Spanish", r#"{"language":"es"}"#);
        let item_id = without_en.item_id.clone();
        let gaps = detect(&[with, without_en]);
        let cefr: Vec<&Gap> = gaps
            .iter()
            .filter(|gap| gap.kind == GapKind::Cefr)
            .collect();
        assert_eq!(cefr.len(), 1);
        assert_eq!(cefr[0].subject.as_deref(), Some("es"));
        assert_eq!(cefr[0].item_id.as_deref(), Some(item_id.as_str()));
        assert_eq!(cefr[0].key(), "cefr:es");
    }

    // FR-6: the writing sample is optional; the rest are not.
    #[test]
    fn only_the_writing_sample_is_optional() {
        assert!(GapKind::WritingSample.optional());
        for kind in [GapKind::Eligibility, GapKind::Cefr, GapKind::NoticePeriod] {
            assert!(!kind.optional(), "{kind:?}");
        }
    }

    #[test]
    fn gap_keys_round_trip() {
        let gap = Gap {
            kind: GapKind::Cefr,
            subject: Some("en".to_string()),
            item_id: None,
        };
        assert_eq!(gap.key(), "cefr:en");
        assert_eq!(GapKind::parse("cefr"), Some(GapKind::Cefr));
        assert_eq!(GapKind::parse("hobby"), None);
        assert_eq!(GapKind::Eligibility.key(), "eligibility");
        assert_eq!(GapKind::NoticePeriod.key(), "notice_period");
        assert_eq!(GapKind::WritingSample.key(), "writing_sample");
    }

    // AC1: answered and skipped questions are remembered by key in the one
    // `setting` row, so a later run reads the same list.
    #[test]
    fn skips_round_trip_through_the_setting_row() {
        let system = FakeSystemRepo::default();
        assert!(skipped(&system).is_empty());
        let keys = vec!["eligibility".to_string(), "cefr:en".to_string()];
        set_skipped(&system, &keys, "2026-10-17T09:00:00.000Z").expect("write");
        assert_eq!(skipped(&system), keys);
        set_skipped(&system, &[], "2026-10-17T09:05:00.000Z").expect("write");
        assert!(skipped(&system).is_empty());
    }

    // AC1: a skipped question lowers the Confidence of the Gate that needed it
    // — one step, never above what the gate itself claimed.
    #[test]
    fn a_skipped_question_lowers_its_related_gates_confidence_one_step() {
        let mut report = GateReport {
            results: vec![
                result(GateKind::Eligibility, Confidence::High),
                result(GateKind::Language, Confidence::Medium),
                result(GateKind::Location, Confidence::High),
            ],
        };
        lower_confidence(&mut report, &["eligibility".to_string()]);
        assert_eq!(
            report.gate(GateKind::Eligibility).unwrap().confidence,
            Confidence::Medium
        );
        // A gate the skip does not concern keeps its confidence.
        assert_eq!(
            report.gate(GateKind::Location).unwrap().confidence,
            Confidence::High
        );
        assert_eq!(
            report.gate(GateKind::Language).unwrap().confidence,
            Confidence::Medium
        );
    }

    #[test]
    fn a_cefr_skip_lowers_the_language_gate_and_low_stays_low() {
        let mut report = GateReport {
            results: vec![
                result(GateKind::Language, Confidence::Low),
                result(GateKind::Eligibility, Confidence::High),
            ],
        };
        lower_confidence(&mut report, &["cefr:es".to_string()]);
        assert_eq!(
            report.gate(GateKind::Language).unwrap().confidence,
            Confidence::Low
        );
        // The Eligibility gate never read the CEFR level.
        assert_eq!(
            report.gate(GateKind::Eligibility).unwrap().confidence,
            Confidence::High
        );
    }

    // The optional writing sample is a question, but skipping it costs nothing.
    #[test]
    fn skipping_the_optional_sample_lowers_no_gate() {
        let mut report = GateReport {
            results: vec![result(GateKind::Eligibility, Confidence::High)],
        };
        lower_confidence(&mut report, &["writing_sample".to_string()]);
        assert_eq!(
            report.gate(GateKind::Eligibility).unwrap().confidence,
            Confidence::High
        );
    }

    #[test]
    fn an_unknown_skip_key_is_ignored() {
        let mut report = GateReport {
            results: vec![result(GateKind::Eligibility, Confidence::High)],
        };
        lower_confidence(&mut report, &["from_a_newer_build".to_string()]);
        assert_eq!(
            report.gate(GateKind::Eligibility).unwrap().confidence,
            Confidence::High
        );
    }

    // AC1: an eligibility answer is a structured fact — region and permit in
    // `detail_json`, the sentence in the content language only (FR-12: the
    // other variant stays missing until it is drafted and confirmed).
    #[test]
    fn an_eligibility_answer_becomes_a_fact_draft() {
        let gap = Gap {
            kind: GapKind::Eligibility,
            subject: None,
            item_id: None,
        };
        let draft = answer_draft(
            &gap,
            &GapAnswer::Eligibility {
                region: "ES".to_string(),
                permit: "citizenship".to_string(),
            },
            Lang::En,
            None,
        )
        .expect("draft");
        assert_eq!(draft.kind, FactKind::Eligibility);
        assert_eq!(
            draft.text_en.as_deref(),
            Some("Authorized to work in ES (citizenship)")
        );
        assert!(draft.text_es.is_none());
        let detail: serde_json::Value = serde_json::from_str(&draft.detail_json).unwrap();
        assert_eq!(detail["region"], "es");
        assert_eq!(detail["permit"], "citizenship");
    }

    #[test]
    fn a_notice_period_answer_becomes_the_fact_text_and_detail() {
        let gap = Gap {
            kind: GapKind::NoticePeriod,
            subject: None,
            item_id: None,
        };
        let draft = answer_draft(
            &gap,
            &GapAnswer::NoticePeriod {
                text: "2 months".to_string(),
            },
            Lang::Es,
            None,
        )
        .expect("draft");
        assert_eq!(draft.kind, FactKind::Eligibility);
        assert_eq!(draft.text_es.as_deref(), Some("Preaviso: 2 months"));
        assert!(draft.text_en.is_none());
        let detail: serde_json::Value = serde_json::from_str(&draft.detail_json).unwrap();
        assert_eq!(detail["notice_period"], "2 months");
    }

    // AC1/FR-12: a CEFR answer revises the language fact it belongs to and
    // keeps everything else the fact states.
    #[test]
    fn a_cefr_answer_revises_the_language_fact() {
        let existing = fact(
            FactKind::Language,
            "Spanish",
            r#"{"language":"es","notes":"native speaker"}"#,
        );
        let gap = Gap {
            kind: GapKind::Cefr,
            subject: Some("es".to_string()),
            item_id: Some(existing.item_id.clone()),
        };
        let draft = answer_draft(
            &gap,
            &GapAnswer::Cefr {
                level: "c1".to_string(),
            },
            Lang::En,
            Some(&existing),
        )
        .expect("draft");
        assert_eq!(draft.kind, FactKind::Language);
        assert_eq!(draft.text_en.as_deref(), Some("Spanish"));
        let detail: serde_json::Value = serde_json::from_str(&draft.detail_json).unwrap();
        assert_eq!(detail["level"], "C1");
        assert_eq!(detail["notes"], "native speaker");
    }

    #[test]
    fn an_unknown_or_off_scale_cefr_level_is_refused() {
        let existing = fact(FactKind::Language, "Spanish", r#"{"language":"es"}"#);
        let gap = Gap {
            kind: GapKind::Cefr,
            subject: Some("es".to_string()),
            item_id: Some(existing.item_id.clone()),
        };
        let error = answer_draft(
            &gap,
            &GapAnswer::Cefr {
                level: "native".to_string(),
            },
            Lang::En,
            Some(&existing),
        )
        .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");
    }

    // AC1: an answer that does not match its question is refused, not guessed.
    #[test]
    fn an_answer_of_the_wrong_kind_is_refused() {
        let gap = Gap {
            kind: GapKind::Eligibility,
            subject: None,
            item_id: None,
        };
        let error = answer_draft(
            &gap,
            &GapAnswer::WritingSample {
                text: "anything".to_string(),
            },
            Lang::En,
            None,
        )
        .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");
    }

    #[test]
    fn a_blank_answer_is_refused() {
        let gap = Gap {
            kind: GapKind::NoticePeriod,
            subject: None,
            item_id: None,
        };
        let error = answer_draft(
            &gap,
            &GapAnswer::NoticePeriod {
                text: "   ".to_string(),
            },
            Lang::En,
            None,
        )
        .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");
    }

    #[test]
    fn an_eligibility_region_is_normalised_and_a_bad_permit_refused() {
        let gap = Gap {
            kind: GapKind::Eligibility,
            subject: None,
            item_id: None,
        };
        let error = answer_draft(
            &gap,
            &GapAnswer::Eligibility {
                region: "ES".to_string(),
                permit: "visa".to_string(),
            },
            Lang::En,
            None,
        )
        .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");

        let error = answer_draft(
            &gap,
            &GapAnswer::Eligibility {
                region: "  ".to_string(),
                permit: "residency".to_string(),
            },
            Lang::En,
            None,
        )
        .unwrap_err();
        assert_eq!(error.message_key(), "errors.profile_gaps.bad_answer");
    }

    #[test]
    fn a_writing_sample_becomes_an_identity_fact_verbatim() {
        let sample = "I write plainly.\nI lead with outcomes.";
        let gap = Gap {
            kind: GapKind::WritingSample,
            subject: None,
            item_id: None,
        };
        let draft = answer_draft(
            &gap,
            &GapAnswer::WritingSample {
                text: sample.to_string(),
            },
            Lang::En,
            None,
        )
        .expect("draft");
        assert_eq!(draft.kind, FactKind::Identity);
        assert_eq!(draft.text_en.as_deref(), Some(sample));
        let detail: serde_json::Value = serde_json::from_str(&draft.detail_json).unwrap();
        assert_eq!(detail["writing_sample"], true);
    }
}
