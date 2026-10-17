//! Text extraction for Master CV import (S3.5, FR-5): the user's own PDF, DOCX,
//! plain-text and LinkedIn-export ZIP files become one text the agent run reads.
//!
//! Everything here is **read-only and in memory**: archives are opened with
//! `zip` and their entries read with `by_index`, never extracted to disk. The
//! workspace pins `zip` >= 9.0.3 (the patched line; 9.0.0–9.0.2 are yanked), so
//! the advisories `deny.toml` ignores by id for the updater's zip 4.6.1 —
//! RUSTSEC-2026-0367/0368/0369, all about extracting an archive to disk — do not
//! apply to this path at all. Reads are bounded three ways (entry count,
//! per-entry bytes, whole-archive bytes) so a crafted archive cannot decompress
//! its way through memory. A file that yields no text (a scanned PDF, a corrupt
//! archive) is [`TextError::NoText`], the "couldn't read text" case (AC2), never
//! an empty profile silently proposed from nothing.

use std::io::Read;
use std::path::PathBuf;

/// The largest single file import reads. A CV is far smaller; the cap is what
/// keeps a malformed or hostile archive from turning into unbounded memory.
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

/// The largest combined text one import hands to the agent run.
pub const MAX_TEXT_CHARS: usize = 200_000;

/// The most entries one import archive (ZIP, DOCX) may carry. A LinkedIn
/// export has a few dozen; a bomb has millions.
pub const MAX_ARCHIVE_ENTRIES: usize = 200;

/// The most uncompressed bytes one archive may yield in total. Per-entry caps
/// alone do not stop a bomb: many small entries add up the same way.
pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// The most one PDF `FlateDecode` stream may inflate to (S3.5 review F2). A
/// few hundred KB of deflated zeros expand past this; a real CV's streams do
/// not come close.
pub const MAX_PDF_STREAM_BYTES: u64 = 64 * 1024 * 1024;

/// The most a PDF's streams may inflate to in total.
pub const MAX_PDF_INFLATED_BYTES: u64 = 128 * 1024 * 1024;

/// A scanned or image-only PDF yields a few form feeds at most; below this many
/// alphanumeric characters there is no readable text layer (AC2).
const MIN_ALNUM: usize = 20;

/// One file the user picked, already read (or pasted text, with a synthetic
/// name). Bytes, never a path: the use case never touches the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The combined text of one import and the files it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// File names in the order they were merged, for the run's input.
    pub files: Vec<String>,
    /// The files' text, each under a `===== <name> =====` header.
    pub text: String,
}

/// Why an import produced no usable text. Each variant is a key the UI shows
/// (AD-3, CH7: `errors.profile_import.*`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// No text layer: a scanned PDF, an empty DOCX, a ZIP with nothing readable.
    #[error("`{file}` has no readable text")]
    NoText { file: String },
    /// A format this slice does not read.
    #[error("`{file}` is not a format Boba reads")]
    Unsupported { file: String },
    /// The file could not be read from disk.
    #[error("`{file}` could not be read")]
    Read { file: String },
    /// The combined text is larger than [`MAX_TEXT_CHARS`].
    #[error("the imported text is too large")]
    TooLarge,
}

impl TextError {
    /// The i18n key the UI shows (AD-3).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::NoText { .. } => "errors.profile_import.no_text",
            Self::Unsupported { .. } => "errors.profile_import.unsupported",
            Self::Read { .. } => "errors.profile_import.read",
            Self::TooLarge => "errors.profile_import.too_large",
        }
    }
}

/// Read the user's picked files, enforcing [`MAX_FILE_BYTES`] per file.
pub fn read_files(paths: &[PathBuf]) -> Result<Vec<ImportFile>, TextError> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let meta = std::fs::metadata(path).map_err(|_| TextError::Read { file: name.clone() })?;
        if meta.len() > MAX_FILE_BYTES as u64 {
            return Err(TextError::TooLarge);
        }
        let bytes = std::fs::read(path).map_err(|_| TextError::Read { file: name.clone() })?;
        files.push(ImportFile { name, bytes });
    }
    Ok(files)
}

/// Extract one text from the user's files (AC1: one or more files merge into a
/// single run input, so an EN and an ES CV arrive together).
pub fn extract(files: &[ImportFile]) -> Result<Extracted, TextError> {
    let mut parts = Vec::with_capacity(files.len());
    let mut names = Vec::with_capacity(files.len());
    for file in files {
        let text = extract_one(&file.name, &file.bytes)?;
        names.push(file.name.clone());
        parts.push(format!("===== {} =====\n{}", file.name, text.trim()));
    }
    let text = parts.join("\n\n");
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(TextError::TooLarge);
    }
    Ok(Extracted { files: names, text })
}

/// One file, by its extension, with the top-level text-layer floor (AC2).
/// Unknown extensions are refused rather than read as lossy text: a binary read
/// as UTF-8 is noise, not a CV.
fn extract_one(name: &str, bytes: &[u8]) -> Result<String, TextError> {
    let text = extract_raw(name, bytes)?;
    if !readable(&text) {
        return Err(TextError::NoText {
            file: name.to_string(),
        });
    }
    Ok(text)
}

/// One file, by its extension, without the text-layer floor: an archive's short
/// CSV rows are read by [`zipped_text`], which applies its own rule.
fn extract_raw(name: &str, bytes: &[u8]) -> Result<String, TextError> {
    match extension_of(name).as_str() {
        "pdf" => pdf_text(name, bytes),
        "docx" => docx_text(name, bytes),
        "zip" => zipped_text(name, bytes),
        "txt" | "md" | "csv" | "json" => Ok(String::from_utf8_lossy(bytes).into_owned()),
        _ => Err(TextError::Unsupported {
            file: name.to_string(),
        }),
    }
}

/// Whether `text` carries a text layer at all (AC2). Whitespace and form feeds
/// from an image-only PDF do not count.
fn readable(text: &str) -> bool {
    text.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_ALNUM
}

/// A PDF's text layer, or `None` when there is none. `pdf-extract` can panic on
/// a malformed file; a panic means the same thing to the user as an empty layer.
fn pdf_text(name: &str, bytes: &[u8]) -> Result<String, TextError> {
    // A compressed stream can inflate into gigabytes before `pdf-extract` ever
    // asks for it, and an OOM abort is not something `catch_unwind` can catch.
    // So the object table is scanned first (S3.5 review F2): every FlateDecode
    // stream is inflated under a byte cap and thrown away.
    if !pdf_streams_within_cap(bytes) {
        return Err(TextError::TooLarge);
    }
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    }));
    match parsed {
        Ok(Ok(text)) => Ok(text),
        // A panic or a malformed file means the same thing to the user: there
        // is no readable text layer in this PDF (AC2).
        _ => Err(TextError::NoText {
            file: name.to_string(),
        }),
    }
}

/// Whether every `FlateDecode` stream in the PDF inflates within the caps.
/// A PDF `lopdf` cannot parse is not judged here: `pdf-extract` fails on it
/// the same way, and the caller reports `NoText`.
fn pdf_streams_within_cap(bytes: &[u8]) -> bool {
    // Object streams and cross-reference streams are decoded **during** load,
    // before any of our code runs (S3.5 re-review F2), so the same cap is given
    // to the loader itself.
    //
    // A stop at the cap is the one verdict this pass owns. The lenient loader
    // swallows it (it carries on with an empty body), so the detection pass is
    // strict: strict reports the error without changing what a real CV does,
    // because every other strict failure is ignored here and left to the
    // lenient load and `pdf-extract`, which reports NoText for a file it cannot
    // read.
    if let Err(lopdf::Error::Decompress(lopdf::DecompressError::MemoryLimitExceeded { .. })) =
        lopdf::Document::load_mem_with_options(bytes, pdf_detect_options())
    {
        return false;
    }
    let Ok(document) = lopdf::Document::load_mem_with_options(bytes, pdf_load_options()) else {
        return true;
    };
    let mut total: u64 = 0;
    for object in document.objects.values() {
        let lopdf::Object::Stream(stream) = object else {
            continue;
        };
        if !stream_filters(stream)
            .iter()
            .any(|filter| filter == "FlateDecode" || filter == "Fl")
        {
            continue;
        }
        let Some(size) = inflate_len_within(&stream.content, MAX_PDF_STREAM_BYTES) else {
            return false;
        };
        total += size;
        if total > MAX_PDF_INFLATED_BYTES {
            return false;
        }
    }
    true
}

/// The loader options the pre-scan uses: the standard lenient parse (real CVs
/// are often slightly non-conforming) with the per-stream decompression cap.
fn pdf_load_options() -> lopdf::LoadOptions {
    lopdf::LoadOptions {
        max_decompressed_size: Some(MAX_PDF_STREAM_BYTES as usize),
        ..lopdf::LoadOptions::default()
    }
}

/// The detection pass's options: the same cap, strict so the loader's limit
/// error reaches this code instead of being swallowed.
fn pdf_detect_options() -> lopdf::LoadOptions {
    lopdf::LoadOptions {
        strict: true,
        ..pdf_load_options()
    }
}

/// Every filter an object names, flattened: a stream's `/Filter` is one name or
/// an array of them.
fn stream_filters(stream: &lopdf::Stream) -> Vec<String> {
    let Ok(filter) = stream.dict.get(b"Filter") else {
        return Vec::new();
    };
    fn walk(object: &lopdf::Object, names: &mut Vec<String>) {
        match object {
            lopdf::Object::Name(name) => {
                names.push(String::from_utf8_lossy(name).into_owned());
            }
            lopdf::Object::Array(items) => {
                for item in items {
                    walk(item, names);
                }
            }
            _ => {}
        }
    }
    let mut names = Vec::new();
    walk(filter, &mut names);
    names
}

/// Deflate `content` and stop one byte past `limit`. `None` means the stream
/// inflates past the cap; an undecodable stream counts as `Some(0)` — it is not
/// a bomb, and `pdf-extract` will report the malformed PDF itself.
fn inflate_len_within(content: &[u8], limit: u64) -> Option<u64> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(content).take(limit + 1);
    let mut buffer = [0u8; 16 * 1024];
    let mut total: u64 = 0;
    loop {
        match decoder.read(&mut buffer) {
            Ok(0) => return Some(total),
            Ok(read) => {
                total += read as u64;
                if total > limit {
                    return None;
                }
            }
            Err(_) => return Some(total),
        }
    }
}

/// The text of a DOCX's `word/document.xml`: every `w:t` run, a line per `w:p`.
fn docx_text(name: &str, bytes: &[u8]) -> Result<String, TextError> {
    let Some(xml) = zip_entry(bytes, "word/document.xml")? else {
        return Err(TextError::NoText {
            file: name.to_string(),
        });
    };
    let mut reader = quick_xml::Reader::from_reader(xml.as_slice());
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(quick_xml::events::Event::Text(node)) => {
                let raw = node.into_inner().into_owned();
                match quick_xml::escape::unescape(&raw) {
                    Ok(decoded) => text.push_str(&decoded),
                    Err(_) => text.push_str(&raw),
                }
            }
            Ok(quick_xml::events::Event::End(node)) => {
                if node.local_name().into_inner() == "p" {
                    text.push('\n');
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(TextError::NoText {
                    file: name.to_string(),
                });
            }
        }
        buffer.clear();
    }
    Ok(text)
}

/// A LinkedIn data export is a ZIP of CSV files (plus PDFs if the user put
/// their CV in it). Entries are read in name order so the same archive always
/// yields the same prompt.
fn zipped_text(name: &str, bytes: &[u8]) -> Result<String, TextError> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| TextError::NoText {
            file: name.to_string(),
        })?;
    // A bomb is cheap to make: refuse before opening a single entry when the
    // central directory alone is bigger than any real export.
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(TextError::TooLarge);
    }
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let Some(entry_name) = entry.enclosed_name() else {
            continue;
        };
        let entry_name = entry_name.to_string_lossy().into_owned();
        let extension = extension_of(&entry_name);
        if !matches!(
            extension.as_str(),
            "pdf" | "docx" | "txt" | "md" | "csv" | "json"
        ) {
            continue;
        }
        // The declared size is checked first (it lies in a hostile archive, so
        // the `take` below still bounds the read), then the bytes actually
        // produced are counted against the per-entry and whole-archive caps.
        if entry.size() > MAX_FILE_BYTES as u64 {
            return Err(TextError::TooLarge);
        }
        let mut content = Vec::new();
        if entry
            .by_ref()
            .take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .is_err()
        {
            continue;
        }
        if content.len() > MAX_FILE_BYTES {
            return Err(TextError::TooLarge);
        }
        total += content.len() as u64;
        if total > MAX_ARCHIVE_BYTES {
            return Err(TextError::TooLarge);
        }
        entries.push((entry_name, content));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut parts = Vec::with_capacity(entries.len());
    for (entry_name, content) in entries {
        let Ok(text) = extract_raw(&entry_name, &content) else {
            continue;
        };
        // Inside an archive even a short CSV row is worth merging: the
        // scanned-PDF floor (`MIN_ALNUM`) would drop a two-line Skills.csv.
        if text.chars().any(|c| c.is_alphanumeric()) {
            parts.push(format!("===== {} =====\n{}", entry_name, text.trim()));
        }
    }
    if parts.is_empty() {
        return Err(TextError::NoText {
            file: name.to_string(),
        });
    }
    Ok(parts.join("\n\n"))
}

/// One archive entry's bytes, read in memory under the size cap. `Ok(None)` is
/// "no such entry" (the DOCX has no body); an oversized entry is [`TooLarge`].
///
/// [`TooLarge`]: TextError::TooLarge
fn zip_entry(bytes: &[u8], wanted: &str) -> Result<Option<Vec<u8>>, TextError> {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return Ok(None);
    };
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(TextError::TooLarge);
    }
    let Ok(mut entry) = archive.by_name(wanted) else {
        return Ok(None);
    };
    if entry.size() > MAX_FILE_BYTES as u64 {
        return Err(TextError::TooLarge);
    }
    let mut content = Vec::new();
    if entry
        .by_ref()
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut content)
        .is_err()
    {
        return Ok(None);
    }
    if content.len() > MAX_FILE_BYTES {
        return Err(TextError::TooLarge);
    }
    Ok(Some(content))
}

/// The lower-cased extension, without the dot.
fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn file(name: &str, bytes: impl Into<Vec<u8>>) -> ImportFile {
        ImportFile {
            name: name.to_string(),
            bytes: bytes.into(),
        }
    }

    fn zip_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
        zip_files(
            entries
                .iter()
                .map(|(name, content)| (name.to_string(), content.as_bytes().to_vec()))
                .collect(),
        )
    }

    /// A ZIP built from owned name/bytes pairs (the size-cap tests need 20 MB
    /// of compressible zeros, not a `&str`).
    fn zip_files(entries: Vec<(String, Vec<u8>)>) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(name, options).expect("start");
            writer.write_all(&content).expect("write");
        }
        writer.finish().expect("finish").into_inner()
    }

    fn docx_bytes(xml: &str) -> Vec<u8> {
        zip_bytes(&[("word/document.xml", xml)])
    }

    /// A one-page PDF whose text layer says `line`, built with lopdf (a real
    /// PDF, so `pdf-extract` is exercised end to end).
    fn pdf_bytes(line: Option<&str>) -> Vec<u8> {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let operations = match line {
            Some(line) => vec![
                lopdf::content::Operation::new("BT", vec![]),
                lopdf::content::Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                lopdf::content::Operation::new("Td", vec![72.into(), 720.into()]),
                lopdf::content::Operation::new("Tj", vec![Object::string_literal(line)]),
                lopdf::content::Operation::new("ET", vec![]),
            ],
            None => vec![],
        };
        let content = lopdf::content::Content { operations };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode"),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save");
        bytes
    }

    /// A one-page PDF whose content stream is `payload` inflated through
    /// `FlateDecode` (the real shape of most PDFs, and of a decompression bomb).
    fn pdf_bytes_deflated(payload: &[u8]) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use lopdf::{Document, Object, Stream, dictionary};
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).expect("deflate");
        let compressed = encoder.finish().expect("finish");

        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content_id = doc.add_object(Stream::new(
            dictionary! { "Filter" => "FlateDecode" },
            compressed,
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save");
        bytes
    }

    /// A valid PDF carrying `inflated` bytes of spaces as an `/ObjStm`
    /// *object stream*: `lopdf` decodes these eagerly while loading, before any
    /// caller can look at them (S3.5 re-review F2). Written by hand because
    /// lopdf's own writer skips objects typed `/ObjStm` — it regenerates those.
    fn pdf_with_object_stream_bomb(inflated: usize) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&vec![b' '; inflated])
            .expect("deflate bomb");
        let compressed = encoder.finish().expect("finish");

        let mut bomb = format!(
            "4 0 obj\n<</Type/ObjStm/N 0/First 0/Filter/FlateDecode/Length {}>>\nstream\n",
            compressed.len()
        )
        .into_bytes();
        bomb.extend_from_slice(&compressed);
        bomb.extend_from_slice(b"\nendstream\nendobj\n");

        let objects: Vec<Vec<u8>> = vec![
            b"1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n".to_vec(),
            b"2 0 obj\n<</Type/Pages/Kids[3 0 R]/Count 1>>\nendobj\n".to_vec(),
            b"3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]>>\nendobj\n".to_vec(),
            bomb,
        ];

        let mut bytes = b"%PDF-1.5\n".to_vec();
        let mut offsets = Vec::new();
        for object in &objects {
            offsets.push(bytes.len());
            bytes.extend_from_slice(object);
        }
        let xref_start = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<</Size 5/Root 1 0 R>>\nstartxref\n{xref_start}\n%%EOF\n").as_bytes(),
        );
        bytes
    }

    /// The content stream of a one-page PDF with `line` drawn on it.
    fn text_ops(line: &str) -> Vec<u8> {
        use lopdf::Object;
        lopdf::content::Content {
            operations: vec![
                lopdf::content::Operation::new("BT", vec![]),
                lopdf::content::Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                lopdf::content::Operation::new("Td", vec![72.into(), 720.into()]),
                lopdf::content::Operation::new("Tj", vec![Object::string_literal(line)]),
                lopdf::content::Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .expect("encode")
    }

    // AC1: a plain-text CV is read as-is.
    #[test]
    fn a_text_file_is_read_as_is() {
        let extracted = extract(&[file(
            "cv.txt",
            "Ada Lovelace — Backend engineer with six years of experience.",
        )])
        .expect("text");
        assert!(extracted.text.contains("Ada Lovelace"));
        assert!(extracted.text.contains("===== cv.txt ====="));
        assert_eq!(extracted.files, vec!["cv.txt"]);
    }

    // AC1: several files of different formats become one input (EN + ES CVs).
    #[test]
    fn several_files_merge_into_one_text_in_pick_order() {
        let extracted = extract(&[
            file("cv-en.txt", "English CV: Ada Lovelace, backend engineer."),
            file(
                "cv-es.txt",
                "CV español: Ada Lovelace, ingeniera de backend.",
            ),
        ])
        .expect("text");
        assert_eq!(extracted.files, vec!["cv-en.txt", "cv-es.txt"]);
        let en = extracted.text.find("English CV").expect("EN first");
        let es = extracted.text.find("CV español").expect("ES second");
        assert!(en < es);
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let error = extract(&[file("photo.png", vec![0x89, 0x50, 0x4E, 0x47])]).unwrap_err();
        assert_eq!(
            error,
            TextError::Unsupported {
                file: "photo.png".to_string()
            }
        );
        assert_eq!(error.message_key(), "errors.profile_import.unsupported");
    }

    // AC2: a scanned (text-less) PDF is the "couldn't read text" case.
    #[test]
    fn a_pdf_with_no_text_layer_is_the_no_text_error() {
        let error = extract(&[file("scan.pdf", pdf_bytes(None))]).unwrap_err();
        assert_eq!(
            error,
            TextError::NoText {
                file: "scan.pdf".to_string()
            }
        );
        assert_eq!(error.message_key(), "errors.profile_import.no_text");
    }

    #[test]
    fn a_corrupt_pdf_is_the_no_text_error_not_a_panic() {
        let error = extract(&[file("broken.pdf", "not a pdf at all".as_bytes())]).unwrap_err();
        assert!(matches!(error, TextError::NoText { .. }));
    }

    // AC1: a compressed (FlateDecode) content stream is read like any other.
    #[test]
    fn a_pdf_with_a_deflated_text_layer_is_read() {
        let extracted = extract(&[file(
            "cv.pdf",
            pdf_bytes_deflated(&text_ops("Ada Lovelace, backend engineer, six years.")),
        )])
        .expect("a deflated text layer");
        assert!(
            extracted.text.contains("Ada Lovelace"),
            "{}",
            extracted.text
        );
    }

    // S3.5 review F2: a small PDF whose streams inflate past the cap is refused
    // before `pdf-extract` can allocate its way through memory.
    #[test]
    fn a_pdf_whose_streams_inflate_past_the_cap_is_refused() {
        // 70 MB of zeros deflate to a few hundred KB: a classic bomb.
        let mut payload = vec![0u8; (MAX_PDF_STREAM_BYTES + 1024 * 1024) as usize];
        payload[0] = b'B';
        payload[1] = b'T';
        let error = extract(&[file("bomb.pdf", pdf_bytes_deflated(&payload))]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
        assert_eq!(error.message_key(), "errors.profile_import.too_large");
    }

    // S3.5 re-review F2: object streams decode during load, so the cap must be
    // given to the loader; the post-load scan then catches what the lenient
    // loader swallowed.
    #[test]
    fn a_pdf_with_a_decompression_bomb_object_stream_is_refused() {
        let bomb = pdf_with_object_stream_bomb((MAX_PDF_STREAM_BYTES + 1024 * 1024) as usize);
        let error = extract(&[file("objstm-bomb.pdf", bomb)]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
    }

    #[test]
    fn the_pdf_pre_scan_hands_the_loader_the_same_cap() {
        let options = pdf_load_options();
        assert_eq!(
            options.max_decompressed_size,
            Some(MAX_PDF_STREAM_BYTES as usize),
            "an unbounded load inflates object streams before the scan"
        );
        assert!(
            !options.strict,
            "lenient stays on: real CVs are often slightly non-conforming"
        );
    }

    // AC1: a real PDF's text layer is extracted.
    #[test]
    fn a_pdf_with_a_text_layer_is_read() {
        let extracted = extract(&[file(
            "cv.pdf",
            pdf_bytes(Some(
                "Ada Lovelace: backend engineer, six years of experience.",
            )),
        )])
        .expect("a text layer");
        assert!(
            extracted.text.contains("Ada Lovelace"),
            "got: {}",
            extracted.text
        );
    }

    // AC1: a DOCX's runs and paragraphs become lines of text.
    #[test]
    fn a_docx_is_read_paragraph_by_paragraph() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Ada Lovelace</w:t></w:r></w:p>
    <w:p><w:r><w:t>Backend engineer at </w:t></w:r><w:r><w:t>Nimbus</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let extracted = extract(&[file("cv.docx", docx_bytes(xml))]).expect("docx");
        let text = &extracted.text;
        assert!(text.contains("Ada Lovelace"), "got: {text}");
        // Runs concatenate; paragraphs separate.
        assert!(text.contains("Backend engineer at Nimbus"), "got: {text}");
        let first = text.find("Ada Lovelace").expect("first paragraph");
        let second = text.find("Backend engineer at Nimbus").expect("second");
        assert!(first < second);
    }

    #[test]
    fn a_docx_without_a_document_body_is_the_no_text_error() {
        let error = extract(&[file("empty.docx", zip_bytes(&[("foo.txt", "hi")]))]).unwrap_err();
        assert!(matches!(error, TextError::NoText { .. }));
    }

    // AC1: a LinkedIn export is a ZIP of CSVs; every readable entry is merged,
    // in name order.
    #[test]
    fn a_linkedin_export_zip_merges_its_csv_entries() {
        let archive = zip_bytes(&[
            ("Positions.csv", "Title,Company\nBackend Engineer,Nimbus\n"),
            ("Skills.csv", "Name\nRust\nSQL\n"),
        ]);
        let extracted = extract(&[file("linkedin-export.zip", archive)]).expect("zip");
        assert!(
            extracted.text.contains("Backend Engineer"),
            "{}",
            extracted.text
        );
        assert!(extracted.text.contains("Skills.csv"), "{}", extracted.text);
        let positions = extracted.text.find("Positions.csv").expect("Positions");
        let skills = extracted.text.find("Skills.csv").expect("Skills");
        assert!(positions < skills, "entries are read in name order");
    }

    #[test]
    fn an_empty_zip_is_the_no_text_error() {
        let error = extract(&[file("empty.zip", zip_bytes(&[]))]).unwrap_err();
        assert!(matches!(error, TextError::NoText { .. }));
    }

    // Zip-bomb guard 1: the entry count is capped before any entry is opened.
    #[test]
    fn an_archive_with_too_many_entries_is_refused() {
        let entries = (0..=MAX_ARCHIVE_ENTRIES)
            .map(|index| (format!("entry-{index}.csv"), b"Name\nX\n".to_vec()))
            .collect();
        let error = extract(&[file("bomb.zip", zip_files(entries))]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
    }

    // Zip-bomb guard 2: one entry bigger than the per-file cap.
    #[test]
    fn an_archive_entry_over_the_size_cap_is_refused() {
        let big = vec![b'a'; MAX_FILE_BYTES + 1];
        let archive = zip_files(vec![("big.csv".to_string(), big)]);
        let error = extract(&[file("bomb.zip", archive)]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
    }

    // Zip-bomb guard 3: many entries that are each under the per-file cap add
    // up past the whole-archive cap.
    #[test]
    fn an_archive_whose_entries_exceed_the_total_cap_is_refused() {
        let per_entry = (MAX_ARCHIVE_BYTES / 4) as usize + 1;
        let entries = (0..4)
            .map(|index| (format!("part-{index}.csv"), vec![b'a'; per_entry]))
            .collect();
        let error = extract(&[file("bomb.zip", zip_files(entries))]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
        assert_eq!(error.message_key(), "errors.profile_import.too_large");
    }

    // The DOCX body goes through the same per-entry cap.
    #[test]
    fn a_docx_body_over_the_size_cap_is_refused() {
        let big = vec![b'a'; MAX_FILE_BYTES + 1];
        let archive = zip_files(vec![("word/document.xml".to_string(), big)]);
        let error = extract(&[file("big.docx", archive)]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
    }

    #[test]
    fn an_oversized_text_is_refused() {
        let big = "a".repeat(MAX_TEXT_CHARS + 1);
        let error = extract(&[file("big.txt", big)]).unwrap_err();
        assert_eq!(error, TextError::TooLarge);
    }

    #[test]
    fn reading_files_from_disk_uses_the_real_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cv.txt");
        std::fs::write(&path, "Ada Lovelace — backend engineer, six years.").expect("write");
        let files = read_files(std::slice::from_ref(&path)).expect("read");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "cv.txt");
        assert!(
            extract(&files)
                .expect("extract")
                .text
                .contains("Ada Lovelace")
        );
    }

    #[test]
    fn a_missing_file_is_a_read_error() {
        let error = read_files(&[PathBuf::from("/definitely/not/here/cv.pdf")]).unwrap_err();
        assert!(matches!(error, TextError::Read { .. }));
    }
}
