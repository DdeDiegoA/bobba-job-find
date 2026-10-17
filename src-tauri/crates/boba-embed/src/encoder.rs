//! The fastembed-backed [`EmbeddingPort`] (AD-8, Story 6.9 AC1).
//!
//! One model (`multilingual-e5-small`, about 120 MB), downloaded once into the
//! cache directory the composition root hands over (app data `models/`), then
//! run on the CPU with no network at score time. The cache layout is
//! huggingface's (`models--<org>--<name>`), because `fastembed` downloads
//! through `hf-hub`; [`EmbeddingPort::installed`] reads that layout instead of
//! trusting an in-memory flag, so a restart knows the model is there.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use boba_app::ports::embedding::{EmbeddingError, EmbeddingPort};
use boba_domain::scoring::f3::{self, ModelInstall};
use boba_domain::scoring::factors::SemanticScore;
use fastembed::similarity::cosine_similarity;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use sha2::{Digest, Sha256};

/// The on-device encoder behind the `embeddings` feature (AC4).
///
/// The model is loaded lazily — the first [`EmbeddingPort::install`] or
/// [`EmbeddingPort::similarity`] picks it up — so opening the app costs
/// nothing when the user never opted in.
pub struct FastembedEncoder {
    cache_dir: PathBuf,
    model: Mutex<Option<TextEmbedding>>,
}

impl FastembedEncoder {
    /// An encoder that keeps (and looks for) the model under `cache_dir`.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            model: Mutex::new(None),
        }
    }

    /// fastembed's options for the one model this build ships. The cache dir is
    /// ours; `HF_HOME`, when the user set it, wins inside fastembed — which is
    /// why [`EmbeddingPort::destination`] follows it too.
    fn options(&self) -> TextInitOptions {
        TextInitOptions::new(EmbeddingModel::MultilingualE5Small)
            .with_cache_dir(self.cache_dir.clone())
            .with_show_download_progress(false)
    }

    /// Run `run` on the loaded model, downloading and loading it on first use
    /// (AC1). The lock is held for the whole call: one encoder, no interleaved
    /// ONNX sessions.
    fn with_model<T>(
        &self,
        run: impl FnOnce(&mut TextEmbedding) -> Result<T, EmbeddingError>,
    ) -> Result<T, EmbeddingError> {
        let mut guard = self
            .model
            .lock()
            .map_err(|_| EmbeddingError::Encode("encoder lock poisoned".into()))?;
        if guard.is_none() {
            // The checksum runs on bytes, before ONNX parses any of them
            // (reviewer-1 F2): fetch the files with the same client fastembed
            // uses, verify, and only then let `try_new` read the cache.
            if !self.installed() {
                download_model_files(&self.destination())?;
            }
            verify_install(&self.destination())?;
            let model = TextEmbedding::try_new(self.options())
                .map_err(|error| EmbeddingError::Download(error.to_string()))?;
            *guard = Some(model);
        }
        let model = guard.as_mut().expect("the model was just loaded");
        run(model)
    }
}

impl EmbeddingPort for FastembedEncoder {
    fn model_id(&self) -> &str {
        f3::MODEL_ID
    }

    fn destination(&self) -> PathBuf {
        match std::env::var_os("HF_HOME") {
            Some(home) => PathBuf::from(home),
            None => self.cache_dir.clone(),
        }
    }

    fn installed(&self) -> bool {
        self.model.lock().is_ok_and(|guard| guard.is_some())
            || std::fs::read_dir(self.destination().join(repo_dir()))
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false)
    }

    fn install(&self) -> Result<ModelInstall, EmbeddingError> {
        self.with_model(|_| {
            Ok(ModelInstall {
                model_id: f3::MODEL_ID.to_string(),
                destination: self.destination(),
                dimension: f3::MODEL_DIMENSION,
            })
        })
    }

    fn similarity(
        &self,
        profile_text: &str,
        offer_text: &str,
    ) -> Result<SemanticScore, EmbeddingError> {
        let embeddings = self.with_model(|model| {
            model
                .embed([profile_text, offer_text], None)
                .map_err(|error| EmbeddingError::Encode(error.to_string()))
        })?;
        let [profile, offer] = embeddings.as_slice() else {
            return Err(EmbeddingError::Encode(format!(
                "expected two embeddings, got {}",
                embeddings.len()
            )));
        };
        f3::score_from_cosine(cosine_similarity(profile, offer))
            .ok_or_else(|| EmbeddingError::Encode("the encoder returned no usable cosine".into()))
    }
}

/// The one revision of `intfloat/multilingual-e5-small` this build trusts
/// (`refs/main` on 2026-10-17). An upstream that has moved fails closed: the
/// download is refused rather than silently accepted (Story 6.9, S6.9 brief).
pub const MODEL_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

/// The SHA-256 of every file `fastembed` reads for the model — the pinned
/// checksums the one download is verified against. They come from the model
/// repo at [`MODEL_REVISION`]; changing any of them is a deliberate bump.
pub const MODEL_FILES: &[(&str, &str)] = &[
    (
        "onnx/model.onnx",
        "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
    ),
    (
        "tokenizer.json",
        "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    ),
    (
        "config.json",
        "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
    ),
    (
        "special_tokens_map.json",
        "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
    ),
    (
        "tokenizer_config.json",
        "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
    ),
];

/// The marker a verified cache carries, so the 470 MB weights are hashed once
/// after the download rather than on every launch.
const VERIFIED_MARKER: &str = "boba-verified";

/// Fetch every file the model needs into the cache — raw bytes only, nothing
/// parsed (reviewer-1 F2). `try_new` afterwards finds them cached and reads
/// exactly the bytes [`verify_install`] hashed.
fn download_model_files(cache_root: &Path) -> Result<(), EmbeddingError> {
    // The network-activity view (S10.4, NFR-1) shows the attempt from the start,
    // not only the success S6.9 records afterwards: an interrupted or failed
    // download is still a destination Boba contacted on the user's behalf.
    boba_app::network_log::record_host(
        boba_app::network_log::Purpose::ModelDownload,
        boba_app::network_log::MODEL_HOST,
    );
    // Own the cache directory here so a root that cannot exist fails before the
    // network is touched, with an error that says which path is wrong.
    std::fs::create_dir_all(cache_root).map_err(|error| {
        EmbeddingError::Download(format!(
            "cannot create the model cache at {}: {error}",
            cache_root.display()
        ))
    })?;
    let api = hf_hub::api::sync::ApiBuilder::new()
        .with_cache_dir(cache_root.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|error| EmbeddingError::Download(error.to_string()))?;
    let repo = api.model(f3::MODEL_ID.to_string());
    for (path, _) in MODEL_FILES {
        repo.get(path)
            .map_err(|error| EmbeddingError::Download(format!("{path}: {error}")))?;
    }
    Ok(())
}

/// Verify the downloaded model against [`MODEL_REVISION`] and [`MODEL_FILES`]
/// (the pinned checksum the opt-in promise rests on).
///
/// A cache that already carries the verified marker for this revision is
/// trusted as-is; anything else is hashed file by file. A mismatch removes the
/// model repo directory — a corrupt cache can never be used — and reports why.
fn verify_install(cache_root: &Path) -> Result<(), EmbeddingError> {
    let repo = cache_root.join(repo_dir());
    let marker = repo.join(VERIFIED_MARKER);
    if std::fs::read_to_string(&marker).is_ok_and(|value| value.trim() == MODEL_REVISION) {
        return Ok(());
    }

    let revision_path = repo.join("refs").join("main");
    let revision = std::fs::read_to_string(&revision_path).map_err(|error| {
        EmbeddingError::Download(format!("cannot read the model revision: {error}"))
    })?;
    if revision.trim() != MODEL_REVISION {
        quarantine(&repo);
        return Err(EmbeddingError::Download(format!(
            "the model revision {} is not the pinned {MODEL_REVISION}",
            revision.trim()
        )));
    }

    let snapshot = repo.join("snapshots").join(MODEL_REVISION);
    for (path, expected) in MODEL_FILES {
        let actual = sha256(&snapshot.join(path)).map_err(|error| {
            quarantine(&repo);
            EmbeddingError::Download(format!("cannot read {path}: {error}"))
        })?;
        if actual != *expected {
            quarantine(&repo);
            return Err(EmbeddingError::Download(format!(
                "{path} failed its checksum (got {actual}, expected {expected})"
            )));
        }
    }

    std::fs::write(&marker, MODEL_REVISION).map_err(|error| {
        EmbeddingError::Download(format!("cannot record the verification: {error}"))
    })
}

/// Remove a cache that failed verification: it can never be used, and a
/// re-download is the only way back.
fn quarantine(repo: &Path) {
    if let Err(error) = std::fs::remove_dir_all(repo) {
        tracing::warn!(?error, path = %repo.display(), "embeddings: could not remove a failed cache");
    }
}

/// The SHA-256 of a file, read in chunks so the 470 MB weights never sit in
/// memory whole.
fn sha256(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// The huggingface cache directory `hf-hub` uses for the model repo.
fn repo_dir() -> String {
    format!("models--{}", f3::MODEL_ID.replace('/', "--"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoder() -> (tempfile::TempDir, FastembedEncoder) {
        let dir = tempfile::tempdir().expect("a temp cache dir");
        let encoder = FastembedEncoder::new(dir.path().to_path_buf());
        (dir, encoder)
    }

    #[test]
    fn it_reports_the_one_model_this_build_ships() {
        let (_dir, encoder) = encoder();
        assert_eq!(encoder.model_id(), "intfloat/multilingual-e5-small");
    }

    #[test]
    fn a_failed_download_is_still_disclosed_in_the_network_log() {
        // A cache root under a regular file cannot be created, so the attempt
        // fails before any network call — and the row must exist anyway.
        let dir = tempfile::tempdir().expect("a temp dir");
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, b"x").expect("write the blocker");
        let error = download_model_files(&blocker.join("cache")).expect_err("must fail");
        assert!(matches!(error, EmbeddingError::Download(_)));

        let activity = boba_app::network_log::snapshot(&[]);
        let row = activity
            .groups
            .iter()
            .find(|group| group.purpose == boba_app::network_log::Purpose::ModelDownload)
            .and_then(|group| {
                group
                    .destinations
                    .iter()
                    .find(|destination| destination.name == boba_app::network_log::MODEL_HOST)
            })
            .expect("the failed download is listed");
        assert!(row.last_seen.is_some());
    }

    #[test]
    fn it_downloads_into_the_cache_dir_it_was_given() {
        let (dir, encoder) = encoder();
        let expected = std::env::var_os("HF_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.path().to_path_buf());
        assert_eq!(encoder.destination(), expected);
        assert_eq!(repo_dir(), "models--intfloat--multilingual-e5-small");
    }

    #[test]
    fn an_empty_cache_is_not_installed() {
        let (_dir, encoder) = encoder();
        assert!(!encoder.installed());
    }

    #[test]
    fn a_cache_with_the_model_repo_is_installed() {
        let (dir, encoder) = encoder();
        std::fs::create_dir_all(dir.path().join(repo_dir()).join("snapshots"))
            .expect("create the cache layout");
        assert!(encoder.installed());
    }

    #[test]
    fn the_model_repo_name_follows_the_org_slash_name() {
        assert!(repo_dir().starts_with("models--intfloat--"));
        assert!(!repo_dir().contains('/'));
    }

    // S6.9 brief: the one download is checked against a pinned checksum.
    #[test]
    fn every_pinned_file_carries_a_sha256() {
        assert_eq!(MODEL_FILES.len(), 5, "model + tokenizer + 3 configs");
        for (path, hash) in MODEL_FILES {
            assert_eq!(hash.len(), 64, "{path} needs a full SHA-256");
            assert!(
                hash.chars().all(|c| c.is_ascii_hexdigit()),
                "{path}'s hash is not hex"
            );
        }
        assert_eq!(MODEL_REVISION.len(), 40);
    }

    #[test]
    fn the_hasher_matches_a_known_sha256() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("abc.txt");
        std::fs::write(&file, b"abc").expect("write");
        assert_eq!(
            sha256(&file).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_verified_cache_is_trusted_by_its_marker() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path().join(repo_dir());
        std::fs::create_dir_all(&repo).expect("create the repo dir");
        std::fs::write(repo.join(VERIFIED_MARKER), MODEL_REVISION).expect("write the marker");

        assert!(verify_install(dir.path()).is_ok());
    }

    /// A cache laid out the way `hf-hub` lays it, with `bytes` for every file.
    fn cache_with(bytes: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path().join(repo_dir());
        let snapshot = repo.join("snapshots").join(MODEL_REVISION);
        std::fs::create_dir_all(repo.join("refs")).expect("create refs");
        std::fs::write(repo.join("refs").join("main"), MODEL_REVISION).expect("write ref");
        for (path, _) in MODEL_FILES {
            let file = snapshot.join(path);
            std::fs::create_dir_all(file.parent().expect("a parent")).expect("create dirs");
            std::fs::write(&file, bytes).expect("write a file");
        }
        dir
    }

    #[test]
    fn the_wrong_revision_fails_closed() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = dir.path().join(repo_dir());
        std::fs::create_dir_all(repo.join("refs")).expect("create refs");
        std::fs::write(
            repo.join("refs").join("main"),
            "0000000000000000000000000000000000000000",
        )
        .expect("write ref");

        assert!(verify_install(dir.path()).is_err());
        assert!(
            !repo.exists(),
            "a cache that failed verification is removed"
        );
    }

    #[test]
    fn a_file_with_the_wrong_bytes_fails_closed_and_is_removed() {
        let dir = cache_with(b"not the model");
        let repo = dir.path().join(repo_dir());

        let error = verify_install(dir.path()).expect_err("wrong bytes");

        assert!(matches!(error, EmbeddingError::Download(_)));
        assert!(!repo.exists());
    }

    #[test]
    fn a_missing_file_fails_closed() {
        let dir = cache_with(b"anything");
        std::fs::remove_file(
            dir.path()
                .join(repo_dir())
                .join("snapshots")
                .join(MODEL_REVISION)
                .join("tokenizer.json"),
        )
        .expect("remove");

        assert!(verify_install(dir.path()).is_err());
    }
}
