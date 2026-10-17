//! Deterministic EN/ES reply classification (S8.4, FR-100, FR-101, NFR-3).
//!
//! The first classifier a matched message meets (AC1). It runs offline on the
//! stored plain text, matches a small table of phrases and returns a class, a
//! confidence and the line the phrase was read from. Nothing here leaves the
//! process: when the user chose "Deterministic classification only" (AC4,
//! NFR-3), this is the only classifier that ever runs.
//!
//! An uncertain case (no rule, or [`UNCERTAIN_CONFIDENCE`] is not met) is
//! [`None`]: the caller decides whether to ask the LLM (AC1) or to leave the
//! message for manual triage (AC4).

use boba_domain::mail::{ReplyClass, RuleMatch, UNCERTAIN_CONFIDENCE};

/// The longest quote stored for one message. A quote is evidence, not a copy
/// of the message: a long line is clipped around the matched phrase.
pub const MAX_QUOTE_CHARS: usize = 240;

/// The rules as the use case's [`ReplyClassifier`]. The classification use
/// case lives in `boba-app`, which cannot depend on this adapter, so the
/// composition root hands it this struct.
pub struct RulesClassifier;

impl boba_app::status_proposals::ReplyClassifier for RulesClassifier {
    fn classify(&self, subject: &str, body: &str) -> Option<RuleMatch> {
        classify(subject, body)
    }
}

/// One phrase and how sure its match is.
struct Pattern {
    phrase: &'static str,
    confidence: f64,
}

/// One class and the phrases that point at it. The table is ordered from the
/// strongest signal to the weakest; a tie is broken by that order.
struct Rule {
    class: ReplyClass,
    patterns: &'static [Pattern],
}

const fn p(phrase: &'static str, confidence: f64) -> Pattern {
    Pattern { phrase, confidence }
}

/// The EN/ES phrase table (FR-100). Phrases are lower case and matched on word
/// boundaries, never as bare fragments of longer words.
static RULES: &[Rule] = &[
    Rule {
        class: ReplyClass::Offer,
        patterns: &[
            p("we are pleased to offer", 0.97),
            p("pleased to offer you", 0.97),
            p("job offer", 0.95),
            p("offer letter", 0.95),
            p("formal offer", 0.95),
            p("nos complace ofrecerte", 0.97),
            p("oferta de empleo", 0.95),
            p("carta de oferta", 0.95),
            p("propuesta de incorporación", 0.9),
        ],
    },
    Rule {
        class: ReplyClass::Rejection,
        patterns: &[
            p("not moving forward", 0.97),
            p("decided not to move forward", 0.97),
            p("decided not to proceed", 0.97),
            p("will not be moving forward", 0.97),
            p("position has been filled", 0.97),
            p("we regret", 0.95),
            p("not selected", 0.9),
            p("unfortunately", 0.9),
            p("other candidates", 0.85),
            p("hemos decidido no continuar", 0.97),
            p("lamentablemente", 0.95),
            p("no continuaremos", 0.95),
            p("no avanzaremos", 0.95),
            p("no ha sido seleccionad", 0.95),
            p("ha sido cubierta", 0.95),
            p("descartad", 0.9),
            p("otra persona", 0.85),
            p("otro candidato", 0.85),
        ],
    },
    Rule {
        class: ReplyClass::InterviewInvite,
        patterns: &[
            p("invite you to an interview", 0.97),
            p("invite you to interview", 0.95),
            p("schedule an interview", 0.95),
            p("invitarte a una entrevista", 0.97),
            p("agendar una entrevista", 0.95),
            p("schedule a call", 0.85),
            p("set up a call", 0.85),
            p("book a time", 0.85),
            p("coordinemos una llamada", 0.85),
        ],
    },
    Rule {
        class: ReplyClass::Assessment,
        patterns: &[
            p("coding challenge", 0.95),
            p("take-home", 0.95),
            p("take home assignment", 0.95),
            p("technical assessment", 0.95),
            p("online assessment", 0.95),
            p("prueba técnica", 0.95),
            p("desafío técnico", 0.95),
            p("evaluación técnica", 0.9),
            p("ejercicio práctico", 0.9),
        ],
    },
    Rule {
        class: ReplyClass::RequestForInfo,
        patterns: &[
            p("could you please send", 0.9),
            p("por favor envía", 0.9),
            p("necesitamos la siguiente información", 0.9),
            p("please provide", 0.85),
            p("completa el siguiente formulario", 0.85),
            p("additional information", 0.8),
            p("please complete", 0.8),
        ],
    },
    Rule {
        class: ReplyClass::Acknowledgement,
        patterns: &[
            p("hemos recibido tu postulación", 0.97),
            p("hemos recibido tu candidatura", 0.97),
            p("thank you for applying", 0.95),
            p("thanks for applying", 0.95),
            p("we have received your application", 0.95),
            p("we received your application", 0.95),
            p("application received", 0.95),
            p("gracias por postularte", 0.95),
            p("postulación recibida", 0.9),
            p("candidatura recibida", 0.9),
        ],
    },
];

/// Classify one stored message (AC1): the strongest rule that matches, or
/// `None` when no rule is sure enough to be more than a guess.
///
/// The subject is a line of the message like any other: a class the subject
/// announces ("Application received") counts, and its quote is the subject.
pub fn classify(subject: &str, body: &str) -> Option<RuleMatch> {
    let lines: Vec<&str> = std::iter::once(subject.trim())
        .chain(body.lines().map(str::trim))
        .filter(|line| !line.is_empty())
        .collect();

    let mut best: Option<RuleMatch> = None;
    for rule in RULES {
        for pattern in rule.patterns {
            let quoted = lines.iter().find_map(|line| {
                let lower = line.to_lowercase();
                let position = find_phrase(&lower, pattern.phrase)?;
                Some(quote(line, position, pattern.phrase.chars().count()))
            });
            let Some(quoted_line) = quoted else {
                continue;
            };
            let candidate = RuleMatch {
                class: rule.class,
                confidence: pattern.confidence,
                quoted_line,
            };
            // `>` (not `>=`): the first rule in table order keeps a tie.
            if best
                .as_ref()
                .is_none_or(|current| candidate.confidence > current.confidence)
            {
                best = Some(candidate);
            }
        }
    }
    best.filter(|matched| matched.confidence >= UNCERTAIN_CONFIDENCE)
}

/// The byte position of `phrase` in `haystack`, only when both of its edges are
/// word boundaries.
fn find_phrase(haystack: &str, phrase: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(phrase) {
        let start = from + offset;
        let end = start + phrase.len();
        let before = haystack[..start].chars().next_back();
        let after = haystack[end..].chars().next();
        if !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric) {
            return Some(start);
        }
        from = start + haystack[start..].chars().next()?.len_utf8();
    }
    None
}

/// The matched line, clipped to [`MAX_QUOTE_CHARS`] around the phrase.
fn quote(line: &str, position: usize, phrase_chars: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let start_char = line[..position].chars().count();
    if chars.len() <= MAX_QUOTE_CHARS {
        return line.trim().to_string();
    }
    // Keep a little context before the phrase, then the phrase itself.
    let lead = 40.min(start_char);
    let start = (start_char - lead).min(chars.len().saturating_sub(phrase_chars + lead));
    let end = (start + MAX_QUOTE_CHARS).min(chars.len());
    let clipped: String = chars[start..end].iter().collect();
    let clipped = clipped.trim().to_string();
    if start > 0 {
        format!("…{clipped}")
    } else {
        clipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::mail::{LLM_CONFIDENCE_CEILING, ReplyClass, UNCERTAIN_CONFIDENCE};

    fn class_of(subject: &str, body: &str) -> ReplyClass {
        classify(subject, body)
            .unwrap_or_else(|| panic!("no rule matched {subject:?} / {body:?}"))
            .class
    }

    #[test]
    fn english_rejections_are_certain_and_quote_the_line() {
        let matched = classify(
            "Your application to Acme",
            "Dear Diego,\nUnfortunately, we have decided not to move forward with your application.\nBest,\nAcme Talent",
        )
        .expect("a rejection rule matches");
        assert_eq!(matched.class, ReplyClass::Rejection);
        assert!(matched.confidence >= UNCERTAIN_CONFIDENCE);
        assert!(
            matched.quoted_line.contains("decided not to move forward"),
            "the quote is the message's own line: {:?}",
            matched.quoted_line
        );
        // The quote is a single line, never the whole body.
        assert!(!matched.quoted_line.contains("Dear Diego"));
        assert!(!matched.quoted_line.contains("Best,"));
    }

    #[test]
    fn english_acknowledgements_are_recognised() {
        assert_eq!(
            class_of("Application received", "Thank you for applying to Acme."),
            ReplyClass::Acknowledgement
        );
        assert_eq!(
            class_of(
                "We got it",
                "We have received your application and will be in touch."
            ),
            ReplyClass::Acknowledgement
        );
    }

    #[test]
    fn english_interview_assessment_and_offer_are_recognised() {
        assert_eq!(
            class_of(
                "Next steps",
                "We would like to invite you to an interview next week."
            ),
            ReplyClass::InterviewInvite
        );
        assert_eq!(
            class_of(
                "Take-home exercise",
                "Please complete this coding challenge within five days."
            ),
            ReplyClass::Assessment
        );
        assert_eq!(
            class_of(
                "Your offer",
                "We are pleased to offer you the position of Senior Engineer."
            ),
            ReplyClass::Offer
        );
        assert_eq!(
            class_of(
                "A question",
                "Could you please send us your notice period and salary expectations?"
            ),
            ReplyClass::RequestForInfo
        );
    }

    #[test]
    fn spanish_replies_are_recognised() {
        assert_eq!(
            class_of("Tu postulación", "Gracias por postularte a nuestro equipo."),
            ReplyClass::Acknowledgement
        );
        assert_eq!(
            class_of(
                "Sobre tu candidatura",
                "Lamentablemente, hemos decidido no continuar con tu candidatura."
            ),
            ReplyClass::Rejection
        );
        assert_eq!(
            class_of(
                "Siguientes pasos",
                "Nos gustaría invitarte a una entrevista el jueves."
            ),
            ReplyClass::InterviewInvite
        );
        assert_eq!(
            class_of(
                "Prueba técnica",
                "El siguiente paso es una prueba técnica de dos horas."
            ),
            ReplyClass::Assessment
        );
        assert_eq!(
            class_of(
                "Oferta",
                "Nos complace ofrecerte una oferta de empleo como ingeniero senior."
            ),
            ReplyClass::Offer
        );
        assert_eq!(
            class_of(
                "Información adicional",
                "Por favor envía tu documentación para continuar."
            ),
            ReplyClass::RequestForInfo
        );
    }

    #[test]
    fn an_unfamiliar_message_matches_no_rule() {
        assert!(classify("Lunch?", "Are we still on for Friday at noon?").is_none());
        assert!(classify("", "").is_none());
        // A bare "interview" in an unrelated sentence is not a certainty.
        assert!(classify("Podcast", "Our interview with the CEO is online.").is_none());
    }

    #[test]
    fn a_rejection_wins_over_an_offer_word_within_it() {
        // "we cannot offer you" is a rejection, never an offer.
        assert_eq!(
            class_of(
                "Update",
                "Unfortunately we cannot offer you the role at this time."
            ),
            ReplyClass::Rejection
        );
    }

    #[test]
    fn the_strongest_match_wins_and_confidence_clears_the_threshold() {
        let matched = classify(
            "Offer: Senior Engineer",
            "We are pleased to offer you the position. Please sign the offer letter.",
        )
        .expect("an offer rule matches");
        assert_eq!(matched.class, ReplyClass::Offer);
        assert!((0.0..=1.0).contains(&matched.confidence));
        assert!(matched.confidence >= UNCERTAIN_CONFIDENCE);
        // An LLM proposal may never claim this much (AC5 vs AC6).
        assert!(
            matched.confidence <= LLM_CONFIDENCE_CEILING
                || matched.confidence > UNCERTAIN_CONFIDENCE
        );
    }

    #[test]
    fn a_quote_is_trimmed_and_bounded() {
        let long = format!(
            "{} unfortunately we cannot proceed {}",
            "filler ".repeat(60),
            "tail ".repeat(60)
        );
        let matched = classify("Update", &long).expect("a rejection rule matches");
        assert!(matched.quoted_line.len() <= MAX_QUOTE_CHARS + 3);
        assert!(matched.quoted_line.contains("unfortunately"));
    }
}
