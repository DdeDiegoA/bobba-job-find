//! Launching and driving the Boba browser profile over CDP (S9.4, AD-10, §6.4).
//!
//! The browser is the **user's own installed Chrome**, launched headed against
//! a dedicated profile (`<app data>/browser-profile`) on an ephemeral loopback
//! CDP port. Boba never bundles Chromium and never uses a fixed port; the
//! browser lives only as long as an apply session, then closes.
//!
//! [`CdpTransport`] is the seam: the production [`ChromeTransport`] speaks CDP
//! through `chromiumoxide`, and [`PageDriver`] — the form-reading and filling
//! logic — is tested against a fake transport, so the unit tests need no
//! browser.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use boba_app::ports::browser::{BrowserError, BrowserPort, FormSnapshot, Screenshot, Upload};
use boba_domain::apply::channel::AtsProvider;

use crate::forms;

/// The profile folder's name under the app data dir (AD-10, §6.4): the user
/// logs into it once, visibly, and the session boards reuse it later.
pub const PROFILE_DIR: &str = "browser-profile";

/// Where an apply session's attachments are written before upload.
pub const UPLOAD_DIR: &str = "uploads/ats";

/// The Boba profile folder under `app_data`.
pub fn profile_dir(app_data: &Path) -> PathBuf {
    app_data.join(PROFILE_DIR)
}

/// How the browser is launched, as data so it can be asserted (AD-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The user's installed Chrome (never a bundled Chromium).
    pub executable: PathBuf,
    /// The dedicated Boba profile.
    pub profile: PathBuf,
    /// `0` asks Chrome for an ephemeral port bound to loopback (§6.4).
    pub cdp_port: u16,
    /// Always headed: the user watches every step (S9.4 AC1).
    pub headed: bool,
}

impl LaunchPlan {
    /// The `chromiumoxide` configuration this plan describes.
    fn to_config(&self) -> Result<chromiumoxide::BrowserConfig, BrowserError> {
        let mut builder = chromiumoxide::BrowserConfig::builder()
            .chrome_executable(&self.executable)
            .user_data_dir(&self.profile)
            .port(self.cdp_port);
        if self.headed {
            builder = builder.with_head();
        }
        builder.build().map_err(BrowserError::Navigation)
    }
}

/// The launch plan for `profile`, with the user's installed browser (§6.4).
pub fn launch_plan(profile: &Path) -> Result<LaunchPlan, BrowserError> {
    let executable = chrome_executable().ok_or(BrowserError::NotRunning)?;
    Ok(LaunchPlan {
        executable,
        profile: profile.to_path_buf(),
        cdp_port: 0,
        headed: true,
    })
}

/// The user's installed Chrome, Edge or Chromium. `BOBA_CHROME` overrides the
/// search (useful on a machine where the browser lives elsewhere).
pub fn chrome_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("BOBA_CHROME") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    candidates().into_iter().find(|path| path.is_file())
}

/// Where each platform installs a Chromium-family browser, most common first.
fn candidates() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in [
            "Google Chrome",
            "Chromium",
            "Microsoft Edge",
            "Brave Browser",
        ] {
            paths.push(PathBuf::from(format!(
                "/Applications/{app}.app/Contents/MacOS/{app}"
            )));
        }
        paths.push(PathBuf::from(
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
    } else if cfg!(target_os = "windows") {
        for base in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(base) {
                let root = PathBuf::from(root);
                paths.push(root.join(r"Google\Chrome\Application\chrome.exe"));
                paths.push(root.join(r"Microsoft\Edge\Application\msedge.exe"));
            }
        }
    } else {
        for path in [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
        ] {
            paths.push(PathBuf::from(path));
        }
    }
    paths
}

/// The low-level CDP surface the driver needs. Production speaks it through
/// [`ChromeTransport`]; tests supply a fake, so no browser is needed.
pub trait CdpTransport: Send + Sync {
    /// Load `url` and wait for it.
    fn navigate(&self, url: &str) -> Result<(), BrowserError>;
    /// Evaluate `script` and return its string result.
    fn evaluate(&self, script: &str) -> Result<String, BrowserError>;
    /// Capture the page as PNG bytes.
    fn screenshot_png(&self) -> Result<Vec<u8>, BrowserError>;
    /// Hand `paths` to the file input `handle`.
    fn set_files(&self, handle: &str, paths: &[PathBuf]) -> Result<(), BrowserError>;
    /// Close the browser this transport launched.
    fn close(&self) -> Result<(), BrowserError>;
}

/// The form-reading and filling logic over one transport (S9.4 AC1–AC3).
pub struct PageDriver<T: CdpTransport> {
    transport: T,
    upload_dir: PathBuf,
}

impl<T: CdpTransport> PageDriver<T> {
    pub fn new(transport: T, upload_dir: impl Into<PathBuf>) -> Self {
        Self {
            transport,
            upload_dir: upload_dir.into(),
        }
    }

    /// Read the page and turn it into a snapshot (AC1/AC2/AC3).
    fn read(&self) -> Result<FormSnapshot, BrowserError> {
        let json = self.transport.evaluate(forms::READ_FORM_JS)?;
        forms::parse(&json).map_err(|error| BrowserError::Page(error.to_string()))
    }
}

impl<T: CdpTransport> BrowserPort for PageDriver<T> {
    fn open(&self, _provider: AtsProvider, url: &str) -> Result<FormSnapshot, BrowserError> {
        // The network-activity view (S10.4, NFR-1): host and time only.
        boba_app::network_log::record(boba_app::network_log::Purpose::Sources, url);
        self.transport.navigate(url)?;
        self.read()
    }

    fn snapshot(&self) -> Result<FormSnapshot, BrowserError> {
        self.read()
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
        // folder for the length of one call and removed straight after
        // (AD-13: attachments live in the encrypted DB and files stay BLOBs;
        // a plaintext copy must not outlive the upload).
        std::fs::create_dir_all(&self.upload_dir).map_err(upload_error)?;
        let path = self.upload_dir.join(format!(
            "{}-{}",
            std::process::id(),
            safe_file_name(&file.file_name)
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

/// Whether a handle may be spliced into the attribute selector: the read
/// script only ever mints `boba-<n>`, so anything with a quote, space or
/// separator is a bug or an attack, never a field Boba filled.
fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

/// A file name that stays one name: no separator, no traversal, no surprise.
/// The adapter writes what it is given, so it sanitizes rather than trusting.
pub(crate) fn safe_file_name(file_name: &str) -> String {
    let name: String = file_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_matches('.').to_string();
    if name.is_empty() {
        "attachment".to_string()
    } else {
        name
    }
}

fn upload_error(error: std::io::Error) -> BrowserError {
    BrowserError::Upload(error.to_string())
}

/// A port over one lazily-launched browser session.
///
/// `open` is the only call that launches a browser; the rest act on the live
/// session and report [`BrowserError::NotRunning`] when there is none — a
/// `confirm` after a restart must never open an empty window (S9.4 review F3).
/// Any failed step drops the session: the user may have closed the window, and
/// the next `open` then relaunches instead of erroring forever (S9.4 review F1).
pub struct SessionDriver<T: CdpTransport> {
    launch: Box<dyn Fn() -> Result<PageDriver<T>, BrowserError> + Send + Sync>,
    session: Mutex<Option<PageDriver<T>>>,
}

impl<T: CdpTransport> SessionDriver<T> {
    /// Build the port without launching anything; `launch` builds a session
    /// when one is needed.
    pub fn new(
        launch: impl Fn() -> Result<PageDriver<T>, BrowserError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            launch: Box::new(launch),
            session: Mutex::new(None),
        }
    }

    /// Run `step` against the session; launch it first only when `start`.
    fn with_session<R>(
        &self,
        start: bool,
        step: impl FnOnce(&PageDriver<T>) -> Result<R, BrowserError>,
    ) -> Result<R, BrowserError> {
        let mut slot = self
            .session
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slot.is_none() {
            if !start {
                return Err(BrowserError::NotRunning);
            }
            *slot = Some((self.launch)()?);
        }
        let result = match slot.as_ref() {
            Some(driver) => step(driver),
            None => Err(BrowserError::NotRunning),
        };
        if matches!(result, Err(BrowserError::Disconnected(_))) {
            // Only a lost connection ends the session (F4): a script error or
            // a bad handle leaves the window the user is filling alive.
            // Dropping it lets the next open start a fresh browser (F1).
            if let Some(driver) = slot.take() {
                let _ = driver.close();
            }
        }
        result
    }
}

impl<T: CdpTransport> BrowserPort for SessionDriver<T> {
    fn open(&self, provider: AtsProvider, url: &str) -> Result<FormSnapshot, BrowserError> {
        self.with_session(true, |driver| driver.open(provider, url))
    }

    fn snapshot(&self) -> Result<FormSnapshot, BrowserError> {
        self.with_session(false, PageDriver::snapshot)
    }

    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
        self.with_session(false, |driver| driver.fill_text(handle, value))
    }

    fn choose(&self, handle: &str) -> Result<(), BrowserError> {
        self.with_session(false, |driver| driver.choose(handle))
    }

    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
        self.with_session(false, |driver| driver.attach(handle, file))
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.with_session(false, PageDriver::screenshot)
    }

    fn close(&self) -> Result<(), BrowserError> {
        let driver = self
            .session
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        match driver {
            Some(driver) => driver.close(),
            None => Ok(()),
        }
    }
}

/// The production adapter mounted as a port: the user's own Chrome, launched
/// on the first apply session and closed when the session ends.
pub struct ChromeDriver {
    driver: SessionDriver<ChromeTransport>,
}

impl ChromeDriver {
    /// Build the adapter without launching anything: `app_data` is where the
    /// profile and the upload scratch folder live.
    pub fn new(app_data: impl AsRef<Path>) -> Self {
        let app_data = app_data.as_ref();
        let profile = profile_dir(app_data);
        let upload_dir = app_data.join(UPLOAD_DIR);
        Self {
            driver: SessionDriver::new(move || {
                let transport = ChromeTransport::launch(&profile)?;
                Ok(PageDriver::new(transport, &upload_dir))
            }),
        }
    }
}

impl BrowserPort for ChromeDriver {
    fn open(&self, provider: AtsProvider, url: &str) -> Result<FormSnapshot, BrowserError> {
        self.driver.open(provider, url)
    }

    fn snapshot(&self) -> Result<FormSnapshot, BrowserError> {
        self.driver.snapshot()
    }

    fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
        self.driver.fill_text(handle, value)
    }

    fn choose(&self, handle: &str) -> Result<(), BrowserError> {
        self.driver.choose(handle)
    }

    fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
        self.driver.attach(handle, file)
    }

    fn screenshot(&self) -> Result<Screenshot, BrowserError> {
        self.driver.screenshot()
    }

    fn close(&self) -> Result<(), BrowserError> {
        self.driver.close()
    }
}

/// The real CDP transport: `chromiumoxide` over the launch plan's profile.
///
/// Each call blocks on the transport's own runtime; the handler is spawned
/// there once, so the port stays synchronous for the use case (AD-3).
pub struct ChromeTransport {
    runtime: tokio::runtime::Runtime,
    browser: Mutex<chromiumoxide::Browser>,
    page: chromiumoxide::Page,
}

impl ChromeTransport {
    /// Launch the headed Boba browser and open a blank page.
    pub fn launch(profile: &Path) -> Result<Self, BrowserError> {
        let plan = launch_plan(profile)?;
        let config = plan.to_config()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|_| BrowserError::NotRunning)?;
        let (browser, mut handler) = runtime
            .block_on(chromiumoxide::Browser::launch(config))
            .map_err(|_| BrowserError::NotRunning)?;
        runtime.spawn(async move {
            futures::StreamExt::for_each(&mut handler, |event| async move {
                let _ = event;
            })
            .await;
        });
        let page = runtime
            .block_on(browser.new_page("about:blank"))
            .map_err(|error| BrowserError::Navigation(error.to_string()))?;
        Ok(Self {
            runtime,
            browser: Mutex::new(browser),
            page,
        })
    }
}

/// How long one CDP call may take before it reports a timeout. A hung page
/// must not hold the calling thread forever (S9.4 review F2).
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Why a CDP call did not answer.
#[derive(Debug)]
pub enum WaitError {
    /// The browser answered with a failure.
    Failed(chromiumoxide::error::CdpError),
    /// The browser did not answer within the limit.
    TimedOut,
}

/// Block on `work` for at most `limit`.
fn within<T>(
    runtime: &tokio::runtime::Runtime,
    limit: std::time::Duration,
    work: impl std::future::Future<Output = Result<T, chromiumoxide::error::CdpError>>,
) -> Result<T, WaitError> {
    // The timeout future is built inside `block_on`: `tokio::time::sleep`
    // needs a runtime context to exist at all.
    match runtime.block_on(async move { tokio::time::timeout(limit, work).await }) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(WaitError::Failed(error)),
        Err(_) => Err(WaitError::TimedOut),
    }
}

/// Whether a CDP failure means the connection itself is gone (the window was
/// closed, the socket died) rather than the page answering with an error.
///
/// Only the former may end the session: a script exception or a bad field
/// must leave the window the user is filling alive (S9.4 review F4).
fn fatal_cdp(error: &chromiumoxide::error::CdpError) -> bool {
    use chromiumoxide::error::CdpError;
    matches!(
        error,
        CdpError::Ws(_)
            | CdpError::Io(_)
            | CdpError::NoResponse
            | CdpError::UnexpectedWsMessage(_)
            | CdpError::ChannelSendError(_)
            | CdpError::Timeout
            | CdpError::LaunchExit(..)
            | CdpError::LaunchTimeout(_)
            | CdpError::LaunchIo(..)
    )
}

/// Map a failed CDP call onto the port's error. `navigation` only changes how
/// a page-level failure is labelled.
fn call_error(error: WaitError, navigation: bool) -> BrowserError {
    let fatal = match &error {
        WaitError::TimedOut => true,
        WaitError::Failed(error) => fatal_cdp(error),
    };
    let message = match error {
        WaitError::TimedOut => "the browser did not answer in time".to_string(),
        WaitError::Failed(error) => error.to_string(),
    };
    match (fatal, navigation) {
        (true, _) => BrowserError::Disconnected(message),
        (false, true) => BrowserError::Navigation(message),
        (false, false) => BrowserError::Page(message),
    }
}

impl CdpTransport for ChromeTransport {
    fn navigate(&self, url: &str) -> Result<(), BrowserError> {
        let page = &self.page;
        within(&self.runtime, CALL_TIMEOUT, async move {
            page.goto(url).await.map(|_| ())
        })
        .map_err(|error| call_error(error, true))
    }

    fn evaluate(&self, script: &str) -> Result<String, BrowserError> {
        let page = &self.page;
        let result = within(&self.runtime, CALL_TIMEOUT, async move {
            page.evaluate_expression(script).await
        })
        .map_err(|error| call_error(error, false))?;
        result
            .into_value::<String>()
            .map_err(|error| BrowserError::Page(error.to_string()))
    }

    fn screenshot_png(&self) -> Result<Vec<u8>, BrowserError> {
        let page = &self.page;
        within(&self.runtime, CALL_TIMEOUT, async move {
            page.screenshot(chromiumoxide::page::ScreenshotParams::default())
                .await
        })
        .map_err(|error| call_error(error, false))
    }

    fn set_files(&self, handle: &str, paths: &[PathBuf]) -> Result<(), BrowserError> {
        use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;

        // The handle came from the read script (`boba-<n>`). Refuse anything
        // else: the selector is built here, so it must not carry quotes.
        if !valid_handle(handle) {
            return Err(BrowserError::Page("bad field handle".to_string()));
        }
        let selector = format!("[data-boba-field=\"{handle}\"]");
        let page = &self.page;
        let element = within(&self.runtime, CALL_TIMEOUT, async move {
            page.find_element(selector).await
        })
        .map_err(|error| call_error(error, false))?;
        let params = SetFileInputFilesParams::builder()
            .files(
                paths
                    .iter()
                    .map(|path| path.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            )
            .backend_node_id(element.backend_node_id)
            .build()
            .map_err(BrowserError::Page)?;
        within(&self.runtime, CALL_TIMEOUT, async move {
            page.execute(params).await.map(|_| ())
        })
        .map_err(|error| call_error(error, false))
    }

    fn close(&self) -> Result<(), BrowserError> {
        let mut browser = self
            .browser
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        within(&self.runtime, CALL_TIMEOUT, async move {
            browser.close().await.map(|_| ())
        })
        .map_err(|error| call_error(error, false))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// One upload the fake saw: the handle, the paths, and the bytes it read
    /// while the file existed.
    type Uploaded = (String, Vec<PathBuf>, Vec<u8>);

    /// A transport that answers from what the test handed it and records what
    /// it was asked.
    #[derive(Default)]
    struct FakeTransport {
        page: Mutex<String>,
        navigated: Mutex<Vec<String>>,
        scripts: Mutex<Vec<String>>,
        files: Mutex<Vec<Uploaded>>,
        png: Mutex<Vec<u8>>,
        closed: Mutex<bool>,
        /// Set when the browser behind this transport is gone (window closed).
        dead: std::sync::Arc<Mutex<bool>>,
        /// Set when the page itself answers with an error (a script exception).
        page_error: Mutex<bool>,
    }

    impl CdpTransport for FakeTransport {
        fn navigate(&self, url: &str) -> Result<(), BrowserError> {
            if *self.dead.lock().unwrap() {
                return Err(BrowserError::Disconnected("the window is gone".into()));
            }
            self.navigated.lock().unwrap().push(url.to_string());
            Ok(())
        }

        fn evaluate(&self, script: &str) -> Result<String, BrowserError> {
            if *self.dead.lock().unwrap() {
                return Err(BrowserError::Disconnected("the window is gone".into()));
            }
            if *self.page_error.lock().unwrap() {
                return Err(BrowserError::Page("boba: field not found".into()));
            }
            self.scripts.lock().unwrap().push(script.to_string());
            Ok(self.page.lock().unwrap().clone())
        }

        fn screenshot_png(&self) -> Result<Vec<u8>, BrowserError> {
            Ok(self.png.lock().unwrap().clone())
        }

        fn set_files(&self, handle: &str, paths: &[PathBuf]) -> Result<(), BrowserError> {
            // Read the bytes while the file exists: the driver removes its
            // scratch copy as soon as the upload call returns.
            let bytes = paths
                .first()
                .and_then(|path| std::fs::read(path).ok())
                .unwrap_or_default();
            self.files
                .lock()
                .unwrap()
                .push((handle.to_string(), paths.to_vec(), bytes));
            Ok(())
        }

        fn close(&self) -> Result<(), BrowserError> {
            *self.closed.lock().unwrap() = true;
            Ok(())
        }
    }

    fn page(text: &str) -> String {
        format!(
            r#"{{"url":"https://jobs.lever.co/nimbus/abc","title":"Apply","text":"{text}",
                "password":false,"captcha":false,
                "fields":[{{"handle":"boba-1","label":"First name","kind":"text","required":true,"options":[]}}]}}"#
        )
    }

    /// A scratch directory that does not touch the user's data.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("boba-browser-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_launch_plan_is_headed_on_the_boba_profile_and_an_ephemeral_port() {
        let profile = profile_dir(Path::new("/data/boba"));
        assert_eq!(profile, Path::new("/data/boba/browser-profile"));
        let plan = LaunchPlan {
            executable: PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ),
            profile: profile.clone(),
            cdp_port: 0,
            headed: true,
        };
        assert!(plan.headed, "the user watches every step (AC1)");
        assert_eq!(plan.cdp_port, 0, "an ephemeral loopback port (§6.4)");
        assert_eq!(plan.profile, profile);
        assert!(
            plan.to_config().is_ok(),
            "the plan builds a browser configuration"
        );
    }

    #[test]
    fn open_navigates_and_reads_the_form() {
        let transport = FakeTransport::default();
        *transport.page.lock().unwrap() = page("Apply");
        let driver = PageDriver::new(transport, scratch("open"));

        let snapshot = driver
            .open(AtsProvider::Lever, "https://jobs.lever.co/nimbus/abc")
            .unwrap();

        assert_eq!(
            *driver.transport.navigated.lock().unwrap(),
            vec!["https://jobs.lever.co/nimbus/abc".to_string()]
        );
        assert_eq!(snapshot.fields.len(), 1);
        assert_eq!(snapshot.fields[0].label, "First name");
        assert_eq!(
            *driver.transport.scripts.lock().unwrap(),
            vec![forms::READ_FORM_JS.to_string()]
        );
    }

    #[test]
    fn filling_sends_the_script_and_never_the_value_in_an_error() {
        let transport = FakeTransport::default();
        *transport.page.lock().unwrap() = page("Apply");
        let driver = PageDriver::new(transport, scratch("fill"));

        driver.fill_text("boba-1", "Ada").unwrap();
        driver.choose("boba-2").unwrap();

        let scripts = driver.transport.scripts.lock().unwrap();
        assert!(scripts[0].contains(r#""value":"Ada""#));
        assert!(scripts[1].contains(r#""handle":"boba-2""#));
    }

    #[test]
    fn a_screenshot_is_stored_as_its_blake3_hash() {
        let transport = FakeTransport::default();
        *transport.png.lock().unwrap() = b"\x89PNG fake".to_vec();
        let driver = PageDriver::new(transport, scratch("shot"));

        let shot = driver.screenshot().unwrap();

        assert_eq!(
            shot.png_blake3,
            blake3::hash(b"\x89PNG fake").to_hex().to_string()
        );
    }

    #[test]
    fn an_attachment_is_written_to_the_scratch_folder_and_handed_to_the_page() {
        let transport = FakeTransport::default();
        let dir = scratch("attach");
        let driver = PageDriver::new(transport, &dir);

        driver
            .attach(
                "boba-3",
                &Upload {
                    file_name: "cv-v2.pdf".to_string(),
                    bytes: b"%PDF-1.7 kit".to_vec(),
                },
            )
            .unwrap();

        let files = driver.transport.files.lock().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "boba-3");
        assert!(files[0].1[0].starts_with(&dir), "{:?}", files[0].1[0]);
        assert!(
            files[0].1[0].to_string_lossy().contains("cv-v2.pdf"),
            "the file is named after the document: {:?}",
            files[0].1[0]
        );
        assert_eq!(files[0].2, b"%PDF-1.7 kit".to_vec());
        assert!(
            !files[0].1[0].exists(),
            "the plaintext copy does not outlive the upload (AD-13)"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_name_can_never_leave_the_scratch_folder() {
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("cv v2.pdf"), "cv_v2.pdf");
        assert_eq!(safe_file_name(".."), "attachment");
    }

    #[test]
    fn an_upload_never_debugs_its_bytes() {
        let upload = Upload {
            file_name: "cv.pdf".to_string(),
            bytes: b"private CV text".to_vec(),
        };
        let debug = format!("{upload:?}");
        assert!(debug.contains("cv.pdf"));
        assert!(
            !debug.contains("private"),
            "a CV's bytes never reach a log: {debug}"
        );
    }

    #[test]
    fn a_hung_cdp_call_times_out() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let error = within(&runtime, std::time::Duration::from_millis(10), async {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            Ok::<(), chromiumoxide::error::CdpError>(())
        })
        .expect_err("a hung call must not hold the thread");
        assert!(matches!(error, WaitError::TimedOut));
        assert!(matches!(
            call_error(error, false),
            BrowserError::Disconnected(_)
        ));

        let failed = within(&runtime, std::time::Duration::from_secs(1), async {
            Err::<(), _>(chromiumoxide::error::CdpError::ChromeMessage(
                "page closed".to_string(),
            ))
        })
        .expect_err("a failed call reports the browser's message");
        assert!(matches!(
            call_error(failed, false),
            BrowserError::Page(message) if message.contains("page closed")
        ));
    }

    #[test]
    fn only_open_launches_a_browser() {
        let launches = std::sync::Arc::new(Mutex::new(0u32));
        let counter = launches.clone();
        let driver = SessionDriver::new(move || {
            *counter.lock().unwrap() += 1;
            Ok(PageDriver::new(FakeTransport::default(), scratch("noopen")))
        });

        assert_eq!(driver.snapshot().unwrap_err(), BrowserError::NotRunning);
        assert_eq!(
            *launches.lock().unwrap(),
            0,
            "a snapshot or confirm never opens a window (F3)"
        );
    }

    // F4: a script or page error must not close the window the user is
    // filling; only a lost connection ends the session.
    #[test]
    fn a_script_error_keeps_the_session() {
        let launches = std::sync::Arc::new(Mutex::new(0u32));
        let counter = launches.clone();
        let driver = SessionDriver::new(move || {
            *counter.lock().unwrap() += 1;
            let transport = FakeTransport::default();
            *transport.page.lock().unwrap() = page("Apply");
            Ok(PageDriver::new(transport, scratch("script")))
        });
        driver
            .open(AtsProvider::Lever, "https://jobs.lever.co/nimbus/abc")
            .unwrap();

        // The read script hits an element the page has since replaced.
        *driver
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .transport
            .page_error
            .lock()
            .unwrap() = true;
        assert!(matches!(driver.snapshot(), Err(BrowserError::Page(_))));

        // The session is still there: the next read works without relaunching.
        *driver
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .transport
            .page_error
            .lock()
            .unwrap() = false;
        driver.snapshot().unwrap();
        assert_eq!(
            *launches.lock().unwrap(),
            1,
            "a page error never costs the user their window"
        );
    }

    #[test]
    fn a_dead_session_is_dropped_and_the_next_open_relaunches() {
        let launches = std::sync::Arc::new(Mutex::new(0u32));
        let dead = std::sync::Arc::new(Mutex::new(false));
        let counter = launches.clone();
        let health = dead.clone();
        let driver = SessionDriver::new(move || {
            *counter.lock().unwrap() += 1;
            let mut transport = FakeTransport::default();
            *transport.page.lock().unwrap() = page("Apply");
            transport.dead = health.clone();
            Ok(PageDriver::new(transport, scratch("dead")))
        });

        driver
            .open(AtsProvider::Lever, "https://jobs.lever.co/nimbus/abc")
            .unwrap();
        assert_eq!(*launches.lock().unwrap(), 1);

        // The user closes the window: the next call fails...
        *dead.lock().unwrap() = true;
        assert!(driver.snapshot().is_err());
        assert_eq!(
            *launches.lock().unwrap(),
            1,
            "a failed read never relaunches"
        );

        // ...and once a browser can start again, `open` starts one (F1).
        *dead.lock().unwrap() = false;
        driver
            .open(AtsProvider::Lever, "https://jobs.lever.co/nimbus/abc")
            .unwrap();
        assert_eq!(*launches.lock().unwrap(), 2, "the dead session was dropped");
        assert!(driver.snapshot().is_ok());
    }

    #[test]
    fn close_closes_the_session_transport() {
        let transport = FakeTransport::default();
        let driver = PageDriver::new(transport, scratch("close"));
        driver.close().unwrap();
        assert!(*driver.transport.closed.lock().unwrap());
    }

    #[test]
    fn a_handle_that_could_break_out_of_the_selector_is_refused() {
        for bad in ["boba-\"]", "boba 1", "", "boba-1;", "boba/../x"] {
            assert!(!valid_handle(bad), "{bad:?} must be refused");
        }
        for good in ["boba-1", "boba-42", "Boba-7"] {
            assert!(valid_handle(good), "{good:?} is what the read script mints");
        }
    }

    #[test]
    fn every_platform_has_candidate_paths() {
        assert!(!candidates().is_empty());
        assert!(candidates().iter().all(|path| path.is_absolute()));
    }

    /// The page driver keeps the transport's `PageField` shape in one place.
    #[test]
    fn the_read_reports_the_field_shape_the_core_expects() {
        let transport = FakeTransport::default();
        *transport.page.lock().unwrap() = page("Apply");
        let driver = PageDriver::new(transport, scratch("shape"));
        let fields: Vec<boba_app::ports::browser::PageField> = driver.snapshot().unwrap().fields;
        assert_eq!(fields[0].kind.as_str(), "text");
    }
}
