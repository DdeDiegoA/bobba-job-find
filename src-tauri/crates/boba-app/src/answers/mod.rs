//! Screening answers and the answer bank (S7.8, FR-80).
//!
//! A form (an ATS form, or questions the user pasted) is answered by one
//! `answer_questions` run. The run drafts; **the core disposes**:
//!
//! * **the class** — the core reads the question's own words and decides
//!   whether it is about pay, eligibility or anything else ([`QuestionClass`]).
//!   The run never picks that. A question the words only make *suspicious*
//!   ("payroll", "rate your skills") is answered by the run as free text and
//!   flagged for review, rather than classed on a guess.
//! * **salary and eligibility** — those answers are the user's Preferences, not
//!   the run's prose (FR-80): the run's words for such a question are dropped
//!   and the value comes from [`Defaults`]. With no Preferences value there is
//!   no answer at all, and the question drops to review. Either way the row is
//!   flagged `needs_review`, always.
//! * **everything else** — a free-text answer stands on the user's own facts: it
//!   names the `fact_ids` it was built from, and a payload naming a fact the
//!   profile does not have is refused whole (the no-fabrication rule).
//!
//! What the run wrote is a **draft**: it lands in `application_answer_bank`
//! `untrusted_origin = 1` and unapproved, and it is shown as data until the user
//! edits or confirms it (ADR-010). Only [`approve`] makes a row reusable, which
//! is what [`reuse`] hands the next run (FR-80: "answers I approve are saved to
//! the answer bank for reuse").

use boba_domain::application::{
    AnswerClass, BankAnswer, NewBankAnswer, new_answer_id, question_key,
};
use serde::{Deserialize, Serialize};

use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};

/// The longest a question may be, in characters. A form's question is a
/// sentence; anything longer is a runaway payload that would bloat the bank row
/// and the next run's prompt, so it is refused rather than stored.
pub const MAX_QUESTION_TEXT: usize = 500;

/// The longest an answer may be, in characters. A screening answer is a
/// paragraph; a runaway one is refused rather than stored and reused forever.
pub const MAX_ANSWER_TEXT: usize = 2000;

/// How many questions one run may answer. Far above a real form (and above the
/// longest ATS application), so nothing honest is lost.
pub const MAX_ANSWERS: usize = 40;

/// The answers the core already knows, from the user's own settings (FR-80).
///
/// The core never reads a price or a work authorization out of the run's prose:
/// the value is what the user set — the Salary Floor in Preferences, the
/// eligibility declarations in the Profile. `None` means "the user has not
/// said", and the question then drops to review instead of being answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Defaults {
    /// The salary expectation, from the Preferences Salary Floor.
    pub salary: Option<String>,
    /// The eligibility answer, from the eligibility facts of the Profile.
    pub eligibility: Option<String>,
}

impl Defaults {
    /// The defaults a run is given.
    pub fn new(salary: Option<String>, eligibility: Option<String>) -> Self {
        Self {
            salary,
            eligibility,
        }
    }

    /// The defaults built from the user's own settings: the Salary Floor from
    /// Preferences — as `(amount_minor, currency, period)`, `None` when they set
    /// none — and the eligibility the Profile declares.
    ///
    /// The declaration is read in the language it was written in first: which
    /// language a run drafts in is not known where a tool call is answered, and
    /// taking the user's own words over a translation is never wrong.
    pub fn from_user_settings(
        facts: &[boba_domain::profile::ProfileFact],
        floor: Option<(i64, String, String)>,
    ) -> Self {
        Self::new(
            floor.map(|(amount_minor, currency, period)| {
                salary_expectation(amount_minor, &currency, &period)
            }),
            eligibility_expectation(facts, boba_domain::profile::Lang::En),
        )
    }

    /// The value for a class that comes from Preferences, `None` for the others
    /// and when the user has set nothing.
    fn for_class(&self, class: AnswerClass) -> Option<&str> {
        match class {
            AnswerClass::Salary => self.salary.as_deref(),
            AnswerClass::Eligibility => self.eligibility.as_deref(),
            AnswerClass::Other => None,
        }
    }
}

/// The salary expectation a salary question is answered with (FR-80): the user's
/// own Salary Floor, written the way a form reads it. Amounts are the AD-17
/// money shape — integer minor units, two decimals, like
/// `boba_domain::posting::Money` — so `600_000` is `USD 6000`.
pub fn salary_expectation(amount_minor: i64, currency: &str, period: &str) -> String {
    let major = amount_minor / 100;
    let minor = (amount_minor % 100).abs();
    let amount = if minor == 0 {
        major.to_string()
    } else {
        format!("{major}.{minor:02}")
    };
    format!("{currency} {amount} per {period}")
}

/// The eligibility answer a question is answered with (FR-80): the eligibility
/// the Profile declares, in the run's language when the user wrote it in one and
/// in the other otherwise. Several declarations are joined, because a form that
/// asks one question needs all of them; `None` when the Profile declares none.
pub fn eligibility_expectation(
    facts: &[boba_domain::profile::ProfileFact],
    lang: boba_domain::profile::Lang,
) -> Option<String> {
    let other = match lang {
        boba_domain::profile::Lang::En => boba_domain::profile::Lang::Es,
        boba_domain::profile::Lang::Es => boba_domain::profile::Lang::En,
    };
    let mut texts: Vec<String> = Vec::new();
    for fact in facts
        .iter()
        .filter(|fact| fact.kind == boba_domain::profile::FactKind::Eligibility)
    {
        let text = fact.text(lang).or_else(|| fact.text(other));
        if let Some(text) = text
            && !texts.iter().any(|seen| seen == text)
        {
            texts.push(text.to_string());
        }
    }
    (!texts.is_empty()).then(|| texts.join("; "))
}

/// A question to answer, as the core knows it (FR-80). The text is third-party
/// data: the run echoes it back and the UI shows it as text only (ADR-010).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    /// The class the core read out of the question (S7.8). Stored so the UI does
    /// not have to re-derive what the core decided.
    pub class: AnswerClass,
}

impl Question {
    /// A question with its class derived the one way: by the core.
    pub fn of(question: impl Into<String>) -> Self {
        let question = question.into();
        let class = QuestionClass::of(&question);
        Self { question, class }
    }
}

/// Whether a question is about pay or eligibility, decided from its own words.
/// This is the core's call, never the run's: it is what makes those two classes
/// take their value from Preferences and stay flagged for review.
///
/// The lexicon has two tiers, because the honest reading of a form's wording is
/// not always a classification:
///
/// * **the class**, from words that mean pay or eligibility and little else
///   ([`SALARY_STEMS`], [`SALARY_PHRASES`], [`ELIGIBILITY_STEMS`],
///   [`ELIGIBILITY_PHRASES`]). Those questions take their value from the user's
///   own settings.
/// * **the suspicion**, from words that are *often* about them but appear in
///   ordinary questions too ("payroll", "rate your skills", "legal name"). A
///   suspicious question stays free text — the run's grounded answer is kept —
///   and is flagged for review ([`is_suspect`]). Guessing here would either drop
///   a real answer or store an invented one; flagging costs the user one look.
pub struct QuestionClass;

/// Word beginnings that make a question a **pay** question (EN + ES), matched as
/// a prefix of a whole word so one entry covers the family: `salar` covers
/// `salary`, `salaries`, `salario`, `salarial`, `salariales`.
const SALARY_STEMS: [&str; 5] = ["salar", "wage", "compensat", "remuner", "sueld"];

/// Pay phrases that a single word cannot carry (EN + ES), matched as whole words.
const SALARY_PHRASES: [&str; 9] = [
    "pay range",
    "pay band",
    "expected pay",
    "desired pay",
    "pay expectation",
    "pay expectations",
    "hourly rate",
    "daily rate",
    "day rate",
];

/// Word beginnings that make a question an **eligibility** question: citizenship,
/// visa and residence wording (`visa` covers `visas` and `visado`).
const ELIGIBILITY_STEMS: [&str; 3] = ["citizenship", "residen", "visa"];

/// Eligibility phrases (EN + ES), matched as whole words.
const ELIGIBILITY_PHRASES: [&str; 19] = [
    "authorized to work",
    "authorised to work",
    "authorization to work",
    "authorisation to work",
    "legally authorized",
    "legally authorised",
    "legally permitted",
    "permitted to work",
    "eligible to work",
    "right to work",
    "work permit",
    "work authorization",
    "work authorisation",
    "work eligibility",
    "sponsorship",
    "permiso de trabajo",
    "permiso de residencia",
    "autorización para trabajar",
    "elegible para trabajar",
];

/// Word beginnings that make a question **suspicious** — pay-ish, eligibility-ish
/// — without settling its class (see [`QuestionClass`]). Kept to words that reach
/// an ordinary question: "payroll", "rate your skills", "legal name".
const SUSPECT_STEMS: [&str; 7] = [
    "pay", "rate", "eligib", "permit", "sponsor", "citizen", "legal",
];

impl QuestionClass {
    /// The class of one question, from its EN/ES wording. Pay is checked first:
    /// a question that asks both ("salary expectation for the role you are
    /// authorized to work in") is a pay question, whose answer is the one the
    /// user must see before it goes anywhere.
    pub fn of(question: &str) -> AnswerClass {
        let words = words_of(question);
        if matches(&words, &SALARY_STEMS, &SALARY_PHRASES) {
            return AnswerClass::Salary;
        }
        if matches(&words, &ELIGIBILITY_STEMS, &ELIGIBILITY_PHRASES) {
            return AnswerClass::Eligibility;
        }
        AnswerClass::Other
    }

    /// Whether the core *suspects* the question is about pay or eligibility
    /// without classing it as one: the answer is still the run's, and the user
    /// is asked to look at it.
    pub fn is_suspect(question: &str) -> bool {
        let words = words_of(question);
        matches(&words, &SUSPECT_STEMS, &[])
    }
}

/// Whether any stem begins a word of `words`, or any phrase is a run of them.
fn matches(words: &str, stems: &[&str], phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| contains_phrase(words, phrase))
        || words
            .split_whitespace()
            .any(|word| stems.iter().any(|stem| word.starts_with(stem)))
}

/// The question's words, lower-cased, with punctuation that is not part of a
/// word (an accent kept, a comma dropped) so phrase matching sees words only.
fn words_of(question: &str) -> String {
    question
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `phrase` appears in `words` as a run of whole words.
fn contains_phrase(words: &str, phrase: &str) -> bool {
    let needle = format!(" {phrase} ");
    format!(" {words} ").contains(&needle)
}

/// One drafted answer, as the core decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftedAnswer {
    /// The bank row's id.
    pub id: String,
    pub question: String,
    pub class: AnswerClass,
    /// The answer the core stored. `None` when it has no confident one (a salary
    /// question with no Salary Floor set): the row waits for the user.
    pub answer: Option<String>,
    /// The profile facts the answer stands on; empty for the classes whose value
    /// comes from Preferences.
    pub fact_ids: Vec<String>,
    pub needs_review: bool,
}

/// What one `save_answers` call stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drafted {
    pub application_id: String,
    /// One entry per question the payload answered, in the order it answered
    /// them.
    pub answers: Vec<DraftedAnswer>,
}

impl Drafted {
    /// The drafted answer of one question key, when the payload had one.
    pub fn answer_for(&self, question: &str) -> Option<&DraftedAnswer> {
        let key = question_key(question);
        self.answers
            .iter()
            .find(|answer| question_key(&answer.question) == key)
    }
}

/// Why a drafting run's payload could not be stored.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswersError {
    /// The run's payload is not a set of answers the core can store. Refused
    /// whole, so the run retries instead of half-answering a form.
    #[error("this answer payload is not usable: {0}")]
    Payload(String),
    /// The user's own edit is not storable (empty, or far longer than an
    /// answer).
    #[error("this edited answer is not usable: {0}")]
    Edit(String),
    /// The application does not exist, so there is nothing to answer for.
    #[error("unknown application: {0}")]
    UnknownApplication(String),
    /// The bank row does not exist.
    #[error("unknown answer: {0}")]
    UnknownAnswer(String),
    /// The store refused the write.
    #[error("apply store error: {0}")]
    Repo(#[from] ApplyRepoError),
}

/// How many profile facts one answer may stand on. A real answer stands on a
/// handful; a payload claiming thousands is a runaway one.
pub const MAX_FACT_IDS: usize = 32;

/// Store the answers one `answer_questions` run drafted (S7.8, FR-80).
///
/// Every answer lands as an unapproved, untrusted row (AC3) — the run's words
/// for a salary or eligibility question are dropped in favour of the
/// Preferences default, and nothing is written at all when the payload, the
/// application or the facts it names cannot be used.
pub fn save_answers(
    repo: &dyn ApplyRepo,
    args: &serde_json::Value,
    defaults: &Defaults,
    facts: &[String],
    created_at: &str,
) -> Result<Drafted, AnswersError> {
    let raw = parse_payload(args)?;
    if repo.application_get(&raw.application_id)?.is_none() {
        return Err(AnswersError::UnknownApplication(raw.application_id));
    }

    let mut answers = Vec::with_capacity(raw.answers.len());
    for answer in raw.answers {
        let class = QuestionClass::of(&answer.question);
        let drafted = dispose(answer, class, defaults, facts)?;
        let id = new_answer_id();
        repo.answer_bank_insert(&NewBankAnswer {
            id: id.clone(),
            application_id: raw.application_id.clone(),
            question: drafted.question.clone(),
            question_key: question_key(&drafted.question),
            answer: drafted.answer.clone(),
            class,
            fact_ids: drafted.fact_ids.clone(),
            needs_review: drafted.needs_review,
            // AC3/ADR-010: the run wrote this, so it is untrusted until the
            // user edits or confirms it.
            untrusted_origin: true,
            created_at: created_at.to_string(),
        })?;
        answers.push(DraftedAnswer { id, ..drafted });
    }

    Ok(Drafted {
        application_id: raw.application_id,
        answers,
    })
}

/// What one drafted answer amounts to, once the core has decided it: the value
/// for a class that comes from Preferences, or the run's grounded answer.
fn dispose(
    answer: RawAnswer,
    class: AnswerClass,
    defaults: &Defaults,
    facts: &[String],
) -> Result<DraftedAnswer, AnswersError> {
    if class.comes_from_preferences() {
        // The run's own words for these are dropped: the value is the user's
        // Preferences, and with none set there is no answer at all — the
        // question drops to review (FR-80).
        return Ok(DraftedAnswer {
            id: String::new(),
            question: answer.question,
            class,
            answer: defaults.for_class(class).map(str::to_string),
            fact_ids: Vec::new(),
            needs_review: true,
        });
    }

    let Some(text) = answer.answer else {
        return Err(malformed(format!(
            "`answer` is missing for the free-text question `{}`",
            answer.question
        )));
    };
    if answer.fact_ids.is_empty() {
        return Err(malformed(format!(
            "`fact_ids` must name the profile facts `{text}` stands on"
        )));
    }
    if let Some(unknown) = answer.fact_ids.iter().find(|id| !facts.contains(id)) {
        return Err(malformed(format!(
            "`fact_ids` names `{unknown}`, which this profile does not have"
        )));
    }
    // A question the core only suspects is answered by the run, and flagged:
    // the near misses ("payroll", "rate your skills") must reach the user
    // rather than be answered quietly (review round 1).
    let needs_review = QuestionClass::is_suspect(&answer.question);
    Ok(DraftedAnswer {
        id: String::new(),
        question: answer.question,
        class,
        answer: Some(text),
        fact_ids: answer.fact_ids,
        needs_review,
    })
}

/// Approve one drafted answer: the user confirmed it as it stands (`edited` is
/// `None`) or replaced its text (AC2, AC3). Either way the row stops being agent
/// text and becomes reusable.
pub fn approve(
    repo: &dyn ApplyRepo,
    id: &str,
    edited: Option<&str>,
    approved_at: &str,
) -> Result<(), AnswersError> {
    if let Some(text) = edited {
        if text.trim().is_empty() {
            return Err(AnswersError::Edit("an answer cannot be blank".to_string()));
        }
        if text.chars().count() > MAX_ANSWER_TEXT {
            return Err(AnswersError::Edit(format!(
                "an answer cannot be longer than {MAX_ANSWER_TEXT} characters"
            )));
        }
    }
    if !repo.answer_bank_approve(id, edited, approved_at)? {
        return Err(AnswersError::UnknownAnswer(id.to_string()));
    }
    Ok(())
}

/// The approved answers that answer one of `questions` (AC2): what the next run
/// is handed so the user answers the same thing twice only once.
pub fn reuse(
    repo: &dyn ApplyRepo,
    questions: &[Question],
) -> Result<Vec<BankAnswer>, AnswersError> {
    let keys: Vec<String> = questions
        .iter()
        .map(|question| question_key(&question.question))
        .collect();
    Ok(repo.answer_bank_reusable(&keys)?)
}

/// The input of an `answer_questions` run: the questions, the Preferences
/// defaults for the sensitive classes, and the answers the user already approved
/// (FR-80). The run drafts from these and its own profile facts, and the core
/// decides what is stored.
pub fn answers_input(
    application_id: &str,
    questions: &[Question],
    defaults: &Defaults,
    reuse: &[BankAnswer],
) -> String {
    let questions: Vec<serde_json::Value> = questions
        .iter()
        .map(|question| {
            serde_json::json!({
                "question": question.question,
                "class": question.class.as_str(),
            })
        })
        .collect();
    let known: Vec<serde_json::Value> = reuse
        .iter()
        .filter_map(|row| {
            row.answer
                .as_ref()
                .map(|answer| serde_json::json!({"question": row.question, "answer": answer}))
        })
        .collect();
    serde_json::json!({
        "app_id": application_id,
        "questions": questions,
        "preferences": {
            "salary": defaults.salary,
            "eligibility": defaults.eligibility,
        },
        "known_answers": known,
    })
    .to_string()
}

/// One answer as the run wrote it, before the core decides anything.
struct RawAnswer {
    question: String,
    answer: Option<String>,
    fact_ids: Vec<String>,
}

/// A payload of answers, parsed and bounded.
struct RawDraft {
    application_id: String,
    answers: Vec<RawAnswer>,
}

/// Parse a `save_answers` payload.
///
/// Everything is checked before any of it is used: a payload the core cannot
/// store is refused whole, so half a form is never answered (the run's output
/// goes back to the agent, which retries, and fails visibly if it cannot fix
/// it). Unknown fields are refused too — an invented field is a payload we do
/// not understand, not one to skip past.
fn parse_payload(args: &serde_json::Value) -> Result<RawDraft, AnswersError> {
    let object = args
        .as_object()
        .ok_or_else(|| malformed("the arguments must be an object"))?;
    for key in object.keys() {
        if key != "app_id" && key != "answers" {
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
        .get("answers")
        .and_then(|value| value.as_array())
        .ok_or_else(|| malformed("`answers` must be an array"))?;
    if raw.len() > MAX_ANSWERS {
        return Err(malformed(format!(
            "a payload cannot carry more than {MAX_ANSWERS} answers, this one has {}",
            raw.len()
        )));
    }

    let mut answers: Vec<RawAnswer> = Vec::with_capacity(raw.len());
    for (index, value) in raw.iter().enumerate() {
        let answer = parse_answer(index, value)?;
        if answers
            .iter()
            .any(|seen| question_key(&seen.question) == question_key(&answer.question))
        {
            return Err(malformed(format!(
                "`answers[{index}].question` answers a question the payload already answered"
            )));
        }
        answers.push(answer);
    }
    Ok(RawDraft {
        application_id,
        answers,
    })
}

fn parse_answer(index: usize, value: &serde_json::Value) -> Result<RawAnswer, AnswersError> {
    let at = |field: &str| format!("`answers[{index}].{field}`");
    let object = value
        .as_object()
        .ok_or_else(|| malformed(format!("{} must be an object", at(""))))?;
    for key in object.keys() {
        if !["question", "answer", "fact_ids"].contains(&key.as_str()) {
            return Err(malformed(format!(
                "{} is not a field an answer has",
                at(key)
            )));
        }
    }
    let question = text(object.get("question"), MAX_QUESTION_TEXT, &at("question"))?;
    // A salary or eligibility question needs no answer from the run (the value
    // is the user's Preferences), so an absent one is allowed here and refused
    // in `dispose` only where an answer really is required.
    let answer = match object.get("answer") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(text(Some(value), MAX_ANSWER_TEXT, &at("answer"))?),
    };
    let fact_ids = match object.get("fact_ids") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(value) => {
            let ids = value
                .as_array()
                .ok_or_else(|| malformed(format!("{} must be an array", at("fact_ids"))))?;
            if ids.len() > MAX_FACT_IDS {
                return Err(malformed(format!(
                    "{} may name at most {MAX_FACT_IDS} facts",
                    at("fact_ids")
                )));
            }
            ids.iter()
                .enumerate()
                .map(|(n, id)| {
                    id.as_str()
                        .filter(|id| !id.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            malformed(format!(
                                "{}[{n}] must be a non-empty string",
                                at("fact_ids")
                            ))
                        })
                })
                .collect::<Result<Vec<String>, AnswersError>>()?
        }
    };
    Ok(RawAnswer {
        question,
        answer,
        fact_ids,
    })
}

/// A non-empty, bounded string of a payload.
fn text(value: Option<&serde_json::Value>, limit: usize, at: &str) -> Result<String, AnswersError> {
    let text = value
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| malformed(format!("{at} must be a non-empty string")))?;
    if text.chars().count() > limit {
        return Err(malformed(format!("{at} is longer than {limit} characters")));
    }
    Ok(text.to_string())
}

/// A payload the core cannot use, with the field that broke it.
fn malformed(why: impl Into<String>) -> AnswersError {
    AnswersError::Payload(why.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use boba_domain::application::{AppStatus, Application};

    use super::*;
    use crate::applications::testing::FakeApplyRepo;

    /// A repo holding one application, as the Tracker would.
    fn repo(app_id: &str) -> FakeApplyRepo {
        let repo = FakeApplyRepo::default();
        repo.seed_application(Application {
            id: app_id.to_string(),
            posting_id: "pst_1".to_string(),
            status: AppStatus::Saved,
            offer_snapshot_json: "{}".to_string(),
            notes: None,
            next_action: None,
            created_at: "2026-10-16T09:00:00Z".to_string(),
            updated_at: "2026-10-16T09:00:00Z".to_string(),
        });
        repo
    }

    /// The profile facts the user's current profile holds.
    fn facts() -> Vec<String> {
        vec!["fact_1".to_string(), "fact_2".to_string()]
    }

    /// The Preferences the caller read for this run.
    fn defaults() -> Defaults {
        Defaults::new(
            Some("USD 6000 per month".to_string()),
            Some("I am authorized to work in the EU".to_string()),
        )
    }

    /// The reason a payload was refused, for assertions.
    fn why(error: &AnswersError) -> String {
        match error {
            AnswersError::Payload(why) => why.clone(),
            other => panic!("expected a refused payload, got {other}"),
        }
    }

    // AC1, widened: the wording real forms use is classed from its own words, and
    // the ambiguous words fail closed instead of silently answering.
    #[test]
    fn the_wording_real_forms_use_is_classed_not_guessed() {
        for question in [
            "What is your desired wage?",
            "Expected pay range?",
            "What are your pretensiones salariales?",
            "¿Cuál es tu sueldo deseado?",
            "What is your hourly rate?",
            "Compensation expectation (annual)?",
            "¿Cuáles son tus expectativas salariales?",
            "What salary do you require?",
        ] {
            assert_eq!(
                QuestionClass::of(question),
                AnswerClass::Salary,
                "{question}"
            );
        }

        for question in [
            "Are you eligible to work in the EU?",
            "Are you legally permitted to work here?",
            "Do you hold citizenship of an EU country?",
            "¿Tienes permiso de residencia?",
            "Will you require visa sponsorship?",
            "Do you have a work permit?",
            "Are you authorised to work in Canada?",
        ] {
            assert_eq!(
                QuestionClass::of(question),
                AnswerClass::Eligibility,
                "{question}"
            );
        }
    }

    /// Fail-closed on the words that are only *sometimes* about pay or
    /// eligibility: the near misses an ordinary question also trips ("payroll",
    /// "rate your skills", "legal name") stay free text — the run's grounded
    /// answer is kept — but they are flagged, so the user is the one who decides.
    #[test]
    fn a_question_the_core_only_suspects_is_answered_and_flagged() {
        let bank = repo("app_1");
        let drafted = save_answers(
            &bank,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {
                        "question": "Have you used payroll software?",
                        "answer": "Yes, two of them.",
                        "fact_ids": ["fact_1"]
                    },
                    {
                        "question": "Rate your Rust skills from 1 to 5.",
                        "answer": "A solid 4.",
                        "fact_ids": ["fact_2"]
                    },
                    {
                        "question": "What is your legal name?",
                        "answer": "Ada Lovelace.",
                        "fact_ids": ["fact_1"]
                    },
                ],
            }),
            &Defaults::default(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("grounded answers");

        assert_eq!(drafted.answers.len(), 3);
        for answer in &drafted.answers {
            assert_eq!(
                answer.class,
                AnswerClass::Other,
                "a suspicion is not a class: {}",
                answer.question
            );
            assert!(
                answer.answer.is_some(),
                "the run's grounded answer is kept: {}",
                answer.question
            );
            assert!(
                answer.needs_review,
                "but the user looks at it: {}",
                answer.question
            );
        }
        assert!(
            bank.answers().iter().all(|row| row.needs_review),
            "the flag reaches the stored draft"
        );

        // A question with none of those words is not flagged.
        let plain = repo("app_2");
        save_answers(
            &plain,
            &json!({
                "app_id": "app_2",
                "answers": [{
                    "question": "What is your notice period?",
                    "answer": "Four weeks.",
                    "fact_ids": ["fact_1"]
                }],
            }),
            &Defaults::default(),
            &facts(),
            "t",
        )
        .expect("a grounded answer");
        assert!(!plain.answers()[0].needs_review);
    }

    // AC1, the core's half: the class of a question is read from its own words,
    // and an ordinary question is never mistaken for a pay or eligibility one.
    #[test]
    fn the_class_of_a_question_comes_from_its_own_words() {
        for question in [
            "What are your salary expectations?",
            "Please state your expected compensation.",
            "¿Cuál es tu pretensión salarial?",
            "Salary expectation (USD per year)?",
        ] {
            assert_eq!(
                QuestionClass::of(question),
                AnswerClass::Salary,
                "{question}"
            );
        }

        for question in [
            "Are you legally authorized to work in the United States?",
            "Do you now or in the future require visa sponsorship?",
            "¿Tienes permiso de trabajo en México?",
            "Do you have the right to work in the EU?",
        ] {
            assert_eq!(
                QuestionClass::of(question),
                AnswerClass::Eligibility,
                "{question}"
            );
        }

        for question in [
            "What is your notice period?",
            "Tell us about a project you are proud of.",
            "Rate your Rust skills from 1 to 5.",
            "Where are you based?",
            "¿Cuál es tu disponibilidad para empezar?",
        ] {
            assert_eq!(
                QuestionClass::of(question),
                AnswerClass::Other,
                "{question}"
            );
        }
    }

    // AC1: salary and eligibility answers are the Preferences values, and the
    // run's own prose for them is dropped rather than stored.
    #[test]
    fn salary_and_eligibility_answers_come_from_preferences_and_are_flagged() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {
                        "question": "What are your salary expectations?",
                        "answer": "I would like USD 9999 per month.",
                        "fact_ids": ["fact_1"]
                    },
                    {
                        "question": "Are you authorized to work in the EU?",
                        "answer": "Yes, absolutely, no doubts.",
                    },
                ],
            }),
            &defaults(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("a valid payload");

        let salary = drafted
            .answer_for("What are your salary expectations?")
            .expect("the salary question is answered");
        assert_eq!(salary.answer.as_deref(), Some("USD 6000 per month"));
        assert_eq!(salary.class, AnswerClass::Salary);
        assert!(salary.needs_review, "salary is always flagged (AC1)");
        assert!(
            salary.fact_ids.is_empty(),
            "the value is a Preference, not a claim about the user"
        );

        let eligibility = drafted
            .answer_for("Are you authorized to work in the EU?")
            .expect("the eligibility question is answered");
        assert_eq!(
            eligibility.answer.as_deref(),
            Some("I am authorized to work in the EU")
        );
        assert!(eligibility.needs_review);

        // Nothing of the run's own words for those two reached the bank.
        for row in repo.answers() {
            let stored = row.answer.unwrap_or_default();
            assert!(!stored.contains("9999"), "{stored}");
            assert!(!stored.contains("absolutely"), "{stored}");
        }
    }

    // AC1: with nothing in Preferences there is no answer, and the question
    // drops to review. The engine never makes one up.
    #[test]
    fn a_sensitive_question_with_no_preference_waits_for_the_user() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {
                        "question": "What are your salary expectations?",
                        "answer": "I would like USD 9999 per month.",
                    },
                ],
            }),
            &Defaults::default(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("a valid payload");

        let answer = drafted.answers.first().expect("one answer");
        assert_eq!(answer.answer, None, "nothing is invented");
        assert!(answer.needs_review);
        assert_eq!(repo.answers()[0].answer, None);
        assert_eq!(repo.answers()[0].approved_at, None);
    }

    // AC1: a free-text answer has to name the profile facts it stands on.
    #[test]
    fn a_free_text_answer_must_stand_on_profile_facts() {
        let repo = repo("app_1");
        let refused = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {"question": "What is your notice period?", "answer": "Four weeks."},
                ],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("an answer with no facts is refused");
        assert!(why(&refused).contains("fact_ids"), "{refused}");

        let invented = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{
                    "question": "What is your notice period?",
                    "answer": "Four weeks.",
                    "fact_ids": ["fact_9"]
                }],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("an answer standing on an unknown fact is refused");
        assert!(why(&invented).contains("fact_9"), "{invented}");
        assert!(repo.answers().is_empty(), "nothing was written");

        // And a grounded answer is stored, one row per question, as a draft.
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{
                    "question": "What is your notice period?",
                    "answer": "Four weeks, per my contract.",
                    "fact_ids": ["fact_1", "fact_2"]
                }],
            }),
            &defaults(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("a grounded answer is valid");

        let stored = repo.answers();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            stored[0].answer.as_deref(),
            Some("Four weeks, per my contract.")
        );
        assert_eq!(stored[0].fact_ids, vec!["fact_1", "fact_2"]);
        assert_eq!(stored[0].class, AnswerClass::Other);
        assert!(!stored[0].needs_review, "a grounded answer needs no review");
        assert!(stored[0].untrusted_origin, "AC3: the run wrote it");
        assert_eq!(stored[0].approved_at, None, "AC2: not reusable yet");
        assert_eq!(
            stored[0].question_key,
            question_key("What is your notice period?")
        );
        assert_eq!(drafted.answers.len(), 1);
    }

    // The payload is refused whole: nothing partial is stored, and the reason
    // names what to fix so the run can retry.
    #[test]
    fn a_payload_the_core_cannot_use_is_refused_whole() {
        let cases = [
            json!("app_1"),
            json!({"answers": []}),
            json!({"app_id": "", "answers": []}),
            json!({"app_id": "app_1"}),
            json!({"app_id": "app_1", "answers": {}}),
            json!({"app_id": "app_1", "answers": ["Four weeks."]}),
            json!({"app_id": "app_1", "answers": [], "note": "hi"}),
            json!({"app_id": "app_1", "answers": [{"answer": "Four weeks."}]}),
            json!({"app_id": "app_1", "answers": [{"question": "", "answer": "x"}]}),
            json!({"app_id": "app_1", "answers": [{"question": "  ", "answer": "x"}]}),
            json!({"app_id": "app_1", "answers": [{"question": "Notice?", "answer": ""}]}),
            json!({"app_id": "app_1", "answers": [{"question": "Notice?", "answer": "  "}]}),
            json!({"app_id": "app_1", "answers": [{"question": "Notice?", "answer": 4}]}),
            json!({"app_id": "app_1", "answers": [{"question": "Notice?", "answer": "x", "confidence": 0.9}]}),
            json!({"app_id": "app_1", "answers": [
                {"question": "Notice?", "answer": "Four weeks.", "fact_ids": ["fact_1"]},
                {"question": "  notice?  ", "answer": "Three weeks.", "fact_ids": ["fact_1"]}
            ]}),
        ];
        for (index, args) in cases.iter().enumerate() {
            let repo = repo("app_1");
            let error = save_answers(&repo, args, &defaults(), &facts(), "t")
                .expect_err(&format!("case {index} must be refused: {args}"));
            assert!(
                error
                    .to_string()
                    .starts_with("this answer payload is not usable:"),
                "{error}"
            );
            assert!(repo.answers().is_empty(), "case {index} wrote something");
        }

        // A free-text question with no answer at all is refused too: a run that
        // skipped a question must say so, not store a blank.
        let repo = repo("app_1");
        let missing = save_answers(
            &repo,
            &json!({"app_id": "app_1", "answers": [{"question": "Notice?"}]}),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("a free-text question without an answer is refused");
        assert!(why(&missing).contains("answer"), "{missing}");
    }

    /// A question and its answer are bounded, so one runaway row cannot bloat
    /// the bank, the run's next prompt or the studio payload.
    #[test]
    fn question_and_answer_text_are_bounded() {
        let repo = repo("app_1");
        let long_question = "q".repeat(MAX_QUESTION_TEXT + 1);
        let error = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{"question": long_question, "answer": "x", "fact_ids": ["fact_1"]}],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("an over-long question is refused");
        assert!(why(&error).contains("question"), "{error}");

        let long_answer = "a".repeat(MAX_ANSWER_TEXT + 1);
        let error = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{"question": "Notice?", "answer": long_answer, "fact_ids": ["fact_1"]}],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("an over-long answer is refused");
        assert!(why(&error).contains("answer"), "{error}");

        // The boundary itself is fine, counted in characters so any language is
        // measured the same way.
        let accented = "á".repeat(MAX_ANSWER_TEXT);
        assert!(
            save_answers(
                &repo,
                &json!({
                    "app_id": "app_1",
                    "answers": [{"question": "Notice?", "answer": accented, "fact_ids": ["fact_1"]}],
                }),
                &defaults(),
                &facts(),
                "t",
            )
            .is_ok()
        );

        // And a payload cannot answer more questions than a form has.
        let many: Vec<serde_json::Value> = (0..=MAX_ANSWERS)
            .map(|n| {
                json!({
                    "question": format!("Question {n}?"),
                    "answer": "Because it is so.",
                    "fact_ids": ["fact_1"]
                })
            })
            .collect();
        let error = save_answers(
            &repo,
            &json!({"app_id": "app_1", "answers": many}),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("too many answers are refused");
        assert!(why(&error).contains("more than"), "{error}");
    }

    /// An application nobody holds cannot be answered for: the rows would hang
    /// off nothing (AD-16, the row's foreign key).
    #[test]
    fn answers_for_an_unknown_application_are_refused() {
        let repo = FakeApplyRepo::default();
        let error = save_answers(
            &repo,
            &json!({
                "app_id": "app_gone",
                "answers": [{
                    "question": "Notice?",
                    "answer": "Four weeks.",
                    "fact_ids": ["fact_1"]
                }],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("no application, no answer");
        assert!(
            matches!(&error, AnswersError::UnknownApplication(app) if app == "app_gone"),
            "{error}"
        );
        assert!(repo.answers().is_empty());
    }

    // AC2/AC3: approving stores the answer for reuse, and the row stops being
    // agent text — the user has made it theirs by confirming it.
    #[test]
    fn approving_an_answer_saves_it_to_the_bank_for_reuse() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{
                    "question": "What is your notice period?",
                    "answer": "Four weeks.",
                    "fact_ids": ["fact_1"]
                }],
            }),
            &defaults(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("a grounded answer");
        let id = drafted.answers[0].id.clone();

        // A draft is not reusable, and it is still agent text.
        assert!(
            reuse(&repo, &[Question::of("What is your notice period?")])
                .expect("reuse")
                .is_empty(),
            "an unapproved answer is not reused"
        );

        approve(&repo, &id, None, "2026-10-16T10:00:00Z").expect("the user confirmed it");

        let approved = reuse(&repo, &[Question::of("What is your notice period?")]).expect("reuse");
        assert_eq!(approved.len(), 1, "the approved answer is reusable");
        assert_eq!(approved[0].answer.as_deref(), Some("Four weeks."));
        assert_eq!(
            approved[0].approved_at.as_deref(),
            Some("2026-10-16T10:00:00Z")
        );
        assert!(
            !approved[0].untrusted_origin,
            "AC3: the user confirmed it, so it is no longer agent text"
        );
        assert!(approved[0].is_reusable());

        // The same question asked on another application reuses it.
        let stored = repo.answers();
        assert_eq!(stored.len(), 1, "one row per answer, not per application");
    }

    /// Approving with an edit stores the user's own words (AC2/AC3).
    #[test]
    fn an_edited_answer_is_stored_as_the_users_own() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{
                    "question": "What are your salary expectations?",
                    "answer": "ignored",
                }],
            }),
            &defaults(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("a valid payload");
        let id = drafted.answers[0].id.clone();

        approve(
            &repo,
            &id,
            Some("USD 7000, negotiable"),
            "2026-10-16T11:00:00Z",
        )
        .expect("the user edited it");

        let stored = repo.answers();
        assert_eq!(stored[0].answer.as_deref(), Some("USD 7000, negotiable"));
        assert!(!stored[0].untrusted_origin);
        assert_eq!(
            stored[0].approved_at.as_deref(),
            Some("2026-10-16T11:00:00Z")
        );
        assert!(stored[0].needs_review, "a salary answer stays flagged");

        // An edit the bank cannot store is refused before anything is written.
        let empty = approve(&repo, &id, Some("   "), "2026-10-16T12:00:00Z")
            .expect_err("an empty edit is refused");
        assert!(matches!(empty, AnswersError::Edit(_)), "{empty}");
        assert_eq!(
            repo.answers()[0].answer.as_deref(),
            Some("USD 7000, negotiable"),
            "the stored answer is untouched"
        );

        let unknown = approve(&repo, "ans_gone", None, "t").expect_err("no such row");
        assert!(
            matches!(&unknown, AnswersError::UnknownAnswer(id) if id == "ans_gone"),
            "{unknown}"
        );
    }

    /// Reuse matches the question, not the words around it: the same question
    /// written two ways reuses one row, a different question never inherits one.
    #[test]
    fn reuse_matches_the_same_question_and_nothing_else() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {
                        "question": "What is your notice period?",
                        "answer": "Four weeks.",
                        "fact_ids": ["fact_1"]
                    },
                    {
                        "question": "Which city are you based in?",
                        "answer": "Berlin.",
                        "fact_ids": ["fact_2"]
                    },
                ],
            }),
            &defaults(),
            &facts(),
            "2026-10-16T09:00:00Z",
        )
        .expect("two grounded answers");
        for answer in &drafted.answers {
            approve(&repo, &answer.id, None, "2026-10-16T10:00:00Z").expect("approve");
        }

        let reused = reuse(
            &repo,
            &[
                Question::of("  what is your   notice period? "),
                Question::of("Have you managed a team?"),
            ],
        )
        .expect("reuse");
        assert_eq!(reused.len(), 1, "only the question that matches");
        assert_eq!(reused[0].answer.as_deref(), Some("Four weeks."));

        // Nothing approved, nothing reused — and a repo that never saw this
        // question holds nothing either.
        let untouched = crate::applications::testing::FakeApplyRepo::default();
        let empty =
            reuse(&untouched, &[Question::of("What is your notice period?")]).expect("reuse");
        assert!(empty.is_empty());
    }

    /// The run's input: the questions with the class the core gave them, the
    /// Preferences defaults, and the answers the user already approved — and
    /// nothing else (no transcript, no drafting notes).
    #[test]
    fn the_run_input_carries_the_questions_the_preferences_and_the_reuse() {
        let repo = repo("app_1");
        let drafted = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [
                    {"question": "Notice?", "answer": "Four weeks.", "fact_ids": ["fact_1"]},
                ],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect("a grounded answer");
        approve(&repo, &drafted.answers[0].id, None, "u").expect("approve");
        let reuse = reuse(&repo, &[Question::of("Notice?")]).expect("reuse");

        let input: serde_json::Value = serde_json::from_str(&answers_input(
            "app_1",
            &[
                Question::of("What are your salary expectations?"),
                Question::of("Notice?"),
            ],
            &defaults(),
            &reuse,
        ))
        .expect("JSON");

        assert_eq!(input["app_id"], "app_1");
        assert_eq!(
            input["questions"][0]["question"],
            "What are your salary expectations?"
        );
        assert_eq!(input["questions"][0]["class"], "salary");
        assert_eq!(input["questions"][1]["class"], "other");
        assert_eq!(input["preferences"]["salary"], "USD 6000 per month");
        assert_eq!(
            input["preferences"]["eligibility"],
            "I am authorized to work in the EU"
        );
        assert_eq!(input["known_answers"][0]["question"], "Notice?");
        assert_eq!(input["known_answers"][0]["answer"], "Four weeks.");
        assert_eq!(
            input.as_object().expect("an object").len(),
            4,
            "the questions, the preferences, the reuse and nothing else: {input}"
        );
    }

    /// A store failure reaches the caller instead of looking like a stored
    /// answer.
    #[test]
    fn a_failed_write_reaches_the_caller() {
        let mut repo = repo("app_1");
        repo.fail = true;
        let error = save_answers(
            &repo,
            &json!({
                "app_id": "app_1",
                "answers": [{
                    "question": "Notice?",
                    "answer": "Four weeks.",
                    "fact_ids": ["fact_1"]
                }],
            }),
            &defaults(),
            &facts(),
            "t",
        )
        .expect_err("the store refused the write");
        assert!(matches!(error, AnswersError::Repo(_)), "{error}");
    }

    /// AC1: the salary default is the user's own Salary Floor, written the way a
    /// form reads it (AD-17 minor units, two decimals).
    #[test]
    fn the_salary_default_is_the_users_own_floor() {
        assert_eq!(
            salary_expectation(600_000, "USD", "month"),
            "USD 6000 per month"
        );
        assert_eq!(
            salary_expectation(600_050, "USD", "month"),
            "USD 6000.50 per month"
        );
        assert_eq!(salary_expectation(4_500, "EUR", "hour"), "EUR 45 per hour");
        assert_eq!(
            salary_expectation(0, "USD", "year"),
            "USD 0 per year",
            "the user may have set a floor of zero; it is still theirs"
        );
    }

    /// AC1: the eligibility default is what the Profile declares, in the run's
    /// language, and there is none when the Profile declares none.
    #[test]
    fn the_eligibility_default_is_what_the_profile_declares() {
        use boba_domain::profile::{FactKind, Lang, ProfileFact, Provenance};

        let fact = |kind: FactKind, en: Option<&str>, es: Option<&str>| ProfileFact {
            id: format!("fact_{}", en.unwrap_or("es")),
            item_id: "item_1".to_string(),
            kind,
            text_en: en.map(str::to_string),
            text_es: es.map(str::to_string),
            evidence: None,
            detail_json: "{}".to_string(),
            supersedes: None,
            profile_version: 1,
            provenance: Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-16T09:00:00Z".to_string(),
        };

        let facts = vec![
            fact(
                FactKind::Eligibility,
                Some("EU citizen"),
                Some("Ciudadanía UE"),
            ),
            fact(FactKind::Eligibility, Some("US green card"), None),
            fact(FactKind::Skill, Some("Rust"), None),
        ];
        assert_eq!(
            eligibility_expectation(&facts, Lang::En).as_deref(),
            Some("EU citizen; US green card")
        );
        assert_eq!(
            eligibility_expectation(&facts, Lang::Es).as_deref(),
            Some("Ciudadanía UE; US green card"),
            "a declaration written in one language is used for the other"
        );
        assert_eq!(
            eligibility_expectation(&facts[2..], Lang::En),
            None,
            "a skill is not an eligibility"
        );
        assert_eq!(eligibility_expectation(&[], Lang::En), None);

        // And through `Defaults`: no floor and no declaration means no answer.
        let none = Defaults::from_user_settings(&[], None);
        assert_eq!(none.salary, None);
        assert_eq!(none.eligibility, None);
        let some = Defaults::from_user_settings(
            &facts,
            Some((600_000, "USD".to_string(), "month".to_string())),
        );
        assert_eq!(some.salary.as_deref(), Some("USD 6000 per month"));
        assert_eq!(
            some.eligibility.as_deref(),
            Some("EU citizen; US green card")
        );
    }
}
