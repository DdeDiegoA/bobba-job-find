//! `BrowserPort` — drive the user's own Chrome over CDP for opt-in automation
//! (AD-10, AD-11, §10). Adapter: `boba-browser`.
//!
//! The browser is launched **headed** on the Boba automation profile (S9.4):
//! the user watches every step, logs in once and presses submit themselves.
//! The port is what the ATS Form use case drives; it never learns which CDP
//! method a step takes, so the driver can change without touching the core.
//!
//! Values typed into the page are never part of a view, an error or a log: a
//! fill carries the value to the adapter and nowhere else (S9.4 AC1).

use boba_domain::apply::channel::AtsProvider;

/// What kind of control a page field is, as the page described it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFieldKind {
    Text,
    Email,
    Tel,
    Url,
    Number,
    Date,
    TextArea,
    Select,
    Checkbox,
    Radio,
    File,
    Unknown,
}

impl PageFieldKind {
    /// The stable id a view shows (`text`, `email`, `file`…).
    pub fn as_str(self) -> &'static str {
        match self {
            PageFieldKind::Text => "text",
            PageFieldKind::Email => "email",
            PageFieldKind::Tel => "tel",
            PageFieldKind::Url => "url",
            PageFieldKind::Number => "number",
            PageFieldKind::Date => "date",
            PageFieldKind::TextArea => "textarea",
            PageFieldKind::Select => "select",
            PageFieldKind::Checkbox => "checkbox",
            PageFieldKind::Radio => "radio",
            PageFieldKind::File => "file",
            PageFieldKind::Unknown => "unknown",
        }
    }

    /// Whether the field takes typed text (select included: `value` is set).
    pub fn takes_text(self) -> bool {
        matches!(
            self,
            PageFieldKind::Text
                | PageFieldKind::Email
                | PageFieldKind::Tel
                | PageFieldKind::Url
                | PageFieldKind::Number
                | PageFieldKind::Date
                | PageFieldKind::TextArea
                | PageFieldKind::Select
        )
    }

    /// Whether the field is chosen by clicking it.
    pub fn is_click_target(self) -> bool {
        matches!(self, PageFieldKind::Checkbox | PageFieldKind::Radio)
    }
}

/// One control the page offers, as the driver read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageField {
    /// The driver's handle for the element (opaque to the core).
    pub handle: String,
    /// The question or caption the page shows for it.
    pub label: String,
    pub kind: PageFieldKind,
    /// Whether the page marks it required.
    pub required: bool,
    /// For a select or a radio group, the choices the page offers.
    pub options: Vec<String>,
}

/// Something that stops automation and hands control to the user (S9.4 AC2,
/// AD-10: Boba never solves a CAPTCHA, creates an account or logs in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Challenge {
    Captcha,
    LoginWall,
}

impl Challenge {
    pub fn as_str(self) -> &'static str {
        match self {
            Challenge::Captcha => "captcha",
            Challenge::LoginWall => "login_wall",
        }
    }
}

/// The submission confirmation a page showed (S9.4 AC3, FR-88). The text is
/// stored as evidence; the screenshot hash is taken by the driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    /// The sentence (or page title) that confirmed the submission.
    pub text: String,
}

/// One read of the page the automation is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormSnapshot {
    pub url: String,
    pub title: String,
    /// The visible text, lower-cased comparisons done by the adapter.
    pub text: String,
    pub fields: Vec<PageField>,
    /// Set when the page asks for a CAPTCHA or a login: the fill stops and the
    /// user takes over, with the window already open on the form.
    pub challenge: Option<Challenge>,
    /// Set when the page says the submission went through.
    pub confirmation: Option<Confirmation>,
}

/// One file handed to the browser for a file input (the verified PDFs).
#[derive(Clone, PartialEq, Eq)]
pub struct Upload {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for Upload {
    /// A CV is personal data: the debug form names the file, never its bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upload")
            .field("file_name", &self.file_name)
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .finish()
    }
}

/// A screenshot's evidence: the blake3 of the PNG bytes (FR-88). The pixels
/// stay out of the log, like every other payload hash (AD-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub png_blake3: String,
}

/// Why the browser could not do what the core asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    /// No browser is running for this session.
    #[error("the browser is not running")]
    NotRunning,
    /// The form's URL could not be opened.
    #[error("the browser could not open the page: {0}")]
    Navigation(String),
    /// The page could not be read or driven (a missing field, a page that
    /// navigated away, a script that failed). Never carries a field value.
    #[error("the page could not be read or driven: {0}")]
    Page(String),
    /// The attachment could not be written for the upload.
    #[error("the attachment could not be prepared: {0}")]
    Upload(String),
}

/// Drives the user's own browser for the ATS Form channel (AD-10).
///
/// Calls are synchronous: the driver owns its runtime, and the use case runs
/// off the UI thread the same way a render does (AD-3).
pub trait BrowserPort: Send + Sync {
    /// Open `url` in the headed Boba profile and read the page
    /// (S9.4 AC1). `provider` names the ATS the form belongs to.
    fn open(&self, provider: AtsProvider, url: &str) -> Result<FormSnapshot, BrowserError>;

    /// Read the page the browser is on now (after the user acted on it).
    fn snapshot(&self) -> Result<FormSnapshot, BrowserError>;

    /// Type `value` into the field `handle` the last snapshot reported.
    ///
    /// The value travels to the adapter and nowhere else: it is never part of
    /// an error, a view or a log (S9.4 AC1).
    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError>;

    /// Choose a checkbox or radio `handle` (a click).
    fn choose(&self, handle: &str) -> Result<(), BrowserError>;

    /// Hand `file` to the file input `handle` (the verified CV or letter).
    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError>;

    /// Capture the page and hash it, for the submission evidence (FR-88).
    fn screenshot(&self) -> Result<Screenshot, BrowserError>;

    /// Close the browser window this session opened.
    fn close(&self) -> Result<(), BrowserError>;
}
