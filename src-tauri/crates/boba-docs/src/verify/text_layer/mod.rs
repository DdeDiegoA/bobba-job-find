//! ATS text-layer verification (S7.6, FR-77, SM-8, AD-14).
//!
//! An ATS reads the PDF, not the draft: if a detail is drawn as a glyph run with
//! no usable text, or the paragraphs come out in the wrong order, the parser
//! misses it and a human never sees the application. So this pass extracts the
//! text layer of the PDF **we just rendered** (ADR-006: `pdf-extract`, never a
//! user-supplied file) and checks four things:
//!
//! * every contact detail the template prints is there as literal text;
//! * the blocks come back in the document's own reading order;
//! * no `(cid:` and no U+FFFD, the two ways a text layer comes out unreadable;
//! * every requirement keyword a Profile fact backs is there (FR-75: the caller
//!   passes `boba_domain::keywords::backed_keywords`, which never holds a gap).
//!
//! A layer that cannot be read at all is an **error**, never a pass: SM-8 counts
//! sent PDFs that pass ATS verification, so failing open would make the number a
//! lie (AD-14).
//!
//! Matching reuses `boba_domain::keywords::{fold, words}`, the same folding the
//! coverage pass used, so "backed" and "present in the layer" cannot drift apart.

use boba_app::ports::verify::VerifyError;

use boba_domain::documents::{Contact, DocumentKind, DocumentModel, Link, TextLayerReport};
use boba_domain::keywords::{fold, words};

/// The `(cid:` marker a font without a usable encoding leaves behind (FR-77).
const CID: &str = "(cid:";

/// The replacement character a decoder leaves where it could not read a glyph.
const REPLACEMENT: &str = "U+FFFD";

/// What an ATS reads back out of `pdf`, and what it failed to find (FR-77).
///
/// `required_keywords` are the requirement keywords a Profile fact backs
/// (FR-75): every one of them must be readable, because the coverage panel told
/// the user it was.
pub fn text_layer_facts(
    pdf: &[u8],
    document: &DocumentModel,
    required_keywords: &[String],
) -> Result<TextLayerReport, VerifyError> {
    let layer = pdf_extract::extract_text_from_mem(pdf)
        .map_err(|error| VerifyError::Measure(format!("text layer: {error}")))?;

    Ok(TextLayerReport {
        missing_contact: missing_contact(&layer, document),
        missing_keywords: missing_keywords(&layer, required_keywords),
        out_of_order: reading_order(&layer, &blocks(document)),
        undecodable: undecodable(&layer),
        text_layer: layer,
    })
}

/// The document's blocks, in the order the templates print them (FR-77).
///
/// Only the text that must be there is listed: the headings come from the draft
/// itself, and the template's own words ("Summary") are not the draft's to check.
pub fn blocks(document: &DocumentModel) -> Vec<String> {
    let mut blocks = Vec::new();
    match document {
        DocumentModel::Cv(cv) => {
            contact_blocks(&cv.contact, DocumentKind::Cv, &mut blocks);
            if let Some(summary) = &cv.summary {
                blocks.push(summary.text.clone());
            }
            for section in &cv.sections {
                blocks.push(section.heading.clone());
                blocks.extend(section.items.iter().map(|item| item.text.clone()));
            }
        }
        DocumentModel::CoverLetter(cover) => {
            contact_blocks(&cover.contact, DocumentKind::CoverLetter, &mut blocks);
            for text in [
                cover.date.as_deref(),
                cover.company.as_deref(),
                cover.recipient.as_deref(),
                cover.salutation.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                blocks.push(text.to_string());
            }
            blocks.extend(cover.paragraphs.iter().map(|p| p.text.clone()));
            if let Some(closing) = cover.closing.as_deref() {
                blocks.push(closing.to_string());
            }
        }
    }
    blocks
}

/// Every block the layer does not show in the document's own order (FR-77).
///
/// Each block the layer shows has to come after the block the document puts
/// before it; the one that shows up before its time is the one reported.
pub fn reading_order(layer: &str, blocks: &[String]) -> Vec<String> {
    let layer = fold(&normalize(layer));
    let mut previous: Option<usize> = None;
    let mut violations = Vec::new();
    for block in blocks {
        let needle = fold(&normalize(block));
        if needle.is_empty() {
            continue;
        }
        // A block the layer does not show is not this check's business: the
        // contact check owns what must be there and the keyword check owns the
        // requirement words. Only the order of what *is* there is read here.
        let Some(position) = layer.find(&needle) else {
            continue;
        };
        if previous.is_some_and(|previous| position < previous) {
            violations.push(block.clone());
            continue;
        }
        previous = Some(position);
    }
    violations
}

/// The contact details the layer does not show as literal text (FR-77).
///
/// Only what the template prints for this kind is required: neither cover
/// template prints a headline or a link, so demanding them there would fail a
/// correct letter. The values are `name`, `headline`, `email`, `phone`,
/// `location` and `links` — a small vocabulary the studio translates.
pub fn missing_contact(layer: &str, document: &DocumentModel) -> Vec<String> {
    let (contact, kind) = match document {
        DocumentModel::Cv(cv) => (&cv.contact, DocumentKind::Cv),
        DocumentModel::CoverLetter(cover) => (&cover.contact, DocumentKind::CoverLetter),
    };
    let layer = fold(&normalize(layer));

    let mut missing = Vec::new();
    let mut field = |field: &str, value: Option<&str>| {
        if let Some(value) = value.filter(|value| !value.trim().is_empty())
            && !layer.contains(&fold(&normalize(value)))
        {
            missing.push(field.to_string());
        }
    };
    field("name", Some(contact.full_name.as_str()));
    if kind == DocumentKind::Cv {
        field("headline", contact.headline.as_deref());
    }
    field("email", contact.email.as_deref());
    field("phone", contact.phone.as_deref());
    field("location", contact.location.as_deref());
    if kind == DocumentKind::Cv && !contact.links.is_empty() {
        // The templates print `label: url`, so the URL is what must be readable.
        let links_missing = contact
            .links
            .iter()
            .any(|Link { url, .. }| !layer.contains(&fold(&normalize(url))));
        if links_missing {
            missing.push("links".to_string());
        }
    }
    missing
}

/// The requirement keywords the layer does not show (FR-77, AC1).
///
/// `required` holds the keywords a Profile fact backs, each of which the coverage
/// panel has already told the user is covered.
pub fn missing_keywords(layer: &str, required: &[String]) -> Vec<String> {
    let present = words(layer);
    let mut missing: Vec<String> = Vec::new();
    for keyword in required {
        let keyword = fold(keyword);
        let keyword = keyword.trim_matches('.').to_string();
        if keyword.is_empty() {
            continue;
        }
        if !present.contains(&keyword) && !missing.contains(&keyword) {
            missing.push(keyword);
        }
    }
    missing
}

/// The undecodable markers a parser would hit: `(cid:` and U+FFFD (FR-77).
pub fn undecodable(layer: &str) -> Vec<String> {
    let mut found = Vec::new();
    if layer.contains(CID) {
        found.push(CID.to_string());
    }
    if layer.contains('\u{FFFD}') {
        found.push(REPLACEMENT.to_string());
    }
    found
}

/// The contact values a template prints, in the order it prints them.
fn contact_blocks(contact: &Contact, kind: DocumentKind, blocks: &mut Vec<String>) {
    let mut push = |value: Option<&str>| {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            blocks.push(value.to_string());
        }
    };
    push(Some(contact.full_name.as_str()));
    if kind == DocumentKind::Cv {
        push(contact.headline.as_deref());
    }
    push(contact.email.as_deref());
    push(contact.phone.as_deref());
    push(contact.location.as_deref());
    if kind == DocumentKind::Cv {
        for Link { label, url } in &contact.links {
            blocks.push(format!("{label}: {url}"));
        }
    }
}

/// The layer as an ATS reads it: repeated whitespace is one space, so a wrapped
/// line does not split a detail in two.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use boba_app::ports::render::RenderPort;
    use boba_app::ports::verify::VerifyPort;
    use boba_domain::documents::{
        Claim, CoverLetterDocument, CvDocument, Lang, Link, RenderedPdf, SCHEMA_VERSION, Section,
        TemplateId,
    };

    use crate::TypstRenderer;

    use super::*;

    fn contact() -> Contact {
        Contact {
            full_name: "Ada Lovelace".into(),
            headline: Some("Backend developer".into()),
            email: Some("ada@example.com".into()),
            phone: Some("+57 300 000 0000".into()),
            location: Some("Bogotá, Colombia".into()),
            links: vec![Link {
                label: "GitHub".into(),
                url: "github.com/ada".into(),
            }],
        }
    }

    fn cv() -> CvDocument {
        CvDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::En,
            contact: contact(),
            summary: Some(Claim {
                text: "Backend developer with six years of experience.".into(),
                fact_ids: vec!["fact_1".into()],
            }),
            sections: vec![Section {
                heading: "Experience".into(),
                items: vec![Claim {
                    text: "Shipped the billing pipeline in Rust and PostgreSQL.".into(),
                    fact_ids: vec!["fact_2".into()],
                }],
            }],
            declared_gaps: vec!["Kubernetes".into()],
        }
    }

    fn cover() -> CoverLetterDocument {
        CoverLetterDocument {
            schema_version: SCHEMA_VERSION,
            lang: Lang::Es,
            contact: Contact {
                headline: None,
                links: vec![],
                ..contact()
            },
            company: Some("Acme".into()),
            recipient: Some("Equipo de selección".into()),
            date: Some("16 de octubre de 2026".into()),
            salutation: Some("Hola:".into()),
            paragraphs: vec![Claim {
                text: "Me postulo al puesto de Backend.".into(),
                fact_ids: vec!["fact_3".into()],
            }],
            closing: Some("Atentamente, Ada Lovelace".into()),
            declared_gaps: vec![],
        }
    }

    fn rendered(document: &DocumentModel) -> RenderedPdf {
        let template = match document {
            DocumentModel::Cv(_) => "cv-classic",
            DocumentModel::CoverLetter(_) => "cover-classic",
        };
        TypstRenderer
            .render_pdf(document, &TemplateId::new(template), document.lang())
            .expect("the default templates always render")
    }

    /// The ATS check, run through the port on a real rendered PDF.
    fn check(document: &DocumentModel, required: &[&str]) -> TextLayerReport {
        let required: Vec<String> = required.iter().map(|k| k.to_string()).collect();
        TypstRenderer
            .text_layer(document, &rendered(document), &required)
            .expect("the text layer of our own render is readable")
    }

    // AC1: a rendered CV passes, and the layer really holds the contact details.
    #[test]
    fn a_rendered_cv_passes_every_ats_check() {
        let report = check(&DocumentModel::Cv(cv()), &["rust", "postgresql"]);

        assert!(report.passed(), "{report:?}");
        assert!(
            report.text_layer.contains("Ada Lovelace")
                && report.text_layer.contains("ada@example.com")
                && report.text_layer.contains("Bogotá"),
            "the layer is the PDF's own text: {}",
            report.text_layer
        );
        assert!(report.missing_contact.is_empty());
        assert!(report.missing_keywords.is_empty());
        assert!(report.out_of_order.is_empty());
        assert!(report.undecodable.is_empty());
    }

    // AC1: the letter too — and a cover template prints no headline and no links,
    // so their absence from the layer is not a miss.
    #[test]
    fn a_rendered_cover_letter_passes_every_ats_check() {
        let report = check(&DocumentModel::CoverLetter(cover()), &["backend"]);

        assert!(report.passed(), "{report:?}");
        assert!(report.text_layer.contains("ada@example.com"));
        assert!(
            report
                .text_layer
                .contains("Me postulo al puesto de Backend.")
        );
    }

    // AC1: the check runs through `VerifyPort`, so the core asks the adapter.
    #[test]
    fn the_check_is_the_ports_text_layer_method() {
        let document = DocumentModel::Cv(cv());
        let report = TypstRenderer
            .text_layer(&document, &rendered(&document), &[])
            .expect("read the layer");

        assert!(report.passed());
        assert_eq!(report, check(&document, &[]));
    }

    // FR-77: a detail the layer does not show is named.
    #[test]
    fn a_contact_detail_the_layer_does_not_show_is_reported() {
        let document = DocumentModel::Cv(cv());
        let layer = "Ada Lovelace Backend developer";

        assert_eq!(
            missing_contact(layer, &document),
            ["email", "phone", "location", "links"]
        );
        assert_eq!(
            missing_contact("", &document),
            ["name", "headline", "email", "phone", "location", "links"]
        );
        assert!(missing_contact(&rendered_layer(&document), &document).is_empty());
    }

    // FR-77: a cover letter is not asked for what its template never prints.
    #[test]
    fn a_cover_letter_is_not_asked_for_a_headline_or_links() {
        let document = DocumentModel::CoverLetter(cover());

        let layer = "Ada Lovelace ada@example.com +57 300 000 0000 Bogotá, Colombia";
        assert!(
            missing_contact(layer, &document).is_empty(),
            "neither cover template prints a headline or a link, so their absence is not a miss"
        );
        assert_eq!(
            missing_contact("", &document),
            ["name", "email", "phone", "location"]
        );
    }

    // FR-77, AC1: a keyword the Profile backs must be readable — case and
    // diacritics never decide it.
    #[test]
    fn a_profile_backed_keyword_must_be_readable_in_the_layer() {
        let document = DocumentModel::Cv(cv());

        let report = check(&document, &["kubernetes"]);
        assert_eq!(report.missing_keywords, ["kubernetes"]);
        assert!(!report.passed(), "a missing keyword fails the ATS check");

        assert!(
            missing_keywords("Ingeniería de datos con PostgreSQL", &["ingenieria".into()])
                .is_empty(),
            "case and accents never decide it"
        );
        assert_eq!(
            missing_keywords("ada@example.com", &["rust".into()]),
            ["rust"]
        );
        assert_eq!(
            missing_keywords(
                "Rust and Postgres",
                &["rust".into(), "rust".into(), "go".into()]
            ),
            ["go"],
            "a repeated keyword is asked for once"
        );
        assert!(
            missing_keywords(
                &rendered_layer(&document),
                &["rust".into(), "postgresql".into()]
            )
            .is_empty()
        );
    }

    // FR-77: the blocks come back in the order the document prints them.
    #[test]
    fn a_scrambled_layer_is_reported_out_of_order() {
        let document = DocumentModel::Cv(cv());
        let blocks = blocks(&document);

        // The real render: heading first, then its line, then the next section.
        assert!(
            reading_order(&rendered_layer(&document), &blocks).is_empty(),
            "{:?}",
            rendered_layer(&document)
        );
        assert_eq!(
            blocks,
            [
                "Ada Lovelace",
                "Backend developer",
                "ada@example.com",
                "+57 300 000 0000",
                "Bogotá, Colombia",
                "GitHub: github.com/ada",
                "Backend developer with six years of experience.",
                "Experience",
                "Shipped the billing pipeline in Rust and PostgreSQL.",
            ]
        );

        // A layer that came back with the line before its heading.
        let order = vec![
            "Experience".to_string(),
            "Shipped the pipeline in Rust".to_string(),
        ];
        assert!(reading_order("Experience\nShipped the pipeline in Rust", &order).is_empty());
        assert_eq!(
            reading_order("Shipped the pipeline in Rust\nExperience", &order),
            ["Shipped the pipeline in Rust"],
            "the block the layer shows before its time"
        );
    }

    // FR-77, SM-8: the two ways a layer comes out unreadable are named.
    #[test]
    fn undecodable_markers_are_found() {
        assert!(undecodable("Ada Lovelace ada@example.com").is_empty());
        assert_eq!(undecodable("Ada (cid:12) Lovelace"), [CID]);
        assert_eq!(undecodable("Ada \u{FFFD} Lovelace"), [REPLACEMENT]);
        assert_eq!(undecodable("(cid:12) and \u{FFFD}"), [CID, REPLACEMENT]);
    }

    // SM-8: a PDF that cannot be read fails closed — never a silent pass.
    #[test]
    fn a_pdf_that_cannot_be_read_is_an_error_not_a_pass() {
        let error = text_layer_facts(b"not a pdf at all", &DocumentModel::Cv(cv()), &[])
            .expect_err("a broken PDF has no text layer");
        let VerifyError::Measure(message) = error else {
            panic!("a broken PDF must fail closed as a measurement error: {error:?}");
        };

        assert!(message.starts_with("text layer:"), "{message}");
    }

    // AC3 (AD-14): with the real renderer, the real layout pass and the real
    // validator, a document is `verified` only when all three checks pass.
    #[test]
    fn a_document_is_verified_only_when_all_three_checks_pass() {
        use boba_app::ports::verify::verify_document;
        use boba_domain::documents::DocumentKind;
        use boba_domain::validator::validate;

        let document = DocumentModel::Cv(cv());
        let template = TemplateId::new("cv-classic");
        let rendered = rendered(&document);
        let known = |id: &str| matches!(id, "fact_1" | "fact_2");

        let fabrication = validate(&document, &known).is_ready();
        assert!(
            fabrication,
            "the fixture's claims name facts the Profile has"
        );

        let verified = verify_document(
            &TypstRenderer,
            &document,
            &template,
            &rendered,
            &["rust".into(), "postgresql".into()],
            fabrication,
        )
        .expect("all three checks answer");
        assert!(verified.is_verified(DocumentKind::Cv, 2), "{verified:?}");

        // A keyword the Profile does not back is a gap, and a gap is never
        // required of a document — so requiring one is what fails the ATS check.
        let required_gap = verify_document(
            &TypstRenderer,
            &document,
            &template,
            &rendered,
            &["kubernetes".into()],
            fabrication,
        )
        .expect("the checks still answer");
        assert!(!required_gap.is_verified(DocumentKind::Cv, 2), "ATS failed");

        // A claim nothing in the Profile backs.
        let fabricated = verify_document(
            &TypstRenderer,
            &document,
            &template,
            &rendered,
            &[],
            validate(&document, &|_| false).is_ready(),
        )
        .expect("the checks still answer");
        assert!(
            !fabricated.is_verified(DocumentKind::Cv, 2),
            "the fabrication check failed"
        );
    }

    fn rendered_layer(document: &DocumentModel) -> String {
        check(document, &[]).text_layer
    }
}
