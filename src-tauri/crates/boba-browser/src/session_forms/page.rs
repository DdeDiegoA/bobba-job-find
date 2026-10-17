//! The page side of a Session Board apply (S11.2, AD-7, AD-10).
//!
//! [`READ_APPLY_JS`] and [`OPEN_APPLY_JS`] are evaluated by the session driver
//! in the page the user is signed into. They never submit, never sign in and
//! never answer a challenge: the open script presses the board's own apply
//! button, the read script describes what is on screen. Their JSON is decoded
//! here, so every reading rule is unit-tested without a browser (the same
//! split as `boba-browser/src/forms` and the S11.1b session adapters).
//!
//! Fields, captchas and confirmations reuse `forms::parse` — a Session Board
//! apply form is an ordinary form once the modal is open. Session boards add
//! three signals a hosted ATS form does not: an "unusual activity" page, a
//! rate-limit page, and an apply form that lives in a cross-origin frame Boba
//! cannot type into. Those are classified here.

use boba_app::apply::session::{SessionChallenge, SessionPage};
use boba_app::ports::browser::Challenge;
use serde::Deserialize;

use crate::forms::{self, FormError};

/// The extra signals a Session Board page carries, on top of the shared form
/// JSON (`forms::parse` reads the rest and ignores unknown keys).
#[derive(Debug, Default, Deserialize)]
struct Signals {
    #[serde(default)]
    unusual_activity: bool,
    #[serde(default)]
    rate_limited: bool,
    #[serde(default)]
    login_wall: bool,
    /// The apply form is in a cross-origin frame: Boba cannot read or type it.
    #[serde(default)]
    frame: bool,
}

/// Read the JSON [`READ_APPLY_JS`] returns into a [`SessionPage`] (AC1).
pub fn parse(json: &str) -> Result<SessionPage, FormError> {
    let snapshot = forms::parse(json)?;
    let signals: Signals =
        serde_json::from_str(json).map_err(|error| FormError::Malformed(error.to_string()))?;
    let confirmation = snapshot.confirmation.or_else(|| {
        session_confirmation(&snapshot.text)
            .map(|text| boba_app::ports::browser::Confirmation { text })
    });
    Ok(SessionPage {
        url: snapshot.url,
        title: snapshot.title,
        text: snapshot.text,
        fields: snapshot.fields,
        challenge: challenge_of(&snapshot.challenge, &signals),
        embedded_frame: signals.frame,
        confirmation,
    })
}

/// The wall the page put up, if any. The order is the severity order the
/// session search adapters use (S11.1b): a rate-limited page that also asks to
/// sign in pauses as rate-limited, never softer.
fn challenge_of(challenge: &Option<Challenge>, signals: &Signals) -> Option<SessionChallenge> {
    if signals.rate_limited {
        return Some(SessionChallenge::RateLimited);
    }
    if signals.unusual_activity {
        return Some(SessionChallenge::UnusualActivity);
    }
    match challenge {
        Some(Challenge::Captcha) => Some(SessionChallenge::Captcha),
        Some(Challenge::LoginWall) => Some(SessionChallenge::LoginWall),
        None if signals.login_wall => Some(SessionChallenge::LoginWall),
        None => None,
    }
}

/// The script the driver evaluates to press the board's own apply button.
///
/// It opens the form and nothing else: no submit, no sign-in, no account. An
/// "Apply on company site" link is deliberately not pressed — that host is not
/// the board, so Boba hands the page to the user instead of following it.
pub const OPEN_APPLY_JS: &str = r#"(() => {
  const visible = (el) => !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
  const text = (el) => ((el && (el.innerText || el.textContent)) || '').replace(/\s+/g, ' ').trim();
  const modal = document.querySelector('.jobs-easy-apply-modal, [role="dialog"].artdeco-modal');
  if (visible(modal)) return JSON.stringify({ opened: true });
  const buttons = Array.from(document.querySelectorAll('button, a[role="button"]'));
  const easy = buttons.find((b) => visible(b) && /easy apply/i.test(text(b)));
  if (easy) { easy.click(); return JSON.stringify({ opened: true }); }
  const apply = buttons.find((b) => visible(b) && /^apply( now)?$/i.test(text(b)));
  if (apply) { apply.click(); return JSON.stringify({ opened: true }); }
  return JSON.stringify({ opened: false });
})()"#;

/// The script the driver evaluates to describe the apply step on screen.
///
/// It scopes to the board's apply surface (LinkedIn's Easy Apply modal, the
/// Indeed apply container) so the site's own header search box never reads as
/// a form field, marks each control with a `data-boba-field` handle, excludes
/// password fields from the fillable set — Boba never types a password — and
/// leaves every decision to [`parse`], where it is unit-tested.
pub const READ_APPLY_JS: &str = r#"(() => {
  const visible = (el) => !!(el && (el.offsetWidth || el.offsetHeight || el.getClientRects().length));
  const labelOf = (el) => {
    if (el.labels && el.labels.length) {
      const text = (el.labels[0].innerText || '').trim();
      if (text) return text;
    }
    return (el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('name') || '').trim();
  };
  const kindOf = (el) => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'textarea') return 'textarea';
    if (tag === 'select') return 'select';
    const type = (el.getAttribute('type') || 'text').toLowerCase();
    if (['text','email','tel','url','number','date','checkbox','radio','file'].includes(type)) return type;
    return type === '' ? 'text' : 'unknown';
  };
  const roots = [
    '.jobs-easy-apply-modal',
    '.artdeco-modal[role="dialog"]',
    '[role="dialog"]',
    '#ia-container',
    'form[data-testid="ia-ApplyForm"]',
    '#indeedapply',
    'form'
  ];
  let root = null;
  for (const selector of roots) {
    const el = document.querySelector(selector);
    if (visible(el)) { root = el; break; }
  }
  if (!root) root = document.body;
  const skipped = ['hidden','submit','button','image','reset','password'];
  const fields = [];
  let n = 0;
  root.querySelectorAll('input, textarea, select').forEach((el) => {
    if (skipped.includes((el.getAttribute('type') || '').toLowerCase())) return;
    if (el.disabled || !visible(el)) return;
    const handle = 'boba-' + (++n);
    el.setAttribute('data-boba-field', handle);
    fields.push({
      handle,
      label: labelOf(el),
      kind: kindOf(el),
      required: !!(el.required || el.getAttribute('aria-required') === 'true'),
      options: Array.from(el.options || []).map((o) => o.value).filter((v) => v !== '')
    });
  });
  const body = document.body ? (document.body.innerText || '') : '';
  const signals = body.slice(0, 4000);
  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: body.slice(0, 20000),
    password: !!document.querySelector('input[type="password"]'),
    captcha: !!document.querySelector('iframe[src*="recaptcha"], iframe[src*="hcaptcha"], iframe[src*="turnstile"], .g-recaptcha, .h-captcha, [data-sitekey]'),
    unusual_activity: /unusual activity|temporarily restricted|suspicious activity/i.test(signals),
    rate_limited: /too many requests|rate limit|rate-limit|error 429/i.test(signals),
    login_wall: /^\/(login|authwall|uas)/.test(location.pathname) || /sign in to continue|sign in to apply|log in to continue|join linkedin to apply|session expired/i.test(signals),
    frame: !!document.querySelector('iframe#indeedapply, iframe[src*="smartapply"], iframe[src*="apply.indeed"]'),
    fields: fields
  });
})()"#;

/// Copy that confirms a Session Board submission where the shared ATS list
/// (S9.4) has no marker — LinkedIn says the application "was sent".
const SESSION_CONFIRMATION_MARKERS: &[&str] = &[
    "your application was sent",
    "application was sent",
    "your application was received",
    "application was received",
    "application sent",
    "tu solicitud se envió",
    "tu solicitud se envio",
    "tu solicitud fue enviada",
];

/// The evidence text for a confirmed submission, if the shared markers did not
/// already find one (AC1, FR-88).
fn session_confirmation(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_ascii_lowercase();
    SESSION_CONFIRMATION_MARKERS
        .iter()
        .find_map(|marker| lower.find(marker).map(|at| sentence_at(&flat, at)))
}

/// The sentence that starts at byte `at`, capped so the evidence stays an
/// excerpt and never the whole page.
fn sentence_at(text: &str, at: usize) -> String {
    const MAX: usize = 240;
    let rest = &text[at..];
    let end = rest
        .char_indices()
        .take_while(|(index, _)| *index < MAX)
        .find(|(_, ch)| matches!(ch, '.' | '!' | '?'))
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or_else(|| rest.len().min(MAX));
    rest[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use boba_app::apply::session::SessionChallenge;
    use boba_app::ports::browser::PageFieldKind;

    use super::{OPEN_APPLY_JS, READ_APPLY_JS, parse};

    fn page(flags: &[&str], fields: &str, text: &str, url: &str) -> String {
        let mut value = serde_json::json!({
            "url": url,
            "title": "Nimbus | LinkedIn",
            "text": text,
            "password": false,
            "captcha": false,
            "unusual_activity": false,
            "rate_limited": false,
            "login_wall": false,
            "frame": false,
            "fields": serde_json::from_str::<serde_json::Value>(&format!("[{fields}]"))
                .expect("the fixture lists fields"),
        });
        for flag in flags {
            value[*flag] = serde_json::Value::Bool(true);
        }
        value.to_string()
    }

    #[test]
    fn a_linkedin_step_reads_its_fields() {
        let json = page(
            &[],
            r#"{"handle":"boba-1","label":"Email address","kind":"email","required":true,"options":[]},
               {"handle":"boba-2","label":"Mobile phone number","kind":"tel","required":true,"options":[]},
               {"handle":"boba-3","label":"Resume","kind":"file","required":true,"options":[]}"#,
            "Contact info",
            "https://www.linkedin.com/jobs/view/123/",
        );

        let page = parse(&json).expect("the step reads");

        assert_eq!(page.url, "https://www.linkedin.com/jobs/view/123/");
        assert_eq!(page.fields.len(), 3);
        assert_eq!(page.fields[0].label, "Email address");
        assert_eq!(page.fields[0].kind, PageFieldKind::Email);
        assert!(page.fields[0].required);
        assert_eq!(page.fields[2].kind, PageFieldKind::File);
        assert!(page.challenge.is_none(), "an ordinary step is no wall");
        assert!(!page.embedded_frame);
        assert!(page.confirmation.is_none());
    }

    #[test]
    fn an_easy_apply_confirmation_is_the_evidence_text() {
        let json = page(
            &[],
            "",
            "Your application was sent to Nimbus. You can track it in My Jobs.",
            "https://www.linkedin.com/jobs/view/123/",
        );
        let confirmation = parse(&json).unwrap().confirmation.expect("confirmed");
        assert_eq!(confirmation.text, "Your application was sent to Nimbus.");
    }

    #[test]
    fn a_human_check_is_a_captcha() {
        let json = page(
            &["captcha"],
            "",
            "Let's do a quick security check",
            "https://www.linkedin.com/jobs/view/123/",
        );
        assert_eq!(
            parse(&json).unwrap().challenge,
            Some(SessionChallenge::Captcha)
        );
    }

    #[test]
    fn a_sign_in_wall_is_a_login_wall() {
        let json = page(
            &["login_wall"],
            "",
            "Sign in to continue to LinkedIn",
            "https://www.linkedin.com/jobs/view/123/",
        );
        assert_eq!(
            parse(&json).unwrap().challenge,
            Some(SessionChallenge::LoginWall)
        );
    }

    #[test]
    fn an_unusual_activity_warning_is_its_own_challenge() {
        let json = page(
            &["unusual_activity"],
            "",
            "We noticed unusual activity on your account. Please sign in.",
            "https://www.linkedin.com/jobs/view/123/",
        );
        assert_eq!(
            parse(&json).unwrap().challenge,
            Some(SessionChallenge::UnusualActivity)
        );
    }

    #[test]
    fn a_rate_limit_page_outranks_the_other_walls() {
        let json = page(
            &["rate_limited", "unusual_activity", "login_wall"],
            "",
            "Too many requests",
            "https://www.indeed.com/viewjob?jk=abc",
        );
        assert_eq!(
            parse(&json).unwrap().challenge,
            Some(SessionChallenge::RateLimited),
            "the most severe signal wins, as in the session search adapter"
        );
    }

    #[test]
    fn an_apply_form_in_a_frame_is_marked_and_is_not_a_challenge() {
        let json = page(
            &["frame"],
            "",
            "Apply to Nimbus",
            "https://www.indeed.com/viewjob?jk=abc",
        );
        let page = parse(&json).unwrap();
        assert!(
            page.embedded_frame,
            "the form is in a frame Boba cannot type into"
        );
        assert!(
            page.challenge.is_none(),
            "a frame is not a wall, only unreachable"
        );
    }

    #[test]
    fn malformed_page_json_is_an_error() {
        assert!(parse("not json").is_err());
    }

    #[test]
    fn the_scripts_never_submit_type_or_sign_in() {
        for script in [READ_APPLY_JS, OPEN_APPLY_JS] {
            let lower = script.to_ascii_lowercase();
            for forbidden in [
                ".submit(",
                "requestsubmit",
                "formdata",
                "document.cookie",
                "fetch(",
                "xmlhttprequest",
                "localstorage",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "a session script must never contain {forbidden:?}: {script}"
                );
            }
        }
        assert!(
            OPEN_APPLY_JS.contains("click()"),
            "the open script presses the board's own apply button"
        );
        assert!(
            READ_APPLY_JS.contains("'password'"),
            "a password field is read as a login wall, never offered for filling"
        );
    }
}
