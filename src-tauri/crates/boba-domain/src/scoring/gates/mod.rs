//! Stage 0 of ranking — the five hard Gates (FR-53, AD-8, offer-scoring.md §2).
//!
//! [`run`] is a pure function: one [`Offer`] plus what the Profile and the
//! Preferences declare (FR-7, FR-11..13) in, five [`GateResult`]s out. No clock
//! of its own (the caller injects `now`), no IO, no LLM — the same Offer and
//! inputs always produce the same report (FR-61).
//!
//! Every gate answers with a [`Verdict`], a [`GateReason`] and a [`Confidence`],
//! and with the evidence it read: the exact sentence of the posting
//! ([`QuotedSpan`]) or the Offer field it came from ([`Evidence`]). The quote is
//! what the UI shows next to a VETO (FR-54) and what a human overrides.
//!
//! Two rules the rest of the app relies on:
//!
//! * **silence is never PASS for eligibility** (AC2): a posting that says
//!   nothing about work authorization gets `FLAG` + [`GateReason::Unverified`];
//! * nothing is imputed (AD-8): a Salary Floor in another currency, a missing
//!   salary or an unreadable deadline is `Unverified`, never a guess.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::posting::{Money, Offer, WorkMode};

mod scan;

#[cfg(test)]
mod tests;

pub use scan::QuotedSpan;

/// Which of the five gates a result came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Eligibility,
    Language,
    Location,
    Compensation,
    Deadline,
}

impl GateKind {
    /// Every gate, in report order.
    pub const ALL: [GateKind; 5] = [
        GateKind::Eligibility,
        GateKind::Language,
        GateKind::Location,
        GateKind::Compensation,
        GateKind::Deadline,
    ];

    /// The stored/serialised form.
    pub fn as_str(self) -> &'static str {
        match self {
            GateKind::Eligibility => "eligibility",
            GateKind::Language => "language",
            GateKind::Location => "location",
            GateKind::Compensation => "compensation",
            GateKind::Deadline => "deadline",
        }
    }
}

/// What a gate decided. Ordered worst-last, so the report's verdict is the
/// maximum of its gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The posting allows the application.
    Pass,
    /// Rank it, but tell the user (FR-53).
    Flag,
    /// Excluded from the ranked list until the user overrides (FR-54), with the
    /// reason and the quote.
    Veto,
}

/// Why a gate decided the way it did (one variant per reason the UI shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateReason {
    /// The requirement is stated and met, or there is nothing to check.
    Satisfied,
    /// The posting does not say, and the gate cannot assume (AC2: never PASS).
    Unverified,
    /// A stated requirement the Profile does not meet.
    RequirementNotMet,
    /// The language is declared, but below the bar the posting states.
    BelowRequiredLevel,
    /// Pay under the Salary Floor (FR-53; `VETO` when hard, `FLAG` when soft).
    BelowFloor,
    /// The deadline is inside the "Closing soon" window (FR-52).
    ClosingSoon,
    /// The deadline has passed (FR-52: "Possibly closed").
    Expired,
}

/// How sure the gate is of its own verdict (offer-scoring.md §3: every factor
/// and gate carries one; a missing input lowers it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

/// What a verdict was read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    /// Exact words of the posting, with their offsets (FR-53).
    Quoted(QuotedSpan),
    /// A structured Offer field, by name (`money`, `deadline`, `work_mode`,
    /// `locations`): structured fields have no span in the text.
    Field(String),
}

/// One gate's decision (FR-53).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    pub gate: GateKind,
    pub verdict: Verdict,
    pub reason: GateReason,
    /// `None` when there was nothing to check, such as no Salary Floor set.
    pub evidence: Option<Evidence>,
    pub confidence: Confidence,
}

/// The five gate results for one Offer (FR-53, FR-54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateReport {
    /// One result per gate, always in [`GateKind::ALL`] order.
    pub results: Vec<GateResult>,
}

impl GateReport {
    /// The worst verdict of the five gates: `Veto` beats `Flag` beats `Pass`.
    /// This is what "no gate FLAGs" (auto-apply, FR-93) and the Excluded section
    /// (FR-54) read.
    pub fn verdict(&self) -> Verdict {
        self.results
            .iter()
            .map(|result| result.verdict)
            .max()
            .unwrap_or(Verdict::Pass)
    }

    /// One gate's result, `None` only for a kind a caller invented.
    pub fn gate(&self, kind: GateKind) -> Option<&GateResult> {
        self.results.iter().find(|result| result.gate == kind)
    }
}

/// The kind of authorization a posting asks for, weakest first: a citizen may
/// work where a work permit is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermitKind {
    WorkPermit,
    Residency,
    Citizenship,
    /// A security clearance; it is not covered by, and does not cover, the rest.
    Clearance,
}

impl PermitKind {
    /// The stored/serialised form.
    pub fn as_str(self) -> &'static str {
        match self {
            PermitKind::WorkPermit => "work_permit",
            PermitKind::Residency => "residency",
            PermitKind::Citizenship => "citizenship",
            PermitKind::Clearance => "clearance",
        }
    }

    /// Higher covers lower (a citizen holds a work permit).
    fn rank(self) -> u8 {
        match self {
            PermitKind::WorkPermit => 1,
            PermitKind::Residency => 2,
            PermitKind::Citizenship => 3,
            PermitKind::Clearance => 0,
        }
    }
}

/// One authorization the Profile declares (`FactKind::Eligibility`, FR-11..13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAuthorization {
    /// Lowercase ISO 3166-1 alpha-2 code, or `eu`. Ignored for a clearance.
    pub region: String,
    pub permit: PermitKind,
}

/// A CEFR level (`A1`..`C2`), the only scale the Profile states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cefr {
    A1,
    A2,
    B1,
    B2,
    C1,
    C2,
}

impl Cefr {
    /// Every level, weakest first.
    pub const ALL: [Cefr; 6] = [Cefr::A1, Cefr::A2, Cefr::B1, Cefr::B2, Cefr::C1, Cefr::C2];

    /// The stored/serialised form.
    pub fn as_str(self) -> &'static str {
        match self {
            Cefr::A1 => "a1",
            Cefr::A2 => "a2",
            Cefr::B1 => "b1",
            Cefr::B2 => "b2",
            Cefr::C1 => "c1",
            Cefr::C2 => "c2",
        }
    }

    /// Parse the stored form or what a posting writes (`"C1"`), `None` otherwise.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| level.as_str().eq_ignore_ascii_case(value))
    }
}

/// A language the Profile declares, with its CEFR level (FR-11..13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredLanguage {
    /// BCP-47 tag (`"en"`, `"es"`), as the detector writes it (FR-48).
    pub language: String,
    pub level: Cefr,
}

/// Whether a Salary Floor is a deal-breaker (FR-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hardness {
    Hard,
    Soft,
}

/// The Salary Floor (FR-7, stored as `amount_minor` + currency + period).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SalaryFloor {
    pub money: Money,
    pub hardness: Hardness,
}

/// Everything the Gates read besides the Offer: the Profile facts and the
/// Preferences a run was configured with (FR-7, FR-11..13, S3.7).
///
/// It crosses the scoring port, so it lives here (AD-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateInputs {
    /// Declared work authorizations; empty means nothing declared.
    pub work_authorizations: Vec<WorkAuthorization>,
    /// Declared working languages and their levels.
    pub languages: Vec<DeclaredLanguage>,
    /// The Salary Floor, `None` when the user set none.
    pub salary_floor: Option<SalaryFloor>,
    /// Deal-breaker: an Offer with no posted pay is `VETO`ed rather than flagged.
    pub requires_known_compensation: bool,
    /// Work modes the user accepts; empty means no restriction.
    pub work_modes_allowed: Vec<WorkMode>,
    /// Places the user accepts (`"madrid"`, `"remote"`); empty means anywhere.
    pub locations: Vec<String>,
    /// Whether the user would relocate for the right role.
    pub relocate_ok: bool,
    /// A deadline this many days away or nearer is "Closing soon".
    pub closing_soon_days: i64,
    /// The injected clock, UTC RFC 3339 (AD-17/AD-8: the gates never read one).
    pub now: String,
}

/// Run all five gates (FR-53). Pure and offline (AD-8).
pub fn run(offer: &Offer, inputs: &GateInputs) -> GateReport {
    GateReport {
        results: vec![
            eligibility(offer, inputs),
            language(offer, inputs),
            location(offer, inputs),
            compensation(offer, inputs),
            deadline(offer, inputs),
        ],
    }
}

/// One gate result, so every gate reads the same way.
fn result(
    gate: GateKind,
    verdict: Verdict,
    reason: GateReason,
    evidence: Option<Evidence>,
    confidence: Confidence,
) -> GateResult {
    GateResult {
        gate,
        verdict,
        reason,
        evidence,
        confidence,
    }
}

/// Work eligibility (FR-53, offer-scoring.md §2): a named requirement is read
/// verbatim and compared to what the Profile declares; silence is *unverified*
/// and never PASS (AC2).
fn eligibility(offer: &Offer, inputs: &GateInputs) -> GateResult {
    let demands = scan::demands(&offer.description_text);
    let Some((span, _, _)) = demands.first() else {
        return result(
            GateKind::Eligibility,
            Verdict::Flag,
            GateReason::Unverified,
            None,
            Confidence::Low,
        );
    };
    let met = demands
        .iter()
        .all(|(_, region, permit)| authorized(inputs, region, *permit));
    result(
        GateKind::Eligibility,
        if met { Verdict::Pass } else { Verdict::Veto },
        if met {
            GateReason::Satisfied
        } else {
            GateReason::RequirementNotMet
        },
        Some(Evidence::Quoted(span.clone())),
        Confidence::High,
    )
}

/// Whether the Profile declares an authorization a demand accepts.
fn authorized(inputs: &GateInputs, region: &str, permit: PermitKind) -> bool {
    inputs.work_authorizations.iter().any(|auth| {
        if permit == PermitKind::Clearance {
            return auth.permit == PermitKind::Clearance;
        }
        auth.permit != PermitKind::Clearance
            && auth.permit.rank() >= permit.rank()
            && auth.region.trim().eq_ignore_ascii_case(region)
    })
}

/// Language (FR-53): a required language nobody declared is a VETO; a declared
/// level under the stated bar is a FLAG (rank it, warn).
fn language(offer: &Offer, inputs: &GateInputs) -> GateResult {
    let required = scan::required_languages(&offer.description_text);
    for (span, tag, bar) in &required {
        let declared = inputs
            .languages
            .iter()
            .find(|declared| declared.language.eq_ignore_ascii_case(tag));
        match declared {
            None => {
                return result(
                    GateKind::Language,
                    Verdict::Veto,
                    GateReason::RequirementNotMet,
                    Some(Evidence::Quoted(span.clone())),
                    Confidence::High,
                );
            }
            Some(declared) if bar.is_some_and(|bar| declared.level < bar) => {
                return result(
                    GateKind::Language,
                    Verdict::Flag,
                    GateReason::BelowRequiredLevel,
                    Some(Evidence::Quoted(span.clone())),
                    Confidence::Medium,
                );
            }
            Some(_) => {}
        }
    }
    let evidence = required
        .first()
        .map(|(span, _, _)| Evidence::Quoted(span.clone()));
    result(
        GateKind::Language,
        Verdict::Pass,
        GateReason::Satisfied,
        evidence.clone(),
        match evidence {
            // Levels are read from words like "fluent" (offer-scoring.md §2).
            Some(_) => Confidence::Medium,
            None => Confidence::High,
        },
    )
}

/// Location and work mode (FR-53): remote always passes; an onsite/hybrid role
/// the user cannot reach, and is not willing to relocate for, is a VETO.
fn location(offer: &Offer, inputs: &GateInputs) -> GateResult {
    let mode = Some(Evidence::Field("work_mode".to_string()));
    let places = Some(Evidence::Field("locations".to_string()));
    if offer.work_mode == WorkMode::Unknown {
        return result(
            GateKind::Location,
            Verdict::Flag,
            GateReason::Unverified,
            mode,
            Confidence::Low,
        );
    }
    if !inputs.work_modes_allowed.is_empty()
        && !inputs.work_modes_allowed.contains(&offer.work_mode)
    {
        return result(
            GateKind::Location,
            Verdict::Veto,
            GateReason::RequirementNotMet,
            mode,
            Confidence::High,
        );
    }
    if offer.work_mode == WorkMode::Remote {
        return result(
            GateKind::Location,
            Verdict::Pass,
            GateReason::Satisfied,
            mode,
            Confidence::High,
        );
    }
    let reachable = inputs.locations.is_empty()
        || accepts_location(&offer.locations, &inputs.locations)
        || inputs.relocate_ok;
    result(
        GateKind::Location,
        if reachable {
            Verdict::Pass
        } else {
            Verdict::Veto
        },
        if reachable {
            GateReason::Satisfied
        } else {
            GateReason::RequirementNotMet
        },
        places,
        Confidence::Medium,
    )
}

/// Whether any posted place is one the user accepts. Token containment after
/// folding: `"Madrid, Spain"` accepts `"madrid"`.
///
/// shortcut: string containment, no geodistance or commute range; upgrade when
/// commute/travel tolerance lands (offer-scoring.md §3 F4).
fn accepts_location(posted: &[String], accepted: &[String]) -> bool {
    posted.iter().any(|place| {
        let place = place.trim().to_lowercase();
        accepted.iter().any(|wanted| {
            let wanted = wanted.trim().to_lowercase();
            !wanted.is_empty() && (place == wanted || place.contains(&wanted))
        })
    })
}

/// Compensation floor (FR-53, FR-7): pay is compared to the Salary Floor when
/// both are known and comparable; a soft floor FLAGs, a hard floor VETOes.
fn compensation(offer: &Offer, inputs: &GateInputs) -> GateResult {
    let field = Some(Evidence::Field("money".to_string()));
    let Some(floor) = &inputs.salary_floor else {
        // No floor set: nothing to check, unless the user demands posted pay.
        return if offer.money.is_none() && inputs.requires_known_compensation {
            result(
                GateKind::Compensation,
                Verdict::Veto,
                GateReason::RequirementNotMet,
                field,
                Confidence::Medium,
            )
        } else {
            result(
                GateKind::Compensation,
                Verdict::Pass,
                GateReason::Satisfied,
                None,
                Confidence::High,
            )
        };
    };
    let Some(posted) = &offer.money else {
        return if inputs.requires_known_compensation {
            result(
                GateKind::Compensation,
                Verdict::Veto,
                GateReason::RequirementNotMet,
                field,
                Confidence::Medium,
            )
        } else {
            result(
                GateKind::Compensation,
                Verdict::Flag,
                GateReason::Unverified,
                field,
                Confidence::Low,
            )
        };
    };
    if !comparable(posted, &floor.money) {
        // No FX rates and no period maths here: never imputed (AD-8).
        return result(
            GateKind::Compensation,
            Verdict::Flag,
            GateReason::Unverified,
            field,
            Confidence::Medium,
        );
    }
    if posted.amount_minor >= floor.money.amount_minor {
        return result(
            GateKind::Compensation,
            Verdict::Pass,
            GateReason::Satisfied,
            field,
            Confidence::High,
        );
    }
    result(
        GateKind::Compensation,
        match floor.hardness {
            Hardness::Hard => Verdict::Veto,
            Hardness::Soft => Verdict::Flag,
        },
        GateReason::BelowFloor,
        field,
        Confidence::High,
    )
}

/// Same currency and same period, so the amounts can be compared directly.
fn comparable(posted: &Money, floor: &Money) -> bool {
    posted.currency == floor.currency && posted.period == floor.period
}

/// Deadline (FR-52/FR-53): expired is a VETO, closing soon a FLAG, and a
/// deadline no clock can read is unverified rather than assumed.
fn deadline(offer: &Offer, inputs: &GateInputs) -> GateResult {
    let Some(posted) = &offer.deadline else {
        return result(
            GateKind::Deadline,
            Verdict::Pass,
            GateReason::Satisfied,
            None,
            Confidence::High,
        );
    };
    let field = Some(Evidence::Field("deadline".to_string()));
    let (Some(due), Some(now)) = (instant(posted), instant(&inputs.now)) else {
        return result(
            GateKind::Deadline,
            Verdict::Flag,
            GateReason::Unverified,
            field,
            Confidence::Low,
        );
    };
    if due <= now {
        return result(
            GateKind::Deadline,
            Verdict::Veto,
            GateReason::Expired,
            field,
            Confidence::High,
        );
    }
    let window = time::Duration::days(inputs.closing_soon_days.max(0));
    if due - now <= window {
        return result(
            GateKind::Deadline,
            Verdict::Flag,
            GateReason::ClosingSoon,
            field,
            Confidence::High,
        );
    }
    result(
        GateKind::Deadline,
        Verdict::Pass,
        GateReason::Satisfied,
        field,
        Confidence::High,
    )
}

/// A UTC instant from an RFC 3339 string, `None` when it is not one (AD-17).
fn instant(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}
