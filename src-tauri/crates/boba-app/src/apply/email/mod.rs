//! Email Apply composition (S9.3, FR-82/FR-88): the recipient, subject and
//! body the user reviews before one click sends the application.
//!
//! Everything here is pure: a draft is built from the Offer and the verified
//! Package, and an [`OutgoingMessage`] is assembled from it. The Offer's text
//! and its apply address are **untrusted** (a posting can say anything), so the
//! subject is forced onto one line and the address must be a plain mailbox:
//! a posting never gets to inject a header or a second recipient.

use boba_domain::posting::Offer;

use crate::ports::mail::{Attachment, OutgoingMessage};

/// The longest subject the composer keeps, so a hostile posting cannot blow up
/// the message.
const SUBJECT_MAX: usize = 200;

/// One reviewed application email, before it is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The posting's apply address, already checked to be one mailbox.
    pub recipient: String,
    pub subject: String,
    pub body_text: String,
}

/// The draft a posting starts from, `None` when it carries no usable apply
/// address (there is no Email Apply channel then).
///
/// The body follows the posting's language: `es*` gets Spanish, anything else
/// English, which is what the product ships from day 1.
pub fn draft_for(offer: &Offer) -> Option<Draft> {
    let recipient = offer.apply.email.as_deref()?.trim();
    if !is_plain_address(recipient) {
        return None;
    }
    let title = one_line(&offer.title);
    let company = one_line(&offer.company);
    let subject = one_line(&format!("Application: {title}"));
    let body_text = if offer.lang.to_lowercase().starts_with("es") {
        body_es(&title, &company)
    } else {
        body_en(&title, &company)
    };
    Some(Draft {
        recipient: recipient.to_string(),
        subject,
        body_text,
    })
}

/// Whether `address` is exactly one mailbox and nothing else.
///
/// Deliberately narrow: no display name, no list, no whitespace or control
/// character — every one of those would be an attempt to reach a header or a
/// second recipient through the posting.
pub fn is_plain_address(address: &str) -> bool {
    let address = address.trim();
    if address.is_empty() || address.len() > 254 {
        return false;
    }
    if address
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ',' | ';' | '<' | '>' | '"'))
    {
        return false;
    }
    let mut parts = address.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
}

/// Force `text` onto one line and cap it: a subject is a header, never a
/// place for a posting's newline.
pub fn one_line(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SUBJECT_MAX {
        flat.chars().take(SUBJECT_MAX).collect::<String>()
    } else {
        flat
    }
}

fn body_en(title: &str, company: &str) -> String {
    let greeting = if company.is_empty() {
        "Dear hiring team,".to_string()
    } else {
        format!("Dear {company} team,")
    };
    format!(
        "{greeting}\n\nI would like to apply for the {title} role.\n\n\
         Please find my application documents attached.\n\nBest regards,"
    )
}

fn body_es(title: &str, company: &str) -> String {
    let greeting = if company.is_empty() {
        "Hola, equipo de contratación:".to_string()
    } else {
        format!("Hola, equipo de {company}:")
    };
    format!(
        "{greeting}\n\nMe gustaría postularme al puesto de {title}.\n\n\
         Adjunto mis documentos de postulación.\n\nUn saludo,"
    )
}

/// The exact bytes hashed into the audit log's `payload_blake3` (AD-12).
///
/// Field by field in a fixed order, with each attachment's bytes, so two
/// messages with the same visible content hash the same and a changed recipient,
/// subject, body or attachment changes the hash.
pub fn payload_bytes(message: &OutgoingMessage) -> Vec<u8> {
    let mut out = Vec::new();
    for to in &message.to {
        out.extend_from_slice(b"to:");
        out.extend_from_slice(to.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"subject:");
    out.extend_from_slice(message.subject.as_bytes());
    out.extend_from_slice(b"\nbody:");
    out.extend_from_slice(message.body_text.as_bytes());
    out.push(b'\n');
    for attachment in &message.attachments {
        out.extend_from_slice(b"attachment:");
        out.extend_from_slice(attachment.filename.as_bytes());
        out.push(b':');
        out.extend_from_slice(attachment.mime.as_bytes());
        out.push(b':');
        out.extend_from_slice(&attachment.bytes);
        out.push(b'\n');
    }
    out
}

/// One reviewed attachment, as the view shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct AttachmentView {
    pub file_name: String,
    /// `cv` or `cover_letter`.
    pub kind: String,
    pub version: u32,
    /// Size in bytes, so the review can show what is being sent.
    pub bytes: u32,
}

impl AttachmentView {
    /// The attachment as the view lists it (AD-14: only a verified PDF of a
    /// Ready document ever reaches here).
    pub fn new(file_name: &str, kind: &str, version: u32, size: usize) -> Self {
        Self {
            file_name: file_name.to_string(),
            kind: kind.to_string(),
            version,
            bytes: u32::try_from(size).unwrap_or(u32::MAX),
        }
    }
}

/// One outgoing PDF attachment from the kit.
pub fn pdf_attachment(file_name: &str, bytes: Vec<u8>) -> Attachment {
    Attachment {
        filename: file_name.to_string(),
        mime: "application/pdf".to_string(),
        bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::posting::{ApplyHints, WorkMode};

    fn offer(lang: &str, email: Option<&str>) -> Offer {
        Offer {
            id: "pst_01".into(),
            source_id: "src_01".into(),
            external_id: "ext-1".into(),
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            locations: vec![],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: "2026-10-17T00:00:00Z".into(),
            last_seen: "2026-10-17T00:00:00Z".into(),
            lang: lang.into(),
            description_html: "<p>Hiring.</p>".into(),
            description_text: "Hiring.".into(),
            requirements: vec![],
            apply: ApplyHints {
                url: "https://jobs.example/apply".into(),
                email: email.map(str::to_string),
            },
        }
    }

    #[test]
    fn a_posting_with_an_apply_address_drafts_an_english_email() {
        let draft = draft_for(&offer("en", Some("jobs@nimbus.example"))).unwrap();
        assert_eq!(draft.recipient, "jobs@nimbus.example");
        assert_eq!(draft.subject, "Application: Backend Engineer");
        assert!(draft.body_text.starts_with("Dear Nimbus team,"));
        assert!(draft.body_text.contains("Backend Engineer"));
    }

    #[test]
    fn a_spanish_posting_gets_a_spanish_body() {
        let draft = draft_for(&offer("es", Some("jobs@nimbus.example"))).unwrap();
        assert!(draft.body_text.starts_with("Hola, equipo de Nimbus:"));
        assert!(draft.body_text.contains("Backend Engineer"));
    }

    #[test]
    fn a_posting_without_an_address_has_no_email_channel() {
        assert_eq!(draft_for(&offer("en", None)), None);
        assert_eq!(draft_for(&offer("en", Some("  "))), None);
    }

    #[test]
    fn a_posting_cannot_smuggle_a_second_recipient_or_a_header() {
        // A list, a display name and a header line are no channel at all.
        for hostile in [
            "jobs@nimbus.example, evil@attacker.example",
            "Nimbus <jobs@nimbus.example>",
            "jobs@nimbus.example\nBcc: evil@attacker.example",
            "jobs@nimbus.example;",
            "not-an-address",
            "@nimbus.example",
            "jobs@localhost",
        ] {
            assert!(
                !is_plain_address(hostile),
                "must refuse the posting address {hostile:?}"
            );
            assert_eq!(draft_for(&offer("en", Some(hostile))), None);
        }
    }

    #[test]
    fn a_hostile_title_stays_on_one_line() {
        let mut posting = offer("en", Some("jobs@nimbus.example"));
        posting.title = "Backend\nBcc: evil@attacker.example".into();
        let draft = draft_for(&posting).unwrap();
        assert!(!draft.subject.contains('\n'), "a subject is one header");
        assert!(
            !draft
                .body_text
                .lines()
                .any(|line| line.trim_start().starts_with("Bcc:")),
            "the posting cannot open a header-looking line in the body either"
        );
    }

    #[test]
    fn the_payload_hash_covers_every_part_of_the_message() {
        let base = OutgoingMessage {
            to: vec!["jobs@nimbus.example".into()],
            subject: "Application: Backend Engineer".into(),
            body_text: "Hello".into(),
            attachments: vec![pdf_attachment("cv-v1.pdf", b"%PDF-1.7".to_vec())],
        };
        let digest = payload_bytes(&base);
        assert_eq!(digest, payload_bytes(&base.clone()), "stable");

        let mut other = base.clone();
        other.body_text = "Hello!".into();
        assert_ne!(payload_bytes(&other), digest, "the body is covered");

        let mut other = base.clone();
        other.attachments[0].bytes = b"%PDF-1.8".to_vec();
        assert_ne!(payload_bytes(&other), digest, "the attachments are covered");

        let mut other = base;
        other.to = vec!["hire@nimbus.example".into()];
        assert_ne!(payload_bytes(&other), digest, "the recipient is covered");
    }
}
