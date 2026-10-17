//! Reading an ATS form page and the scripts that drive it (S9.4, FR-82, AD-10).
//!
//! The driver evaluates [`READ_FORM_JS`] in the page and hands the JSON back
//! here; this module turns it into a [`FormSnapshot`] and decides what the page
//! is asking for. Everything here is pure — no Chrome, no network, no file IO —
//! so the reading rules are unit-tested without a browser.

use serde::Deserialize;

use boba_app::ports::browser::{Challenge, Confirmation, FormSnapshot, PageField, PageFieldKind};

/// Why a page description could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormError {
    /// The page returned something that is not a form description.
    #[error("the page description could not be read: {0}")]
    Malformed(String),
}

/// The page as the read script described it.
#[derive(Debug, Deserialize)]
struct RawPage {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    text: String,
    /// The page holds a password input: a login or account wall (AC2).
    #[serde(default)]
    password: bool,
    /// The page holds a CAPTCHA widget (AC2).
    #[serde(default)]
    captcha: bool,
    #[serde(default)]
    fields: Vec<RawField>,
}

/// One control the read script found.
#[derive(Debug, Deserialize)]
struct RawField {
    handle: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    options: Vec<String>,
}

/// Read the JSON [`READ_FORM_JS`] returns into a snapshot (AC1/AC2/AC3).
pub fn parse(json: &str) -> Result<FormSnapshot, FormError> {
    let raw: RawPage =
        serde_json::from_str(json).map_err(|e| FormError::Malformed(e.to_string()))?;
    let challenge = challenge_of(&raw);
    let confirmation =
        confirmation_of(&raw.text, &raw.url, &raw.title).map(|text| Confirmation { text });
    Ok(FormSnapshot {
        url: raw.url,
        title: raw.title,
        text: raw.text,
        fields: raw
            .fields
            .into_iter()
            .map(|field| PageField {
                handle: field.handle,
                label: field.label,
                kind: kind_of(&field.kind),
                required: field.required,
                options: field.options,
            })
            .collect(),
        challenge,
        confirmation,
    })
}

/// The script the driver evaluates to describe the page it is on.
///
/// It reports every visible control, marks each one with a `data-boba-field`
/// handle (so later scripts can address it without trusting a selector the
/// core never sees), excludes password fields from the fillable set — Boba
/// never types a password — and leaves the CAPTCHA/login and confirmation
/// decisions to [`parse`], where they are unit-tested.
pub const READ_FORM_JS: &str = r#"(() => {
  const visible = (el) => !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
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
  const skipped = ['hidden','submit','button','image','reset','password'];
  const fields = [];
  let n = 0;
  document.querySelectorAll('input, textarea, select').forEach((el) => {
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
  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: (document.body ? document.body.innerText : '').slice(0, 20000),
    password: !!document.querySelector('input[type="password"]'),
    captcha: !!document.querySelector('iframe[src*="recaptcha"], iframe[src*="hcaptcha"], iframe[src*="turnstile"], .g-recaptcha, .h-captcha, [data-sitekey]'),
    fields: fields
  });
})()"#;

/// The script that types `value` into the field `handle`.
///
/// The value is embedded as JSON — up to the surrounding `const a = …;` — so a
/// value with quotes, backslashes or a `};` in it can never become JS source.
pub fn fill_script(handle: &str, value: &str) -> String {
    let args = serde_json::json!({ "handle": handle, "value": value });
    format!(
        r#"(() => {{
  const a = {args};
  const el = document.querySelector('[data-boba-field="' + a.handle + '"]');
  if (!el) throw new Error('boba: field not found');
  el.focus();
  if (el.tagName.toLowerCase() === 'select') {{
    el.value = a.value;
  }} else {{
    const proto = el.tagName.toLowerCase() === 'textarea' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
    setter.call(el, a.value);
  }}
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return 'ok';
}})()"#
    )
}

/// The script that clicks the checkbox or radio `handle`.
pub fn choose_script(handle: &str) -> String {
    let args = serde_json::json!({ "handle": handle });
    format!(
        r#"(() => {{
  const a = {args};
  const el = document.querySelector('[data-boba-field="' + a.handle + '"]');
  if (!el) throw new Error('boba: field not found');
  el.click();
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  return 'ok';
}})()"#
    )
}

/// Map the type the script reported onto the port's kind.
fn kind_of(kind: &str) -> PageFieldKind {
    match kind.trim().to_ascii_lowercase().as_str() {
        "" | "text" => PageFieldKind::Text,
        "email" => PageFieldKind::Email,
        "tel" => PageFieldKind::Tel,
        "url" => PageFieldKind::Url,
        "number" => PageFieldKind::Number,
        "date" => PageFieldKind::Date,
        "textarea" => PageFieldKind::TextArea,
        "select" => PageFieldKind::Select,
        "checkbox" => PageFieldKind::Checkbox,
        "radio" => PageFieldKind::Radio,
        "file" => PageFieldKind::File,
        _ => PageFieldKind::Unknown,
    }
}

/// Copy that means the page is asking the user to sign in (AC2: hand control
/// back — Boba never logs in on the user's behalf, AD-10).
const LOGIN_MARKERS: &[&str] = &[
    "sign in to continue",
    "sign in to apply",
    "log in to continue",
    "log in to apply",
    "please sign in",
    "please log in",
    "you must be signed in",
    "you must be logged in",
    "create an account to continue",
    "inicia sesión para continuar",
    "inicia sesion para continuar",
    "entra para continuar",
];

/// Copy that means the submission went through (AC3, FR-88).
const CONFIRMATION_MARKERS: &[&str] = &[
    "thank you for applying",
    "thanks for applying",
    "thank you for your application",
    "application received",
    "application has been received",
    "we received your application",
    "we have received your application",
    "we've received your application",
    "application submitted",
    "your application has been submitted",
    "successfully submitted",
    "application complete",
    "gracias por postular",
    "gracias por postularte",
    "postulación recibida",
    "postulacion recibida",
    "hemos recibido tu postulación",
    "hemos recibido tu postulacion",
    "solicitud recibida",
    "hemos recibido tu solicitud",
    "postulación enviada",
    "postulacion enviada",
    "solicitud enviada",
];

/// URL path segments that mark a confirmation page, for a site whose copy is
/// thin (AC3).
const CONFIRMATION_SEGMENTS: &[&str] = &[
    "thank-you",
    "thankyou",
    "thanks",
    "confirmation",
    "confirmed",
    "application-submitted",
    "submitted",
];

/// The wall the page put up, if any (AC2).
fn challenge_of(raw: &RawPage) -> Option<Challenge> {
    if raw.captcha {
        return Some(Challenge::Captcha);
    }
    let text = raw.text.to_ascii_lowercase();
    if raw.password || LOGIN_MARKERS.iter().any(|marker| text.contains(marker)) {
        return Some(Challenge::LoginWall);
    }
    None
}

/// The evidence text for a confirmed submission, if this page is one (AC3).
fn confirmation_of(text: &str, url: &str, title: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_ascii_lowercase();
    if let Some(found) = CONFIRMATION_MARKERS
        .iter()
        .find_map(|marker| lower.find(marker).map(|at| sentence_at(&flat, at)))
    {
        return Some(found);
    }
    if confirms_by_url(url) {
        let title = title.trim();
        return Some(if title.is_empty() {
            url.to_string()
        } else {
            title.to_string()
        });
    }
    None
}

/// Whether the URL names a confirmation step.
fn confirms_by_url(url: &str) -> bool {
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let path = rest.split(['?', '#']).next().unwrap_or_default();
    path.split('/')
        .any(|segment| CONFIRMATION_SEGMENTS.contains(&segment.to_ascii_lowercase().as_str()))
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
    use super::*;

    fn page_json(fields: &str, text: &str, url: &str) -> String {
        format!(
            r#"{{"url":"{url}","title":"Apply","text":"{text}",
                "password":false,"captcha":false,"fields":[{fields}]}}"#
        )
    }

    #[test]
    fn parse_reads_the_fields_and_their_kinds() {
        let json = page_json(
            r#"{"handle":"boba-1","label":"First name","kind":"text","required":true,"options":[]},
               {"handle":"boba-2","label":"Email","kind":"email","required":true,"options":[]},
               {"handle":"boba-3","label":"Resume/CV","kind":"file","required":true,"options":[]},
               {"handle":"boba-4","label":"How did you hear about us?","kind":"select","required":false,"options":["Board","Friend"]}"#,
            "Apply to Nimbus",
            "https://jobs.lever.co/nimbus/abc",
        );

        let snapshot = parse(&json).expect("the page reads");

        assert_eq!(snapshot.url, "https://jobs.lever.co/nimbus/abc");
        assert_eq!(snapshot.title, "Apply");
        assert_eq!(snapshot.fields.len(), 4);
        assert_eq!(snapshot.fields[0].handle, "boba-1");
        assert_eq!(snapshot.fields[0].label, "First name");
        assert_eq!(snapshot.fields[0].kind, PageFieldKind::Text);
        assert!(snapshot.fields[0].required);
        assert_eq!(snapshot.fields[2].kind, PageFieldKind::File);
        assert_eq!(snapshot.fields[3].kind, PageFieldKind::Select);
        assert_eq!(snapshot.fields[3].options, vec!["Board", "Friend"]);
        assert!(snapshot.challenge.is_none(), "an ordinary form is no wall");
        assert!(snapshot.confirmation.is_none());
    }

    #[test]
    fn a_captcha_widget_hands_control_back() {
        let json = r#"{"url":"https://x.test/apply","title":"Apply","text":"Apply","password":false,"captcha":true,"fields":[]}"#;
        assert_eq!(parse(json).unwrap().challenge, Some(Challenge::Captcha));
    }

    #[test]
    fn a_password_field_is_a_login_wall() {
        let json = r#"{"url":"https://x.test/apply","title":"Apply","text":"Apply","password":true,"captcha":false,"fields":[]}"#;
        assert_eq!(parse(json).unwrap().challenge, Some(Challenge::LoginWall));
    }

    #[test]
    fn sign_in_copy_is_a_login_wall() {
        for text in [
            "Sign in to continue to Nimbus careers",
            "Please log in to apply",
            "Inicia sesión para continuar",
        ] {
            let json = page_json("", text, "https://x.test/apply");
            assert_eq!(
                parse(&json).unwrap().challenge,
                Some(Challenge::LoginWall),
                "{text}"
            );
        }
    }

    #[test]
    fn a_confirmation_sentence_is_the_evidence_text() {
        let json = page_json(
            "",
            "Thanks! Thank you for applying to Nimbus. We will be in touch.",
            "https://jobs.lever.co/nimbus/abc/apply",
        );
        let confirmation = parse(&json).unwrap().confirmation.expect("confirmed");
        assert_eq!(confirmation.text, "Thank you for applying to Nimbus.");
    }

    #[test]
    fn a_confirmation_url_counts_even_with_little_copy() {
        let json = page_json(
            "",
            "Nimbus",
            "https://jobs.ashbyhq.com/nimbus/abc/thank-you",
        );
        let confirmation = parse(&json).unwrap().confirmation.expect("confirmed");
        assert_eq!(confirmation.text, "Apply");
    }

    #[test]
    fn an_ordinary_page_is_not_a_confirmation() {
        for (text, url) in [
            ("Apply to Nimbus", "https://jobs.lever.co/nimbus/abc"),
            (
                "Application form",
                "https://boards.greenhouse.io/nimbus/jobs/1",
            ),
        ] {
            let json = page_json("", text, url);
            assert!(parse(&json).unwrap().confirmation.is_none(), "{url}");
        }
    }

    #[test]
    fn a_field_value_is_never_interpolated_raw_into_the_script() {
        let script = fill_script("boba-2", "x\"); alert(1); (\"");
        assert!(
            script.contains(r#""value":"x\"); alert(1); (\"""#),
            "the value is JSON-encoded, never spliced into JS source: {script}"
        );
        assert!(
            !script.contains("value\":x\")"),
            "no raw interpolation: {script}"
        );
    }

    #[test]
    fn the_scripts_target_the_handle_the_snapshot_reported() {
        assert!(fill_script("boba-7", "Ada").contains(r#""handle":"boba-7""#));
        assert!(choose_script("boba-9").contains(r#""handle":"boba-9""#));
    }

    #[test]
    fn malformed_page_json_is_an_error() {
        assert!(matches!(parse("not json"), Err(FormError::Malformed(_))));
    }
}
