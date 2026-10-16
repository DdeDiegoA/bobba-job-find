//! Layout verification (S7.5, FR-76, AD-14).
//!
//! The renderer (S7.1/S7.2) reports how many pages a draft takes; this pass lays
//! the same embedded template out and reads the frames, which is where a section
//! heading stranded at the bottom of a page becomes visible (§9: page count and
//! frame positions, never a parsed PDF).
//!
//! Measurement writes no file and renders no PDF: the core asks how a draft
//! *would* lay out, then `boba_app::fitting` decides what to cut. `boba-docs`
//! implements `VerifyPort` here (AD-14, §4.4).

// `::typst` is the compiler crate: the crate has a module of the same name.
use ::typst::layout::{Frame, FrameItem};
use typst_layout::PagedDocument;

use boba_app::ports::verify::{VerifyError, VerifyPort};
use boba_domain::documents::{DocumentModel, LayoutReport, SCHEMA_VERSION, TemplateId};

use crate::TypstRenderer;
use crate::typst::Template;

impl VerifyPort for TypstRenderer {
    fn layout(
        &self,
        document: &DocumentModel,
        template: &TemplateId,
    ) -> Result<LayoutReport, VerifyError> {
        layout_facts(document, template.as_str())
    }
}

/// What `document` lays out to as `template` (FR-76).
///
/// Fails when the template is unknown, belongs to the other kind, or the
/// document's schema is newer than this build knows.
pub fn layout_facts(document: &DocumentModel, template: &str) -> Result<LayoutReport, VerifyError> {
    let template = Template::parse(template)
        .ok_or_else(|| VerifyError::UnknownTemplate(template.to_string()))?;
    if template.kind() != document.kind() {
        return Err(VerifyError::WrongTemplate {
            template: template.name().to_string(),
            kind: document.kind(),
        });
    }
    if document.schema_version() != SCHEMA_VERSION {
        return Err(VerifyError::UnsupportedSchema {
            version: document.schema_version(),
            expected: SCHEMA_VERSION,
        });
    }

    // The templates read the draft itself, the way the renderer hands it over.
    let json = match document {
        DocumentModel::Cv(cv) => serde_json::to_string(cv),
        DocumentModel::CoverLetter(cover) => serde_json::to_string(cover),
    }
    .map_err(|e| VerifyError::Measure(e.to_string()))?;

    let paged = crate::typst::compile_paged(template, &json).map_err(convert)?;
    Ok(LayoutReport {
        pages: u32::try_from(paged.pages().len()).unwrap_or(u32::MAX),
        orphans: count_orphans(&paged, &headings(document)),
    })
}

/// The renderer's error, as this port reports it.
fn convert(error: crate::typst::RenderError) -> VerifyError {
    use crate::typst::RenderError;
    match error {
        RenderError::UnsupportedSchema(version) => VerifyError::UnsupportedSchema {
            version,
            expected: SCHEMA_VERSION,
        },
        RenderError::WrongTemplate { template, kind } => VerifyError::WrongTemplate {
            template: template.to_string(),
            kind,
        },
        RenderError::Compile(message) => VerifyError::Measure(message),
        RenderError::Json(error) => VerifyError::Measure(error.to_string()),
        RenderError::Pdf(message) => VerifyError::Measure(message),
    }
}

/// The section headings of a draft. A cover letter has none, so nothing in it can
/// be stranded.
fn headings(document: &DocumentModel) -> Vec<String> {
    match document {
        DocumentModel::Cv(cv) => cv
            .sections
            .iter()
            .map(|section| section.heading.trim().to_string())
            .collect(),
        DocumentModel::CoverLetter(_) => Vec::new(),
    }
}

/// How many pages end on a section heading: its lines all started on the next
/// page, so the heading is stranded at the bottom of this one (FR-76, AC2).
///
/// shortcut: a heading is recognised by its text, because the templates do not
/// label headings; a body line whose whole text equals a heading would count too.
fn count_orphans(paged: &PagedDocument, headings: &[String]) -> u32 {
    paged
        .pages()
        .iter()
        .filter(|page| {
            last_text(&page.frame).is_some_and(|text| headings.iter().any(|h| *h == text.trim()))
        })
        .count()
        .try_into()
        .unwrap_or(u32::MAX)
}

/// The last text run laid out in a frame, in layout order.
fn last_text(frame: &Frame) -> Option<String> {
    let mut last = None;
    collect_last_text(frame, &mut last);
    last
}

fn collect_last_text(frame: &Frame, last: &mut Option<String>) {
    for (_, item) in frame.items() {
        match item {
            FrameItem::Text(text) => *last = Some(text.text.to_string()),
            // Bullet lists and other blocks lay their text out in a subframe.
            FrameItem::Group(group) => collect_last_text(&group.frame, last),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use boba_app::fitting::{PageBudget, cover_fits, fit_cv, verify_cover};
    use boba_domain::documents::{
        Claim, Contact, CoverLetterDocument, CvDocument, DocumentKind, Lang, Section,
    };

    use super::*;

    fn claim(text: impl Into<String>) -> Claim {
        Claim {
            text: text.into(),
            fact_ids: vec!["fact_1".into()],
        }
    }

    fn contact() -> Contact {
        Contact {
            full_name: "Ada Lovelace".into(),
            headline: Some("Backend developer".into()),
            email: Some("ada@example.com".into()),
            phone: None,
            location: Some("Bogotá, Colombia".into()),
            links: vec![],
        }
    }

    fn cv(sections: Vec<(&str, Vec<String>)>) -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            summary: Some(claim(
                "Backend developer with six years of experience in billing systems.",
            )),
            sections: sections
                .into_iter()
                .map(|(heading, items)| Section {
                    heading: heading.into(),
                    items: items.into_iter().map(claim).collect(),
                })
                .collect(),
            declared_gaps: vec![],
        }
    }

    fn cover(paragraphs: Vec<String>) -> CoverLetterDocument {
        CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            company: Some("Acme".into()),
            recipient: Some("Hiring team".into()),
            date: Some("5 October 2026".into()),
            salutation: Some("Dear hiring team,".into()),
            paragraphs: paragraphs.into_iter().map(claim).collect(),
            closing: Some("Sincerely, Ada Lovelace".into()),
            declared_gaps: vec![],
        }
    }

    /// Filler lines a page cannot hold many of: each one is about three lines.
    fn long_lines(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                format!(
                    "Shipped the billing pipeline number {i} in Rust and Postgres, cutting p99 latency by 40% across three regions."
                )
            })
            .collect()
    }

    /// A page holds 22 of these; each one is about two lines.
    fn two_line_lines(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| {
                format!("Shipped the billing pipeline number {i} in Rust and Postgres, cutting p99 latency by 40% across three regions. Shipped the billing pipeline number {i} in Rust and Postgres, cutting p99 latency by 40% across three regions.")
            })
            .collect()
    }

    fn measure(document: &DocumentModel) -> LayoutReport {
        let template = match document {
            DocumentModel::Cv(_) => "cv-classic",
            DocumentModel::CoverLetter(_) => "cover-classic",
        };
        layout_facts(document, template).expect("the default templates always measure")
    }

    // FR-76: a heading nothing follows is stranded, and the pass counts it.
    #[test]
    fn a_heading_with_no_lines_under_it_is_a_stranded_heading() {
        let bare = cv(vec![("Experience", long_lines(40)), ("Projects", vec![])]);
        let report = measure(&DocumentModel::Cv(bare));
        assert_eq!(report.orphans, 1, "the page ends on the bare heading");

        let filled = cv(vec![
            ("Experience", long_lines(40)),
            ("Projects", vec!["Built the boba scheduler".into()]),
        ]);
        assert_eq!(
            measure(&DocumentModel::Cv(filled)).orphans,
            0,
            "the heading keeps its line, so nothing is stranded"
        );
    }

    // FR-76: a CV that runs long is measured, not guessed at.
    #[test]
    fn a_long_cv_is_measured_across_pages() {
        let report = measure(&DocumentModel::Cv(cv(vec![("Experience", long_lines(40))])));
        assert!(
            report.pages >= 1,
            "every document lays out to at least one page"
        );
        assert_eq!(report.orphans, 0);

        let long = measure(&DocumentModel::Cv(cv(vec![("Experience", long_lines(60))])));
        assert!(
            long.pages > report.pages,
            "more lines lay out over more pages: {long:?} vs {report:?}"
        );
    }

    #[test]
    fn unknown_template_and_a_wrong_kind_and_a_newer_schema_are_rejected() {
        let document = DocumentModel::Cv(cv(vec![("Experience", long_lines(1))]));

        assert_eq!(
            layout_facts(&document, "nope"),
            Err(VerifyError::UnknownTemplate("nope".into()))
        );
        assert_eq!(
            layout_facts(&document, "cover-classic"),
            Err(VerifyError::WrongTemplate {
                template: "cover-classic".into(),
                kind: DocumentKind::Cv
            })
        );

        let mut newer = cv(vec![("Experience", long_lines(1))]);
        newer.schema_version = SCHEMA_VERSION + 1;
        assert_eq!(
            layout_facts(&DocumentModel::Cv(newer), "cv-classic"),
            Err(VerifyError::UnsupportedSchema {
                version: SCHEMA_VERSION + 1,
                expected: SCHEMA_VERSION
            })
        );
    }

    // AC2: the letter is exactly one page, with nothing stranded.
    #[test]
    fn a_cover_letter_is_exactly_one_page() {
        let fits = cover(vec![
            "I am writing to apply for the Backend Engineer role at Acme, where I would bring six years of experience building billing systems in Rust and Postgres.".into(),
            "In my last role I shipped the billing pipeline that cut p99 latency by 40% across three regions, and I led the migration off the legacy queue without downtime.".into(),
            "I would welcome the chance to talk about how I can help Acme keep that record.".into(),
        ]);
        let report = verify_cover(&fits, &TemplateId::new("cover-classic"), &TypstRenderer)
            .expect("measure the letter");
        assert_eq!(
            report.pages, 1,
            "a cover letter is exactly one page (FR-76)"
        );
        assert_eq!(report.orphans, 0);
        assert!(cover_fits(&report));
        assert_eq!(measure(&DocumentModel::CoverLetter(fits)).pages, 1);

        let overflows = cover(
            (0..30)
                .map(|i| {
                    format!("Paragraph {i}: I shipped the billing pipeline in Rust and Postgres, cutting p99 latency by 40% across three regions, and I led the migration off the legacy queue without downtime.")
                })
                .collect(),
        );
        let report = measure(&DocumentModel::CoverLetter(overflows));
        assert!(
            report.pages > 1,
            "an over-long letter does not fit one page"
        );
        assert!(!cover_fits(&report), "and it does not pass the rule");
    }

    // AC1: a CV over budget is cut down line by line until it fits, and the studio
    // gets to show what went.
    #[test]
    fn fitting_cuts_the_least_relevant_lines_until_the_cv_fits() {
        let mut items = two_line_lines(24);
        items.extend([
            "Led the billing migration for three regions without downtime.".into(),
            "Designed the Postgres schema behind the billing pipeline.".into(),
            "Mentored two engineers through their first production launch.".into(),
            "Cut infrastructure cost by 30% by retiring the legacy queue.".into(),
        ]);
        let document = DocumentModel::Cv(cv(vec![("Experience", items)]));

        let unfitted = measure(&document);
        assert!(
            unfitted.pages > 1,
            "the fixture is over budget: {unfitted:?}"
        );
        assert_eq!(unfitted.orphans, 0);

        // The four lines that matter are relevant; the filler answers nothing.
        let relevance = |claim: &Claim| {
            if claim
                .text
                .starts_with("Shipped the billing pipeline number")
            {
                0.1
            } else {
                1.0
            }
        };
        let fitted = fit_cv(
            &document_to_cv(&document),
            &cover(vec![]),
            &TemplateId::new("cv-classic"),
            PageBudget::new(1).expect("1 page is a budget"),
            &relevance,
            &TypstRenderer,
        )
        .expect("the CV fits in one page once the low-relevance lines are gone");

        assert!(!fitted.cut.is_empty(), "something had to go");
        assert!(fitted.report.pages <= 1, "it fits: {:?}", fitted.report);
        assert!(
            fitted
                .cut
                .iter()
                .all(|cut| cut.text.starts_with("Shipped the billing pipeline number")),
            "only the lines that answer nothing went: {:?}",
            fitted.cut
        );
        assert!(
            fitted
                .cut
                .windows(2)
                .all(|pair| pair[0].score <= pair[1].score),
            "the lowest score goes first: {:?}",
            fitted.cut
        );
        assert!(
            fitted
                .cut
                .iter()
                .all(|cut| cut.section.as_deref() == Some("Experience")),
            "every cut line says where it came from: {:?}",
            fitted.cut
        );
        let kept: Vec<&str> = fitted.cv.sections[0]
            .items
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        for relevant in [
            "Led the billing migration for three regions without downtime.",
            "Designed the Postgres schema behind the billing pipeline.",
            "Mentored two engineers through their first production launch.",
            "Cut infrastructure cost by 30% by retiring the legacy queue.",
        ] {
            assert!(
                kept.contains(&relevant),
                "a line that matters was cut: {kept:?}"
            );
        }
    }

    fn document_to_cv(document: &DocumentModel) -> CvDocument {
        match document {
            DocumentModel::Cv(cv) => cv.clone(),
            DocumentModel::CoverLetter(_) => unreachable!("the fixture is a CV"),
        }
    }
}
