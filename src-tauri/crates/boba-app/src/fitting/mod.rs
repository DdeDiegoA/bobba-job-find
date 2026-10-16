//! Relevance-weighted fitting (S7.5, FR-76, AD-14).
//!
//! A tailored CV that lays out past its page budget is cut back to size: every
//! line is scored for **relevance** (how well it answers the Offer), **uniqueness**
//! (how little another line already says the same thing) and **cover-letter
//! dependency** (whether the letter leans on it), and the lowest-scoring line goes
//! first — never "oldest first" (upstream rule, research §3). What went is kept, so
//! the studio can show it (AC1).
//!
//! Measurement comes from [`VerifyPort`], so this use case never sees Typst and
//! never renders a PDF: it asks how the draft *would* lay out (AD-1, AD-14).

use boba_domain::documents::{
    Claim, CoverLetterDocument, CvDocument, DocumentModel, LayoutReport, TemplateId,
};
use thiserror::Error;

use crate::ports::verify::{VerifyError, VerifyPort};

/// The CV page budget (FR-76): two pages by default, 1–3 configurable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageBudget(u32);

impl PageBudget {
    /// The budget a user who has not changed it gets.
    pub const DEFAULT: Self = Self(2);

    /// A budget the user configured. `None` outside 1–3: FR-76 allows no other.
    pub fn new(pages: u32) -> Option<Self> {
        (1..=3).contains(&pages).then_some(Self(pages))
    }

    /// The pages a CV may take.
    pub const fn pages(self) -> u32 {
        self.0
    }
}

/// A line the fit removed, as the studio shows it (AC1: "what was cut is shown").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct CutLine {
    /// The section the line came from; `None` when it was the summary.
    pub section: Option<String>,
    /// The line itself.
    pub text: String,
    /// The score it was cut on; lower goes first.
    pub score: f32,
}

/// A fitted CV: the draft to render, what it lays out to, and what was cut.
#[derive(Debug, Clone, PartialEq)]
pub struct FitOutcome {
    /// The CV to render. It lays out within its budget.
    pub cv: CvDocument,
    /// What the fitted CV laid out to. An `orphans` count here is a heading the
    /// cut could not help; the studio flags it (S7.7).
    pub report: LayoutReport,
    /// The lines that were removed, in the order they went (lowest scored first).
    pub cut: Vec<CutLine>,
}

/// Why a CV could not be fitted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FitError {
    /// The layout could not be measured.
    #[error(transparent)]
    Verify(#[from] VerifyError),
    /// Every cuttable line went and the CV still lays out past its budget.
    #[error("the CV still lays out to {pages} page(s) after cutting every line")]
    CannotFit { pages: u32 },
}

/// Cut `cv` down to `budget` pages, lowest-scoring lines first (FR-76, AC1).
///
/// `relevance` scores a line against the Offer (0.0–1.0); the scoring model lands
/// in S6.3, so the run passes the signal it has. `cover` is the letter that ships
/// with this CV: a line it also uses is cut last, so a cut cannot open a hole in
/// both documents. Cutting a section's last line removes the section, so the fit
/// never leaves a heading behind (the thing the layout pass counts as an orphan).
pub fn fit_cv(
    cv: &CvDocument,
    cover: &CoverLetterDocument,
    template: &TemplateId,
    budget: PageBudget,
    relevance: &dyn Fn(&Claim) -> f32,
    verify: &dyn VerifyPort,
) -> Result<FitOutcome, FitError> {
    let mut fitted = cv.clone();
    let mut cut = Vec::new();
    let mut report = measure(&fitted, template, verify)?;
    while report.pages > budget.pages() {
        let Some((slot, claim, score)) = weakest(&fitted, cover, relevance) else {
            return Err(FitError::CannotFit {
                pages: report.pages,
            });
        };
        cut.push(CutLine {
            section: slot.heading(&fitted),
            text: claim.text,
            score,
        });
        slot.remove(&mut fitted);
        report = measure(&fitted, template, verify)?;
    }
    Ok(FitOutcome {
        cv: fitted,
        report,
        cut,
    })
}

/// Measure a cover letter. Its rule is [`cover_fits`] (FR-76, AC2).
pub fn verify_cover(
    cover: &CoverLetterDocument,
    template: &TemplateId,
    verify: &dyn VerifyPort,
) -> Result<LayoutReport, VerifyError> {
    verify.layout(&DocumentModel::CoverLetter(cover.clone()), template)
}

/// The one-page rule (FR-76, AC2): exactly one page, with no heading stranded at
/// the bottom of it and nothing overflowing.
pub fn cover_fits(report: &LayoutReport) -> bool {
    report.pages == 1 && report.orphans == 0
}

/// Where a cuttable line sits in the CV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The summary, above every section.
    Summary,
    /// One line of one section.
    Item { section: usize, item: usize },
}

impl Slot {
    /// The section this line belongs to; `None` for the summary.
    fn heading(self, cv: &CvDocument) -> Option<String> {
        match self {
            Slot::Summary => None,
            Slot::Item { section, .. } => cv.sections.get(section).map(|s| s.heading.clone()),
        }
    }

    /// Take the line out. A section that loses its last line goes with it: a
    /// heading with nothing under it is the orphan the layout pass counts.
    fn remove(self, cv: &mut CvDocument) {
        match self {
            Slot::Summary => cv.summary = None,
            Slot::Item { section, item } => {
                cv.sections[section].items.remove(item);
                if cv.sections[section].items.is_empty() {
                    cv.sections.remove(section);
                }
            }
        }
    }
}

/// Every line the fit may cut, in document order.
fn slots(cv: &CvDocument) -> Vec<Slot> {
    let mut slots = Vec::new();
    if cv.summary.is_some() {
        slots.push(Slot::Summary);
    }
    for (section, item) in cv
        .sections
        .iter()
        .enumerate()
        .flat_map(|(section, section_value)| {
            (0..section_value.items.len()).map(move |item| (section, item))
        })
    {
        slots.push(Slot::Item { section, item });
    }
    slots
}

/// The line to cut: the lowest score, and on a tie the later line, so the earlier
/// content of the draft is what survives.
fn weakest(
    cv: &CvDocument,
    cover: &CoverLetterDocument,
    relevance: &dyn Fn(&Claim) -> f32,
) -> Option<(Slot, Claim, f32)> {
    let slots = slots(cv);
    let lines: Vec<&Claim> = slots
        .iter()
        .map(|slot| match slot {
            Slot::Summary => cv.summary.as_ref(),
            Slot::Item { section, item } => cv.sections[*section].items.get(*item),
        })
        .collect::<Option<Vec<_>>>()?;

    let mut weakest: Option<(Slot, Claim, f32)> = None;
    for (slot, claim) in slots.into_iter().zip(&lines) {
        let score = relevance(claim)
            + uniqueness(claim, &lines)
            + if cover_dependency(claim, cover) {
                1.0
            } else {
                0.0
            };
        if weakest
            .as_ref()
            .is_none_or(|(_, _, lowest)| score <= *lowest)
        {
            weakest = Some((slot, (*claim).clone(), score));
        }
    }
    weakest
}

/// How little the rest of the document already says this line (0.0–1.0, 1.0 = the
/// line is the only one of its kind).
fn uniqueness(claim: &Claim, lines: &[&Claim]) -> f32 {
    let own = words(&claim.text);
    let mut most_similar = 0.0_f32;
    for other in lines {
        if std::ptr::eq(*other, claim) {
            continue;
        }
        most_similar = most_similar.max(jaccard(&own, &words(&other.text)));
    }
    1.0 - most_similar
}

/// Whether the cover letter leans on this line: it reuses the line's text, or is
/// built from one of the same profile facts (FR-76's cover-letter dependency).
fn cover_dependency(claim: &Claim, cover: &CoverLetterDocument) -> bool {
    cover.paragraphs.iter().any(|paragraph| {
        paragraph.text == claim.text
            || (!claim.fact_ids.is_empty()
                && paragraph
                    .fact_ids
                    .iter()
                    .any(|fact| claim.fact_ids.contains(fact)))
    })
}

/// The comparable words of a line: lowercase, three characters or more, so
/// punctuation and filler ("a", "of") do not make a line look unique.
fn words(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect()
}

/// How much two lines say the same thing (0.0–1.0).
fn jaccard(a: &std::collections::HashSet<String>, b: &std::collections::HashSet<String>) -> f32 {
    let shared = a.intersection(b).count();
    if shared == 0 {
        return 0.0;
    }
    shared as f32 / (a.len() + b.len() - shared) as f32
}

/// Ask the adapter how a draft lays out (AD-14: the core never renders itself).
fn measure(
    cv: &CvDocument,
    template: &TemplateId,
    verify: &dyn VerifyPort,
) -> Result<LayoutReport, FitError> {
    Ok(verify.layout(&DocumentModel::Cv(cv.clone()), template)?)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use boba_domain::documents::{
        Contact, Lang, RenderedPdf, SCHEMA_VERSION, Section, TextLayerReport,
    };

    fn claim(text: &str, facts: &[&str]) -> Claim {
        Claim {
            text: text.to_string(),
            fact_ids: facts.iter().map(|f| (*f).to_string()).collect(),
        }
    }

    fn contact() -> Contact {
        Contact {
            full_name: "Ada Lovelace".into(),
            headline: None,
            email: None,
            phone: None,
            location: None,
            links: vec![],
        }
    }

    fn cv_with(sections: &[(&str, &[&str])], summary: Option<&str>) -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            summary: summary.map(|text| claim(text, &["fact_summary"])),
            sections: sections
                .iter()
                .map(|(heading, items)| Section {
                    heading: (*heading).to_string(),
                    items: items
                        .iter()
                        .map(|text| claim(text, &["fact_line"]))
                        .collect(),
                })
                .collect(),
            declared_gaps: vec![],
        }
    }

    fn cover_of(paragraphs: &[(&str, &[&str])]) -> CoverLetterDocument {
        CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            company: Some("Acme".into()),
            recipient: None,
            date: None,
            salutation: None,
            paragraphs: paragraphs
                .iter()
                .map(|(text, facts)| claim(text, facts))
                .collect(),
            closing: None,
            declared_gaps: vec![],
        }
    }

    fn template() -> TemplateId {
        TemplateId::new("cv-classic")
    }

    /// A `VerifyPort` that lays out `per_page` lines to a page (test-only: no
    /// Typst, no files, no user data).
    #[derive(Default)]
    struct FakeVerify {
        per_page: u32,
        orphans: u32,
        calls: AtomicU32,
    }

    impl FakeVerify {
        fn calls(&self) -> u32 {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl VerifyPort for FakeVerify {
        fn layout(
            &self,
            document: &DocumentModel,
            _template: &TemplateId,
        ) -> Result<LayoutReport, VerifyError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let lines = match document {
                DocumentModel::Cv(cv) => {
                    u32::from(cv.summary.is_some())
                        + cv.sections
                            .iter()
                            .map(|section| section.items.len() as u32)
                            .sum::<u32>()
                }
                DocumentModel::CoverLetter(cover) => cover.paragraphs.len() as u32,
            };
            Ok(LayoutReport {
                pages: lines.div_ceil(self.per_page.max(1)).max(1),
                orphans: self.orphans,
            })
        }

        // S7.6's ATS check: these tests are about cutting, so the fake reports a
        // layer nothing failed in. `boba-docs` tests the check itself on real
        // rendered PDFs.
        fn text_layer(
            &self,
            _document: &DocumentModel,
            _rendered: &RenderedPdf,
            _required_keywords: &[String],
        ) -> Result<TextLayerReport, VerifyError> {
            Ok(TextLayerReport::default())
        }
    }

    /// A verifier whose pages never come down: the CV cannot be fitted.
    struct StuckVerify;

    impl VerifyPort for StuckVerify {
        fn layout(
            &self,
            _document: &DocumentModel,
            _template: &TemplateId,
        ) -> Result<LayoutReport, VerifyError> {
            Ok(LayoutReport {
                pages: 9,
                orphans: 0,
            })
        }

        fn text_layer(
            &self,
            _document: &DocumentModel,
            _rendered: &RenderedPdf,
            _required_keywords: &[String],
        ) -> Result<TextLayerReport, VerifyError> {
            Ok(TextLayerReport::default())
        }
    }

    /// The texts of a section, in order.
    fn lines(cv: &CvDocument, section: usize) -> Vec<String> {
        cv.sections[section]
            .items
            .iter()
            .map(|item| item.text.clone())
            .collect()
    }

    #[test]
    fn a_cv_within_budget_is_left_alone() {
        let cv = cv_with(
            &[("Experience", &["Shipped the API", "Wrote the docs"])],
            None,
        );
        let verify = FakeVerify {
            per_page: 4,
            ..Default::default()
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::DEFAULT,
            &|_| 0.5,
            &verify,
        )
        .expect("two lines fit the default budget");

        assert_eq!(fitted.cv, cv, "nothing is cut when it already fits");
        assert!(fitted.cut.is_empty());
        assert_eq!(fitted.report.pages, 1);
        assert_eq!(verify.calls(), 1, "one measurement, no cutting");
    }

    #[test]
    fn cutting_goes_by_score_not_by_age() {
        // The first line is the least relevant one: "oldest first" would keep it.
        let cv = cv_with(
            &[(
                "Experience",
                &[
                    "Filled out status reports",
                    "Led the billing migration",
                    "Shipped the billing API",
                ],
            )],
            None,
        );
        let verify = FakeVerify {
            per_page: 2,
            ..Default::default()
        };
        let relevance = |claim: &Claim| {
            if claim.text.starts_with("Filled out") {
                0.0
            } else {
                1.0
            }
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &relevance,
            &verify,
        )
        .expect("one cut is enough");

        assert_eq!(fitted.cut.len(), 1);
        assert_eq!(fitted.cut[0].text, "Filled out status reports");
        assert_eq!(fitted.report.pages, 1);
        assert_eq!(
            lines(&fitted.cv, 0),
            ["Led the billing migration", "Shipped the billing API"],
            "the later, relevant lines stay"
        );
    }

    #[test]
    fn a_line_another_line_already_says_goes_first() {
        let cv = cv_with(
            &[(
                "Experience",
                &["Shipped the API", "Shipped the API", "Wrote the docs"],
            )],
            None,
        );
        let verify = FakeVerify {
            per_page: 2,
            ..Default::default()
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &|_| 0.5,
            &verify,
        )
        .expect("one cut is enough");

        assert_eq!(
            fitted.cut.len(),
            1,
            "the duplicate goes, the distinct line stays"
        );
        assert_eq!(fitted.cut[0].text, "Shipped the API");
        assert_eq!(lines(&fitted.cv, 0), ["Shipped the API", "Wrote the docs"]);
    }

    #[test]
    fn a_line_the_cover_letter_reuses_is_cut_last() {
        let mut cv = cv_with(&[("Experience", &[])], None);
        cv.sections[0].items = vec![
            claim("Shipped the billing API", &["fact_api"]),
            claim("Wrote the release notes", &["fact_docs"]),
        ];
        // The letter leans on the API line only (FR-76's cover-letter dependency).
        let cover = cover_of(&[("I shipped the billing API at Acme.", &["fact_api"])]);
        let verify = FakeVerify {
            per_page: 1,
            ..Default::default()
        };

        let fitted = fit_cv(
            &cv,
            &cover,
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &|_| 0.5,
            &verify,
        )
        .expect("one cut is enough");

        assert_eq!(fitted.cut.len(), 1);
        assert_eq!(fitted.cut[0].text, "Wrote the release notes");
        assert_eq!(lines(&fitted.cv, 0), ["Shipped the billing API"]);
    }

    #[test]
    fn cutting_a_sections_last_line_removes_its_heading() {
        let cv = cv_with(
            &[
                ("Experience", &["Shipped the billing API"]),
                ("Education", &["BSc in Computer Science"]),
            ],
            None,
        );
        let verify = FakeVerify {
            per_page: 1,
            ..Default::default()
        };
        let relevance = |claim: &Claim| {
            if claim.text.starts_with("BSc") {
                0.0
            } else {
                1.0
            }
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &relevance,
            &verify,
        )
        .expect("one cut is enough");

        assert_eq!(fitted.cut.len(), 1);
        assert_eq!(fitted.cut[0].section.as_deref(), Some("Education"));
        assert_eq!(
            fitted
                .cv
                .sections
                .iter()
                .map(|section| section.heading.as_str())
                .collect::<Vec<_>>(),
            ["Experience"],
            "a heading with no line under it would be the orphan the verifier counts"
        );
    }

    #[test]
    fn what_was_cut_carries_its_text_and_section() {
        let cv = cv_with(
            &[("Experience", &["Shipped the billing API"])],
            Some("Backend developer with six years of experience"),
        );
        let verify = FakeVerify {
            per_page: 1,
            ..Default::default()
        };
        let relevance = |claim: &Claim| {
            if claim.text.starts_with("Backend") {
                0.0
            } else {
                1.0
            }
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &relevance,
            &verify,
        )
        .expect("one cut is enough");

        assert_eq!(fitted.cut.len(), 1);
        assert_eq!(
            fitted.cut[0].section, None,
            "the summary belongs to no section"
        );
        assert_eq!(
            fitted.cut[0].text,
            "Backend developer with six years of experience"
        );
        assert!(
            (fitted.cut[0].score - 1.0).abs() < 1e-6,
            "relevance 0.0 + a unique line 1.0, got {}",
            fitted.cut[0].score
        );
        assert!(fitted.cv.summary.is_none());
        assert_eq!(lines(&fitted.cv, 0), ["Shipped the billing API"]);
    }

    #[test]
    fn every_cut_is_measured_and_the_fit_stops_when_it_fits() {
        let cv = cv_with(
            &[(
                "Experience",
                &["One", "Two", "Three", "Four but much longer than the rest"],
            )],
            None,
        );
        let verify = FakeVerify {
            per_page: 2,
            ..Default::default()
        };

        let fitted = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &|_| 0.5,
            &verify,
        )
        .expect("two cuts are enough");

        assert_eq!(fitted.cut.len(), 2);
        assert_eq!(lines(&fitted.cv, 0), ["One", "Two"]);
        assert_eq!(
            verify.calls(),
            3,
            "once up front, once after each cut, and not again"
        );
    }

    #[test]
    fn a_cv_that_cannot_be_fitted_is_an_error() {
        let cv = cv_with(
            &[("Experience", &["Shipped the API", "Wrote the docs"])],
            None,
        );

        let error = fit_cv(
            &cv,
            &cover_of(&[]),
            &template(),
            PageBudget::new(1).expect("1 page is a budget"),
            &|_| 0.5,
            &StuckVerify,
        )
        .expect_err("every line went and it is still 9 pages");

        assert_eq!(error, FitError::CannotFit { pages: 9 });
    }

    #[test]
    fn a_budget_outside_one_to_three_is_not_a_budget() {
        assert_eq!(PageBudget::DEFAULT.pages(), 2);
        for pages in 1..=3 {
            assert_eq!(
                PageBudget::new(pages).map(PageBudget::pages),
                Some(pages),
                "FR-76 configures 1–3"
            );
        }
        assert_eq!(PageBudget::new(0), None);
        assert_eq!(PageBudget::new(4), None);
    }

    #[test]
    fn the_cover_letter_rule_is_one_page_and_no_stranded_heading() {
        assert!(cover_fits(&LayoutReport {
            pages: 1,
            orphans: 0
        }));
        assert!(
            !cover_fits(&LayoutReport {
                pages: 2,
                orphans: 0
            }),
            "a letter that overflows one page"
        );
        assert!(
            !cover_fits(&LayoutReport {
                pages: 1,
                orphans: 1
            }),
            "a heading stranded at the bottom of the page"
        );
    }

    #[test]
    fn verify_cover_measures_the_letter() {
        let cover = cover_of(&[
            ("Dear hiring team,", &["fact_1"]),
            ("I would love to join Acme.", &["fact_2"]),
        ]);
        let verify = FakeVerify {
            per_page: 1,
            ..Default::default()
        };

        let report = verify_cover(&cover, &TemplateId::new("cover-classic"), &verify)
            .expect("measure the letter");

        assert_eq!(report.pages, 2);
        assert!(
            !cover_fits(&report),
            "two paragraphs do not fit one page here"
        );
    }
}
