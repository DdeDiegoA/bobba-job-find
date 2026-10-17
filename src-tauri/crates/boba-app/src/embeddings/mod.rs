//! Optional on-device embeddings: the opt-in, the model download, and the F3
//! similarity scoring reads (FR-55, AD-8; Story 6.9).
//!
//! The encoder is an adapter behind [`EmbeddingPort`]; nothing in this module
//! names `fastembed` or an ONNX runtime (AC3, hexagonal rule). What the core
//! owns is the *choice*: the user opts in (multilingual-e5-small, ~487 MB),
//! the download's destination is recorded so the network-activity view can list
//! it (AC1), and until then F3 is simply absent — the factor drops out and the
//! axis renormalises (AC2). A build without the `embeddings` feature has no
//! adapter at all: `available` is `false` and scoring is unchanged (AC4).

use std::sync::Arc;

use boba_domain::posting::Offer;
use boba_domain::scoring::f3::{self, APPROXIMATE_BYTES, MODEL_ID, Side};
use boba_domain::scoring::factors::{Prefs, ProfileSnapshot, SemanticScore};
use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use crate::ports::clock::ClockPort;
use crate::ports::embedding::{EmbeddingError, EmbeddingPort};
use crate::ports::repo::system::{SystemRepo, SystemRepoError};

/// The `setting` row that records the opt-in. Its value is the disclosure
/// marker, so a changed disclosure (a different size or model) can ask again.
pub const OPT_IN_KEY: &str = "scoring.embeddings";
/// The opt-in marker this build writes (`v1` = multilingual-e5-small, ~487 MB
/// downloaded once and then used offline).
pub const OPT_IN_MARKER: &str = "v1";
/// The `setting` row with the directory the model was downloaded into (AC1).
pub const DESTINATION_KEY: &str = "scoring.embeddings.destination";
/// The `setting` row with when the download completed (AC1).
pub const DOWNLOADED_AT_KEY: &str = "scoring.embeddings.downloaded_at";
/// The `setting` row with the model the user opted in to.
pub const MODEL_KEY: &str = "scoring.embeddings.model";

/// What the Settings screen shows about F3 (AC1, AC4). Crosses IPC (AD-3), so
/// the path is the string the user sees, not a `PathBuf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EmbeddingStatus {
    /// This build carries an encoder (`boba-embed` + the `embeddings` feature).
    pub available: bool,
    /// The user accepted the download (the opt-in).
    pub opted_in: bool,
    /// The model files are on disk.
    pub installed: bool,
    pub model_id: String,
    /// Where the model lives, once the opt-in downloaded it.
    pub destination: Option<String>,
    /// What the download costs, for the opt-in copy. `u32`: IPC carries no
    /// 64-bit integers (specta), and half a gigabyte fits with room to spare.
    pub approximate_bytes: u32,
}

/// One line the network-activity view lists (AC1): the model download, its
/// destination on disk and when it finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct NetworkEntry {
    pub model_id: String,
    pub destination: String,
    pub downloaded_at: String,
    pub approximate_bytes: u32,
}

/// Why the embeddings use case could not answer.
#[derive(Debug, Clone, Error)]
pub enum EmbeddingsError {
    /// This build has no encoder, so there is nothing to opt in to (AC4).
    #[error("this build has no embeddings support")]
    Unavailable,
    /// The adapter could not download, load or run the model.
    #[error(transparent)]
    Port(#[from] EmbeddingError),
    /// The `setting` rows could not be read or written.
    #[error(transparent)]
    System(#[from] SystemRepoError),
}

impl EmbeddingsError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Unavailable => "errors.embeddings.unavailable",
            Self::Port(EmbeddingError::NotInstalled(_)) => "errors.embeddings.not_installed",
            Self::Port(EmbeddingError::Download(_)) => "errors.embeddings.download",
            Self::Port(EmbeddingError::Encode(_)) => "errors.embeddings.encode",
            Self::System(_) => "errors.embeddings.store",
        }
    }
}

/// The opt-in, the download and the F3 input (AC1, AC2, AC4).
///
/// The port is optional because a build without the `embeddings` feature has
/// no adapter: everything still answers, just with F3 unavailable.
pub struct EmbeddingsService {
    port: Option<Arc<dyn EmbeddingPort>>,
    system: Arc<dyn SystemRepo>,
    clock: Arc<dyn ClockPort>,
}

impl EmbeddingsService {
    /// The use case over an optional adapter, the `setting` rows and the clock.
    pub fn new(
        port: Option<Arc<dyn EmbeddingPort>>,
        system: Arc<dyn SystemRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            port,
            system,
            clock,
        }
    }

    /// What F3 can do right now (AC1, AC4).
    pub fn status(&self) -> Result<EmbeddingStatus, EmbeddingsError> {
        Ok(EmbeddingStatus {
            available: self.port.is_some(),
            opted_in: self.opted_in()?,
            installed: self.port.as_ref().is_some_and(|port| port.installed()),
            model_id: MODEL_ID.to_string(),
            destination: self.system.setting_get(DESTINATION_KEY)?,
            approximate_bytes: APPROXIMATE_BYTES as u32,
        })
    }

    /// The user opted in: download the model if it is not on disk (AC1).
    ///
    /// The opt-in is recorded last, so a failed download leaves the user
    /// opted out rather than opted in to something that is not there.
    pub fn opt_in(&self) -> Result<EmbeddingStatus, EmbeddingsError> {
        let Some(port) = &self.port else {
            return Err(EmbeddingsError::Unavailable);
        };
        // Already opted in and on disk: nothing to download again.
        if !(self.opted_in()? && port.installed()) {
            let install = port.install()?;
            let now = self.clock.now_rfc3339();
            self.system
                .setting_put(MODEL_KEY, &install.model_id, &now)?;
            self.system.setting_put(
                DESTINATION_KEY,
                &install.destination.to_string_lossy(),
                &now,
            )?;
            self.system.setting_put(DOWNLOADED_AT_KEY, &now, &now)?;
            self.system.setting_put(OPT_IN_KEY, OPT_IN_MARKER, &now)?;
        }
        self.status()
    }

    /// The user turned F3 off: F3 drops out of scoring from the next run on
    /// (AC2). The model files stay on disk; nothing is deleted behind their
    /// back.
    pub fn opt_out(&self) -> Result<(), EmbeddingsError> {
        for key in [OPT_IN_KEY, MODEL_KEY, DESTINATION_KEY, DOWNLOADED_AT_KEY] {
            self.system.setting_delete(key)?;
        }
        Ok(())
    }

    /// The downloads in the network-activity view (AC1): the destination the
    /// opt-in recorded, read back from the `setting` rows so it survives a
    /// restart (the view itself is S10.4's).
    pub fn network_activity(&self) -> Result<Vec<NetworkEntry>, EmbeddingsError> {
        let Some(destination) = self.system.setting_get(DESTINATION_KEY)? else {
            return Ok(Vec::new());
        };
        let downloaded_at = self.system.setting_get(DOWNLOADED_AT_KEY)?;
        let model_id = self
            .system
            .setting_get(MODEL_KEY)?
            .unwrap_or_else(|| MODEL_ID.to_string());
        Ok(vec![NetworkEntry {
            model_id,
            destination,
            downloaded_at: downloaded_at.unwrap_or_default(),
            approximate_bytes: APPROXIMATE_BYTES as u32,
        }])
    }

    /// F3's input for one Offer — `None` whenever the factor must drop out:
    /// no opt-in (AC2), no encoder in this build (AC4), or nothing to encode.
    pub fn similarity(
        &self,
        offer: &Offer,
        profile: &ProfileSnapshot,
        prefs: &Prefs,
    ) -> Result<Option<SemanticScore>, EmbeddingsError> {
        // AC2: no opt-in (or no encoder in this build, AC4) means F3 has no
        // input — the factor drops out and renormalises; nothing is invented.
        if !self.opted_in()? {
            return Ok(None);
        }
        let Some(port) = &self.port else {
            return Ok(None);
        };
        let (Some(profile_text), Some(offer_text)) =
            (f3::profile_text(profile, prefs), f3::offer_text(offer))
        else {
            return Ok(None);
        };
        let score = port.similarity(
            &f3::encode_input(Side::Query, &profile_text),
            &f3::encode_input(Side::Passage, &offer_text),
        )?;
        Ok(Some(score))
    }

    fn opted_in(&self) -> Result<bool, EmbeddingsError> {
        Ok(self
            .system
            .setting_get(OPT_IN_KEY)?
            .as_deref()
            .is_some_and(|value| value == OPT_IN_MARKER))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::system::FixedClock;
    use crate::system::testing::FakeSystemRepo;
    use boba_domain::posting::{ApplyHints, WorkMode};
    use boba_domain::posting_features::Seniority;
    use boba_domain::scoring::f3::ModelInstall;

    const NOW: &str = "2026-10-17T09:00:00Z";

    /// A fake encoder: counts downloads, records the texts it was asked about.
    #[derive(Default)]
    struct FakeEncoder {
        installs: AtomicUsize,
        ready: AtomicBool,
        fails: AtomicBool,
        asked: Mutex<Vec<(String, String)>>,
    }

    impl FakeEncoder {
        fn destination() -> PathBuf {
            PathBuf::from("/tmp/boba-models")
        }
    }

    fn destination_string() -> String {
        FakeEncoder::destination().to_string_lossy().to_string()
    }

    impl EmbeddingPort for FakeEncoder {
        fn model_id(&self) -> &str {
            f3::MODEL_ID
        }

        fn destination(&self) -> PathBuf {
            Self::destination()
        }

        fn installed(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }

        fn install(&self) -> Result<ModelInstall, EmbeddingError> {
            self.installs.fetch_add(1, Ordering::SeqCst);
            self.ready.store(true, Ordering::SeqCst);
            Ok(ModelInstall {
                model_id: f3::MODEL_ID.to_string(),
                destination: Self::destination(),
                dimension: f3::MODEL_DIMENSION,
            })
        }

        fn similarity(
            &self,
            profile_text: &str,
            offer_text: &str,
        ) -> Result<SemanticScore, EmbeddingError> {
            if self.fails.load(Ordering::SeqCst) {
                return Err(EmbeddingError::Encode("onnx exploded".into()));
            }
            self.asked
                .lock()
                .unwrap()
                .push((profile_text.to_string(), offer_text.to_string()));
            SemanticScore::new(0.82).ok_or_else(|| EmbeddingError::Encode("bad cosine".into()))
        }
    }

    fn offer() -> Offer {
        Offer {
            id: "pst_01E5".into(),
            source_id: "remotive".into(),
            external_id: "abc-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Boba Labs".into(),
            locations: vec!["Remote".into()],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: NOW.into(),
            last_seen: NOW.into(),
            lang: "en".into(),
            description_html: "<p>Build APIs.</p>".into(),
            description_text: "Build Rust services and PostgreSQL schemas.".into(),
            requirements: Vec::new(),
            apply: ApplyHints {
                url: "https://jobs.example.com/abc-1".into(),
                email: None,
            },
        }
    }

    fn profile() -> ProfileSnapshot {
        ProfileSnapshot {
            skills: vec!["Rust".into()],
            seniority: Some(Seniority::Senior),
            years_experience: Some(8),
            education: Vec::new(),
            certifications: Vec::new(),
            languages: Vec::new(),
            locations: vec!["Bogotá".into()],
        }
    }

    fn service(fake: &Arc<FakeEncoder>) -> EmbeddingsService {
        EmbeddingsService::new(
            Some(Arc::clone(fake) as Arc<dyn EmbeddingPort>),
            Arc::new(FakeSystemRepo::default()),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    fn service_with(fake: &Arc<FakeEncoder>, system: FakeSystemRepo) -> EmbeddingsService {
        EmbeddingsService::new(
            Some(Arc::clone(fake) as Arc<dyn EmbeddingPort>),
            Arc::new(system),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    fn without_encoder() -> EmbeddingsService {
        EmbeddingsService::new(
            None,
            Arc::new(FakeSystemRepo::default()),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    // AC1: opting in downloads the model and records where it went.
    #[test]
    fn opting_in_downloads_and_lists_the_destination() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);

        let status = service.opt_in().unwrap();

        assert!(status.available);
        assert!(status.opted_in);
        assert!(status.installed);
        assert_eq!(status.destination, Some(destination_string()));
        assert_eq!(fake.installs.load(Ordering::SeqCst), 1);

        let activity = service.network_activity().unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].destination, destination_string());
        assert_eq!(activity[0].model_id, f3::MODEL_ID);
        assert_eq!(activity[0].downloaded_at, NOW);
        assert_eq!(activity[0].approximate_bytes, APPROXIMATE_BYTES as u32);
    }

    #[test]
    fn a_second_opt_in_does_not_download_again() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);

        service.opt_in().unwrap();
        let status = service.opt_in().unwrap();

        assert!(status.installed);
        assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
    }

    // AC1: with the opt-in, F3 is computed on device from the two E5 texts.
    #[test]
    fn with_the_opt_in_the_encoder_supplies_f3() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);
        service.opt_in().unwrap();

        let score = service
            .similarity(&offer(), &profile(), &Prefs::default())
            .unwrap()
            .expect("a score once opted in");

        assert_eq!(score.get(), 0.82);
        let asked = fake.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].0.starts_with("query: "));
        assert!(asked[0].1.starts_with("passage: "));
        assert!(asked[0].1.contains("Senior Rust Engineer"));
    }

    // AC2: without the opt-in the encoder is never asked; F3 drops out.
    #[test]
    fn without_the_opt_in_scoring_never_asks_the_encoder() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);

        let score = service
            .similarity(&offer(), &profile(), &Prefs::default())
            .unwrap();

        assert_eq!(score, None);
        assert!(fake.asked.lock().unwrap().is_empty());
    }

    // AC2: a Profile with nothing to encode has no side; F3 drops out.
    #[test]
    fn a_profile_with_nothing_to_say_drops_f3() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);
        service.opt_in().unwrap();

        let score = service
            .similarity(&offer(), &ProfileSnapshot::empty(), &Prefs::default())
            .unwrap();

        assert_eq!(score, None);
        assert!(fake.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn opting_out_clears_the_opt_in_and_the_listing() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);
        service.opt_in().unwrap();

        service.opt_out().unwrap();

        let status = service.status().unwrap();
        assert!(!status.opted_in);
        assert_eq!(status.destination, None);
        assert!(service.network_activity().unwrap().is_empty());
        assert_eq!(
            service
                .similarity(&offer(), &profile(), &Prefs::default())
                .unwrap(),
            None
        );
    }

    // AC4: a build without the encoder cannot opt in, and scoring is unchanged.
    #[test]
    fn a_build_without_the_encoder_has_no_f3() {
        let service = without_encoder();

        let status = service.status().unwrap();
        assert!(!status.available);
        assert!(!status.opted_in);
        assert_eq!(status.model_id, MODEL_ID);

        assert!(matches!(
            service.opt_in(),
            Err(EmbeddingsError::Unavailable)
        ));
        assert_eq!(
            service
                .similarity(&offer(), &profile(), &Prefs::default())
                .unwrap(),
            None
        );
    }

    // AD-8: an encoder failure is reported, never imputed into a score.
    #[test]
    fn an_encoder_failure_is_reported_not_invented() {
        let fake = Arc::new(FakeEncoder::default());
        let service = service(&fake);
        service.opt_in().unwrap();
        fake.fails.store(true, Ordering::SeqCst);

        let error = service
            .similarity(&offer(), &profile(), &Prefs::default())
            .unwrap_err();

        assert!(matches!(
            error,
            EmbeddingsError::Port(EmbeddingError::Encode(_))
        ));
    }

    // AC3 (hexagonal rule, AD-8): the encoder is an adapter; the core only
    // names the port. Neither this manifest nor any source file here may
    // depend on `fastembed`.
    #[test]
    fn the_core_never_depends_on_fastembed() {
        let manifest = include_str!("../../Cargo.toml");
        for line in manifest.lines() {
            let line = line.trim();
            assert!(
                !line.starts_with("fastembed") && !line.contains("\"dep:fastembed\""),
                "boba-app/Cargo.toml must not depend on fastembed: {line}"
            );
        }
        assert!(
            !manifest.contains("embeddings = ["),
            "fastembed's feature belongs to boba-embed, not boba-app"
        );

        fn scan(dir: &std::path::Path) {
            for entry in std::fs::read_dir(dir).expect("read boba-app/src") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    scan(&path);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let source = std::fs::read_to_string(&path).expect("read source");
                    // Only a real import counts; prose may name the crate.
                    let production = source
                        .split("#[cfg(test)]")
                        .next()
                        .expect("a first segment");
                    let imports = production.contains("use fastembed")
                        || production.contains("fastembed::")
                        || production.contains("dep:fastembed");
                    assert!(
                        !imports,
                        "{} imports fastembed; it belongs in boba-embed (AD-1, AD-8)",
                        path.display()
                    );
                }
            }
        }
        scan(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    }

    #[test]
    fn the_opt_in_is_read_back_from_the_settings_on_a_restart() {
        let fake = Arc::new(FakeEncoder::default());
        let mut system = FakeSystemRepo::default();
        system.seed_setting(OPT_IN_KEY, OPT_IN_MARKER);
        system.seed_setting(DESTINATION_KEY, "/tmp/boba-models");
        system.seed_setting(DOWNLOADED_AT_KEY, NOW);
        system.seed_setting(MODEL_KEY, f3::MODEL_ID);
        let service = service_with(&fake, system);

        let status = service.status().unwrap();
        assert!(status.opted_in);
        assert_eq!(status.destination, Some(destination_string()));

        let activity = service.network_activity().unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].destination, destination_string());
    }
}
