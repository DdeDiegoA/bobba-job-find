//! Driving one Session Board apply in the user's own session (S11.2, AD-10).
//!
//! [`SessionPageDriver`] is the form logic over a [`CdpTransport`]; it is
//! unit-tested against a fake transport, so the tests need no browser. It
//! mirrors `crate::driver::PageDriver` (S9.4) with the session read script: a
//! Session Board form lives in a modal that opens a beat after the board's own
//! apply button is pressed, so `open` settles before it reads.
//!
//! [`SessionFormDriver`] is the production port: it launches the user's Chrome
//! headed on **that Source's** profile (S11.1a) over an ephemeral loopback port
//! (never a fixed one), keeps one session at a time, and closes it when the
//! attempt ends. Cookies stay in Chrome; no value crosses this module.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use boba_app::apply::session::{SessionFormPort, SessionPage};
use boba_app::ports::browser::{BrowserError, Screenshot, Upload};

use crate::driver::{self, CdpTransport, ChromeTransport};
use crate::forms;
use crate::session;

use super::page;

/// How many reads `open` spends waiting for the board's apply surface to show.
const SETTLE_READS: usize = 5;

/// The pause between two settle reads in the real browser (tests pass none).
const SETTLE_PAUSE: Duration = Duration::from_millis(400);

/// Where an apply session's attachments are written before upload.
pub const UPLOAD_DIR: &str = "uploads/session";

/// One Session Board page over a CDP transport (S11.2 AC1).
pub struct SessionPageDriver<T: CdpTransport> {
    transport: T,
    upload_dir: PathBuf,
    /// The wait between settle reads; a function so the tests stay instant.
    pause: Box<dyn Fn() + Send + Sync>,
}

impl<T: CdpTransport> SessionPageDriver<T> {
    pub fn new(transport: T, upload_dir: impl Into<PathBuf>) -> Self {
        Self {
            transport,
            upload_dir: upload_dir.into(),
            pause: Box::new(|| std::thread::sleep(SETTLE_PAUSE)),
        }
    }

    /// A driver whose settle wait is skipped (tests only).
    #[cfg(test)]
    pub(crate) fn with_pause(
        transport: T,
        upload_dir: impl Into<PathBuf>,
        pause: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            transport,
            upload_dir: upload_dir.into(),
            pause: Box::new(pause),
        }
    }

    /// Open `url`, press the board's own apply button and read the step it
    /// opened. Never submits and never signs in.
    fn open(&self, url: &str) -> Result<SessionPage, BrowserError> {
        self.transport.navigate(url)?;
        let opened = self.transport.evaluate(page::OPEN_APPLY_JS)?;
        let clicked = serde_json::from_str::<serde_json::Value>(&opened)
            .ok()
            .and_then(|value| value.get("opened").and_then(serde_json::Value::as_bool))
            .unwrap_or(false);
        let mut read = self.read()?;
        if !clicked {
            // No apply button was found (a sign-in wall, an "apply on the
            // company site" page): the read is the truth, not a poll loop.
            return Ok(read);
        }
        for _ in 1..SETTLE_READS {
            if settled(&read) {
                break;
            }
            (self.pause)();
            read = self.read()?;
        }
        Ok(read)
    }

    /// Read the page the session is on now.
    fn snapshot(&self) -> Result<SessionPage, BrowserError> {
        self.read()
    }

    /// The page as the session script describes it.
    fn read(&self) -> Result<SessionPage, BrowserError> {
        let json = self.transport.evaluate(page::READ_APPLY_JS)?;
        page::parse(&json).map_err(|error| BrowserError::Page(error.to_string()))
    }

    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
        self.transport
            .evaluate(&forms::fill_script(handle, value))
            .map(|_| ())
    }

    fn choose(&self, handle: &str) -> Result<(), BrowserError> {
        self.transport
            .evaluate(&forms::choose_script(handle))
            .map(|_| ())
    }

    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
        // CDP uploads take a path, so the verified PDF is written to a scratch
        // folder for the length of one call and removed straight after (AD-13:
        // a plaintext copy must not outlive the upload).
        std::fs::create_dir_all(&self.upload_dir).map_err(upload_error)?;
        let path = self.upload_dir.join(format!(
            "{}-{}",
            std::process::id(),
            driver::safe_file_name(&file.file_name)
        ));
        std::fs::write(&path, &file.bytes).map_err(upload_error)?;
        let uploaded = self
            .transport
            .set_files(handle, std::slice::from_ref(&path));
        let _ = std::fs::remove_file(&path);
        uploaded
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        let png = self.transport.screenshot_png()?;
        Ok(Screenshot {
            png_blake3: blake3::hash(&png).to_hex().to_string(),
        })
    }

    fn close(&self) -> Result<(), BrowserError> {
        self.transport.close()
    }
}

/// Whether the page has something to act on: a field set, a wall, a frame or a
/// confirmation. An empty ordinary page only means the modal has not opened.
fn settled(page: &SessionPage) -> bool {
    !page.fields.is_empty()
        || page.challenge.is_some()
        || page.embedded_frame
        || page.confirmation.is_some()
}

fn upload_error(error: std::io::Error) -> BrowserError {
    BrowserError::Upload(error.to_string())
}

/// The one live apply session: its Source id and its page driver.
type Live = (String, SessionPageDriver<ChromeTransport>);

/// The production `SessionFormPort`: one headed Chrome per Source profile.
///
/// Only one apply is live at a time (the UI applies to one Offer); opening a
/// second Source closes the first window. Dropping the session shuts Chrome
/// down, and the profile keeps its cookies for the next run (S11.1a).
pub struct SessionFormDriver {
    app_data: PathBuf,
    session: Mutex<Option<Live>>,
}

impl SessionFormDriver {
    pub fn new(app_data: impl Into<PathBuf>) -> Self {
        Self {
            app_data: app_data.into(),
            session: Mutex::new(None),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Live>> {
        self.session
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The live session, never launching one. A read after a restart must not
    /// open an empty window (S9.4 F3).
    fn current(&self) -> Result<std::sync::MutexGuard<'_, Option<Live>>, BrowserError> {
        let slot = self.slot();
        if slot.is_none() {
            return Err(BrowserError::NotRunning);
        }
        Ok(slot)
    }

    /// Close whatever session is open for another Source.
    fn retire_other(&self, source_id: &str) {
        let mut slot = self.slot();
        if slot
            .as_ref()
            .is_some_and(|(open_source, _)| open_source != source_id)
            && let Some((_, old)) = slot.take()
        {
            let _ = old.close();
        }
    }
}

impl SessionFormPort for SessionFormDriver {
    fn open(&self, source_id: &str, url: &str) -> Result<SessionPage, BrowserError> {
        // The board's own policy has already allowed this id and URL (the core
        // checks both); the profile path re-validates the id so a bad call can
        // never resolve outside `browser-sessions/`.
        let plan =
            session::launch_plan(&self.app_data, source_id).map_err(|error| match error {
                boba_domain::session::SessionError::InvalidSourceId(id) => {
                    BrowserError::Page(format!("'{id}' is not a session source id"))
                }
                _ => BrowserError::NotRunning,
            })?;
        self.retire_other(source_id);
        let mut slot = self.slot();
        if slot.is_none() {
            let transport = ChromeTransport::launch(&plan.profile)?;
            let upload_dir = self.app_data.join(UPLOAD_DIR);
            *slot = Some((
                source_id.to_string(),
                SessionPageDriver::new(transport, upload_dir),
            ));
        }
        match slot.as_ref() {
            Some((_, driver)) => driver.open(url),
            None => Err(BrowserError::NotRunning),
        }
    }

    fn snapshot(&self) -> Result<SessionPage, BrowserError> {
        self.current()?
            .as_ref()
            .map(|(_, driver)| driver.snapshot())
            .unwrap_or(Err(BrowserError::NotRunning))
    }

    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
        self.current()?
            .as_ref()
            .map(|(_, driver)| driver.fill_text(handle, value))
            .unwrap_or(Err(BrowserError::NotRunning))
    }

    fn choose(&self, handle: &str) -> Result<(), BrowserError> {
        self.current()?
            .as_ref()
            .map(|(_, driver)| driver.choose(handle))
            .unwrap_or(Err(BrowserError::NotRunning))
    }

    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
        self.current()?
            .as_ref()
            .map(|(_, driver)| driver.attach(handle, file))
            .unwrap_or(Err(BrowserError::NotRunning))
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.current()?
            .as_ref()
            .map(|(_, driver)| driver.screenshot())
            .unwrap_or(Err(BrowserError::NotRunning))
    }

    fn close(&self) -> Result<(), BrowserError> {
        let driver = self.slot().take();
        match driver {
            Some((_, driver)) => driver.close(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn read_json(fields: &str, extra: &str) -> String {
        format!(
            r#"{{"url":"https://www.linkedin.com/jobs/view/123/","title":"Nimbus",
                 "text":"Contact info","password":false,"captcha":false,
                 "unusual_activity":false,"rate_limited":false,"login_wall":false,
                 "frame":false{extra},"fields":[{fields}]}}"#
        )
    }

    /// A transport the test scripts: every call is recorded, results are queued.
    #[derive(Default)]
    struct FakeTransport {
        navigations: Mutex<Vec<String>>,
        scripts: Mutex<Vec<String>>,
        reads: Mutex<Vec<String>>,
        uploads: Mutex<Vec<(String, Vec<PathBuf>, bool)>>,
        closes: AtomicUsize,
        png: Vec<u8>,
    }

    impl FakeTransport {
        fn with_reads(reads: Vec<String>) -> Self {
            Self {
                reads: Mutex::new(reads),
                png: b"png-bytes".to_vec(),
                ..Self::default()
            }
        }
    }

    impl CdpTransport for FakeTransport {
        fn navigate(&self, url: &str) -> Result<(), BrowserError> {
            self.navigations.lock().unwrap().push(url.to_string());
            Ok(())
        }

        fn evaluate(&self, script: &str) -> Result<String, BrowserError> {
            self.scripts.lock().unwrap().push(script.to_string());
            if script == page::OPEN_APPLY_JS {
                return Ok(r#"{"opened":true}"#.to_string());
            }
            if script == page::READ_APPLY_JS {
                let mut reads = self.reads.lock().unwrap();
                return Ok(if reads.is_empty() {
                    read_json("", "")
                } else {
                    reads.remove(0)
                });
            }
            Ok("ok".to_string())
        }

        fn screenshot_png(&self) -> Result<Vec<u8>, BrowserError> {
            Ok(self.png.clone())
        }

        fn set_files(&self, handle: &str, paths: &[PathBuf]) -> Result<(), BrowserError> {
            let present = paths.first().is_some_and(|path| path.is_file());
            self.uploads
                .lock()
                .unwrap()
                .push((handle.to_string(), paths.to_vec(), present));
            Ok(())
        }

        fn close(&self) -> Result<(), BrowserError> {
            self.closes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boba-session-forms-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn driver(reads: Vec<String>) -> (SessionPageDriver<FakeTransport>, PathBuf) {
        let uploads = scratch("uploads");
        (
            SessionPageDriver::with_pause(FakeTransport::with_reads(reads), &uploads, || {}),
            uploads,
        )
    }

    #[test]
    fn open_navigates_then_reads_the_step_it_opened() {
        let (driver, root) = driver(vec![read_json(
            r#"{"handle":"boba-1","label":"Email address","kind":"email","required":true,"options":[]}"#,
            "",
        )]);

        let page = driver
            .open("https://www.linkedin.com/jobs/view/123/")
            .unwrap();

        assert_eq!(
            *driver.transport.navigations.lock().unwrap(),
            vec!["https://www.linkedin.com/jobs/view/123/".to_string()]
        );
        let scripts = driver.transport.scripts.lock().unwrap();
        assert_eq!(
            scripts.first().map(String::as_str),
            Some(page::OPEN_APPLY_JS)
        );
        assert_eq!(
            scripts.get(1).map(String::as_str),
            Some(page::READ_APPLY_JS)
        );
        assert_eq!(page.fields.len(), 1);
        assert_eq!(page.fields[0].label, "Email address");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_waits_for_a_modal_that_opens_a_beat_after_the_click() {
        let (driver, root) = driver(vec![
            read_json("", ""),
            read_json(
                r#"{"handle":"boba-1","label":"Email address","kind":"email","required":true,"options":[]}"#,
                "",
            ),
        ]);

        let page = driver
            .open("https://www.linkedin.com/jobs/view/123/")
            .unwrap();

        assert_eq!(page.fields.len(), 1, "the second read saw the modal");
        let reads = driver
            .transport
            .scripts
            .lock()
            .unwrap()
            .iter()
            .filter(|script| script.as_str() == page::READ_APPLY_JS)
            .count();
        assert_eq!(reads, 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fill_text_and_choose_evaluate_the_shared_scripts() {
        let (driver, root) = driver(vec![]);
        driver.fill_text("boba-2", "x\"); alert(1); (\"").unwrap();
        driver.choose("boba-3").unwrap();

        let scripts = driver.transport.scripts.lock().unwrap().clone();
        assert!(scripts[0].contains(r#""handle":"boba-2""#));
        assert!(
            scripts[0].contains(r#""value":"x\"); alert(1); (\"""#),
            "a value is JSON-encoded, never spliced into JS source"
        );
        assert!(scripts[1].contains(r#""handle":"boba-3""#));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn attach_writes_a_scratch_file_for_the_call_and_removes_it_after() {
        let (driver, root) = driver(vec![]);
        driver
            .attach(
                "boba-7",
                &Upload {
                    file_name: "../../cv.pdf".into(),
                    bytes: b"%PDF-1.7".to_vec(),
                },
            )
            .unwrap();

        let uploads = driver.transport.uploads.lock().unwrap();
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].0, "boba-7");
        assert!(uploads[0].2, "the file existed when CDP read it");
        let path = &uploads[0].1[0];
        assert!(
            path.starts_with(&root),
            "the scratch file stays under the upload folder"
        );
        assert_eq!(
            path.parent(),
            Some(root.as_path()),
            "a traversal in the name can never leave the upload folder"
        );
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            !name.contains('/') && !name.contains('\\'),
            "the file name keeps no separator: {name}"
        );
        assert!(!path.exists(), "no plaintext copy outlives the upload");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_screenshot_is_kept_as_a_hash_never_as_pixels() {
        let (driver, root) = driver(vec![]);
        let screenshot = driver.screenshot().unwrap();
        assert_eq!(
            screenshot.png_blake3,
            blake3::hash(b"png-bytes").to_hex().to_string()
        );
        assert_ne!(screenshot.png_blake3, "png-bytes");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn close_shuts_the_transport_down() {
        let (driver, root) = driver(vec![]);
        driver.close().unwrap();
        assert_eq!(driver.transport.closes.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_bad_source_id_never_launches_or_resolves_a_profile() {
        let root = scratch("bad-source");
        let adapter = SessionFormDriver::new(&root);
        let error = adapter.open("../linkedin", "https://www.linkedin.com/jobs/view/1");
        assert!(
            matches!(error, Err(BrowserError::Page(_))),
            "a traversal id is refused before anything launches: {error:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_read_before_open_never_launches_a_window() {
        let root = scratch("no-session");
        let adapter = SessionFormDriver::new(&root);
        assert_eq!(
            adapter.snapshot().unwrap_err(),
            BrowserError::NotRunning,
            "confirm after a restart must not open an empty browser (S9.4 F3)"
        );
        assert!(
            adapter.close().is_ok(),
            "closing an idle adapter is harmless"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_settle_rule_waits_only_while_the_page_is_an_ordinary_empty_one() {
        let empty = SessionPage::default();
        assert!(!settled(&empty));
        let mut with_fields = empty.clone();
        with_fields
            .fields
            .push(boba_app::ports::browser::PageField {
                handle: "boba-1".into(),
                label: "Email".into(),
                kind: boba_app::ports::browser::PageFieldKind::Email,
                required: true,
                options: vec![],
            });
        assert!(settled(&with_fields));
        let mut framed = empty.clone();
        framed.embedded_frame = true;
        assert!(settled(&framed), "a frame is a conclusion, not a wait");
    }
}
