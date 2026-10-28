//! A file, read as a document: what it is, told by its bytes, and its text.
//!
//! A file's name proposes it - an extension nooma reads documents by, or no
//! extension at all - and its bytes decide what it is. A PDF a browser saved
//! as `invoice` is a PDF, a Word document renamed to `.pdf` is a Word
//! document, a note with no extension is text. The name is not asked again:
//! a guess by name that turned out wrong would index a PDF's bytes as text,
//! or skip a document that was fine.
//!
//! A file that turns out to be no document - an archive, a recording - is
//! said to be one only when its name claimed it was: one named `.pdf` is
//! worth a line in the report, one with no extension is passed over without
//! a word.
//!
//! A picture is a document where its text can be read - where there is OCR
//! ([`crate::ocr`]) - and holds text: a screenshot, a photo of a page. One
//! with no text in it, a photo of a cat, is passed over without a word, as a
//! file that is no document is, and looked at again only when it changes or
//! what reads pictures does. So is a PDF page that is a picture of text.

use std::any::Any;
use std::io::{Cursor, Read as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use crate::document::{self, DocKind, Document};
use crate::encoding;

/// The extensions a file is offered to be read as a document by. A file
/// with none is offered too: its bytes say whether it is one.
const EXTENSIONS: [&str; 7] = ["md", "markdown", "txt", "text", "pdf", "docx", "epub"];

/// The extensions of pictures, offered where their text can be read: what
/// screenshots, scans and photos of pages are saved as.
const PICTURES: [&str; 12] = ["png", "jpg", "jpeg", "jfif", "tif", "tiff", "bmp", "gif", "webp", "heic", "heif", "avif"];

/// How much of a file is read to tell what it is before the rest is.
const HEAD_BYTES: usize = 8192;

/// A file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    /// What it is.
    pub kind: DocKind,
    /// Its text, as a document.
    pub document: Document,
    /// What read it, when that is not plain UTF-8: see [`reader`].
    pub reader: String,
    /// The encoding of a text file whose encoding was guessed.
    pub guessed: Option<&'static str>,
    /// Why pages that are pictures of text could not be read, when they
    /// could not: the document is read without them.
    pub unrecognized: Option<String>,
}

/// Why a file was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// It is not a document nooma reads, or too large to be one: what it is,
    /// said for a person. Reported when the file's name claimed a document.
    NotADocument(String),
    /// It is a document, and it could not be read: why, said for a person.
    /// Always reported.
    Failed(String),
    /// It is a picture with no text in it: nothing to index and nothing to
    /// report. What looked at it is kept, so it is looked at again when that
    /// changes - see [`reader`].
    NoText(String),
}

/// Whether a file is offered to be read as a document: its extension is one
/// documents have, or it has none.
pub fn offered(path: &Path) -> bool {
    path.extension().is_none() || claims_a_document(path)
}

/// Whether a file's name says it is a document: a document's extension, or
/// a picture's where pictures are read.
pub fn claims_a_document(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    let among = |known: &[&str]| known.iter().any(|known| extension.eq_ignore_ascii_case(known));
    among(&EXTENSIONS) || (among(&PICTURES) && crate::ocr::available())
}

/// What reads documents of a kind, as the library records it with every
/// file it reads: a different one now means the file is read again, even
/// unchanged. Empty for text read as UTF-8, which nothing guessed.
///
/// A PDF's reader says whether OCR read its pages that are pictures, and
/// which: a scan read before there was OCR, or before a language was added,
/// is read again by what reads it now.
pub fn reader(kind: DocKind, guessed: bool) -> String {
    match kind {
        DocKind::Pdf if crate::ocr::available() => format!("{}; {}", crate::pdf::reader(), crate::ocr::reader()),
        DocKind::Pdf => crate::pdf::reader(),
        DocKind::Image => crate::ocr::reader(),
        DocKind::Docx => crate::docx::reader(),
        DocKind::Epub => crate::epub::reader(),
        DocKind::Markdown | DocKind::Text if guessed => encoding::detector(),
        DocKind::Markdown | DocKind::Text => String::new(),
    }
}

/// Whether a file recorded as read by `recorded` would be read the same way
/// now.
pub fn reads_the_same(kind: DocKind, recorded: &str) -> bool {
    recorded == reader(kind, !recorded.is_empty())
}

/// A file's bytes, once its first ones say it may be a document and its
/// size that it is not too large to be the one they say.
pub fn load(path: &Path) -> Result<Vec<u8>, Unread> {
    let failed = |error: std::io::Error| Unread::Failed(error.to_string());
    let mut file = std::fs::File::open(path).map_err(failed)?;
    let size = file.metadata().map_err(failed)?.len();
    let mut bytes = Vec::with_capacity(usize::try_from(size.min(HEAD_BYTES as u64)).unwrap_or(HEAD_BYTES));
    file.by_ref().take(HEAD_BYTES as u64).read_to_end(&mut bytes).map_err(failed)?;
    let limit = match format_of(&bytes) {
        Format::Pdf => DocKind::Pdf.max_bytes(),
        Format::Zip | Format::Ole => DocKind::Docx.max_bytes().max(DocKind::Epub.max_bytes()),
        Format::Picture(what) if !crate::ocr::available() => return Err(Unread::NotADocument(what.to_string())),
        Format::Picture(_) => DocKind::Image.max_bytes(),
        Format::Text => {
            // A file whose first bytes are not text is not text further on.
            if let Err(not_text) = encoding::decode(text_sample(&bytes, size)) {
                return Err(Unread::NotADocument(not_text.0.to_string()));
            }
            DocKind::Text.max_bytes()
        }
        Format::Not(what) => return Err(Unread::NotADocument(what.to_string())),
    };
    if size > limit {
        return Err(Unread::NotADocument(format!("larger than {} MB", limit / 1024 / 1024)));
    }
    file.read_to_end(&mut bytes).map_err(failed)?;
    Ok(bytes)
}

/// The start of a text file, cut where a character may be cut in two: the
/// last few bytes are left to the whole file's reading.
fn text_sample(head: &[u8], size: u64) -> &[u8] {
    if head.len() as u64 >= size {
        return head;
    }
    // At most three bytes of a UTF-8 character, or one UTF-16 unit, hang
    // over the end; an even cut keeps UTF-16 whole.
    let cut = head.len().saturating_sub(4) & !1;
    &head[..cut]
}

/// Read a file's bytes as the document they are.
pub fn read(path: &Path, bytes: Vec<u8>) -> Result<Opened, Unread> {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let opened = |kind: DocKind, document: Document| Opened {
        kind,
        document,
        reader: reader(kind, false),
        guessed: None,
        unrecognized: None,
    };
    match format_of(&bytes) {
        Format::Pdf => {
            let mut paged = crate::pdf::read(bytes).map_err(Unread::Failed)?;
            let unrecognized = recognize_pictured_pages(path, &mut paged).err();
            Ok(Opened {
                unrecognized,
                ..opened(DocKind::Pdf, document::read_paged(&stem, &paged))
            })
        }
        Format::Picture(what) if !crate::ocr::available() => Err(Unread::NotADocument(what.to_string())),
        Format::Picture(_) => match crate::ocr::read_picture(&bytes).map_err(Unread::Failed)? {
            Some(text) => Ok(opened(DocKind::Image, document::read_picture(&stem, &text))),
            None => Err(Unread::NoText(reader(DocKind::Image, false))),
        },
        Format::Zip => match container_of(&bytes) {
            Some(DocKind::Docx) => Ok(opened(
                DocKind::Docx,
                document::read_blocks(&stem, &crate::docx::read(bytes).map_err(Unread::Failed)?),
            )),
            Some(DocKind::Epub) => Ok(opened(
                DocKind::Epub,
                document::read_blocks(&stem, &crate::epub::read(bytes).map_err(Unread::Failed)?),
            )),
            _ => Err(Unread::NotADocument("a zip archive, not a Word document or a book".to_string())),
        },
        Format::Ole if holds_an_encrypted_package(&bytes) => Err(Unread::Failed("protected by a password".to_string())),
        Format::Ole => Err(Unread::NotADocument(
            "an Office file of the old binary kind (.doc, .xls, .ppt), which nooma does not read".to_string(),
        )),
        Format::Not(what) => Err(Unread::NotADocument(what.to_string())),
        Format::Text => {
            let decoded = encoding::decode(&bytes).map_err(|not_text| Unread::NotADocument(not_text.0.to_string()))?;
            let markdown = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown"));
            let kind = if markdown { DocKind::Markdown } else { DocKind::Text };
            Ok(Opened {
                kind,
                document: document::read(kind, &stem, &decoded.text),
                reader: reader(kind, decoded.guessed),
                guessed: decoded.guessed.then_some(decoded.encoding),
                unrecognized: None,
            })
        }
    }
}

/// Read by OCR the pages of a PDF that hold no text, where there is OCR: the
/// pages of a scan, or one scanned page in a document typed otherwise.
///
/// Windows draws the pages from the file's bytes, read again: they were
/// given to the PDF reader, and keeping a copy of every PDF for the few
/// that are scans would double what a folder of books holds in memory.
fn recognize_pictured_pages(path: &Path, paged: &mut document::Paged) -> Result<(), String> {
    let blank: Vec<u32> = paged
        .pages
        .iter()
        .enumerate()
        .filter(|(_, text)| text.trim().is_empty())
        .filter_map(|(index, _)| u32::try_from(index).ok())
        .collect();
    if blank.is_empty() || !crate::ocr::available() {
        return Ok(());
    }
    let unread = |error: String| format!("its pages that are pictures could not be read: {error}");
    let bytes = std::fs::read(path).map_err(|error| unread(error.to_string()))?;
    let texts = crate::ocr::read_pages(&bytes, &blank).map_err(unread)?;
    for (index, text) in blank.into_iter().zip(texts) {
        if !text.trim().is_empty() {
            paged.pages[index as usize] = text;
            paged.recognized.push(index + 1);
        }
    }
    Ok(())
}

/// Read a file from disk as the document it is.
pub fn read_path(path: &Path) -> Result<Opened, Unread> {
    read(path, load(path)?)
}

/// What a file's first bytes say it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pdf,
    /// A zip archive: a Word document, a book, or neither.
    Zip,
    /// A compound file: an old Office document, or a new one under a
    /// password, which Office wraps in one.
    Ole,
    /// A picture, said for a person.
    Picture(&'static str),
    /// Bytes that may be text in some encoding.
    Text,
    /// Something else, said for a person.
    Not(&'static str),
}

/// The signatures pictures begin with, and what each is, said for a person.
/// WebP, BMP and the HEIF family are told by more than their first bytes:
/// see [`format_of`].
const PICTURE_SIGNATURES: [(&[u8], &str); 5] = [
    (b"\x89PNG\r\n\x1a\n", "a PNG image"),
    (b"\xFF\xD8\xFF", "a JPEG image"),
    (b"GIF8", "a GIF image"),
    (b"II*\x00", "a TIFF image"),
    (b"MM\x00*", "a TIFF image"),
];

/// The brands a file's `ftyp` box names when it is a picture of the HEIF
/// family rather than a video.
const PICTURE_BRANDS: [(&[u8; 4], &str); 12] = [
    (b"heic", "a HEIC image"),
    (b"heix", "a HEIC image"),
    (b"heim", "a HEIC image"),
    (b"heis", "a HEIC image"),
    (b"hevc", "a HEIC image"),
    (b"hevx", "a HEIC image"),
    (b"hevm", "a HEIC image"),
    (b"hevs", "a HEIC image"),
    (b"mif1", "a HEIF image"),
    (b"msf1", "a HEIF image"),
    (b"avif", "an AVIF image"),
    (b"avis", "an AVIF image"),
];

/// The signatures files begin with: what they are, said for a person, for
/// the kinds a folder of documents holds beside them.
const NOT_DOCUMENTS: [(&[u8], &str); 8] = [
    (b"RIFF", "a sound or a video"),
    (b"\x1F\x8B", "a gzip archive"),
    (b"7z\xBC\xAF\x27\x1C", "a 7-Zip archive"),
    (b"Rar!", "a RAR archive"),
    (b"{\\rtf", "an RTF document, which nooma does not read"),
    (b"ID3", "an MP3 recording"),
    (b"OggS", "an Ogg recording"),
    (b"fLaC", "a FLAC recording"),
];

fn format_of(head: &[u8]) -> Format {
    if head.starts_with(b"%PDF-") {
        return Format::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Format::Zip;
    }
    if head.starts_with(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
        return Format::Ole;
    }
    if let Some((_, what)) = PICTURE_SIGNATURES.iter().find(|(signature, _)| head.starts_with(signature)) {
        return Format::Picture(what);
    }
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        return Format::Picture("a WebP image");
    }
    if is_a_bmp(head) {
        return Format::Picture("a BMP image");
    }
    if head.get(4..8) == Some(b"ftyp") {
        let brand = head.get(8..12);
        return match PICTURE_BRANDS.iter().find(|(known, _)| brand == Some(known.as_slice())) {
            Some((_, what)) => Format::Picture(what),
            None => Format::Not("a video"),
        };
    }
    match NOT_DOCUMENTS.iter().find(|(signature, _)| head.starts_with(signature)) {
        Some((_, what)) => Format::Not(what),
        None => Format::Text,
    }
}

/// Whether a file is a Windows bitmap: `BM`, and the size of one of the
/// headers bitmaps have where it stands. Two letters alone begin notes too.
fn is_a_bmp(head: &[u8]) -> bool {
    let header = head.get(14..18).map(|size| u32::from_le_bytes([size[0], size[1], size[2], size[3]]));
    head.starts_with(b"BM") && matches!(header, Some(12 | 40 | 52 | 56 | 64 | 108 | 124))
}

/// The document a zip archive holds, by what it says it is: a Word
/// document's content types name its main part, a book's `mimetype` its
/// kind.
fn container_of(bytes: &[u8]) -> Option<DocKind> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    if crate::epub::is_epub(&mut zip) {
        return Some(DocKind::Epub);
    }
    let mut types = String::new();
    let mut entry = zip.by_name("[Content_Types].xml").ok()?;
    if entry.size() > 1024 * 1024 {
        return None;
    }
    entry.read_to_string(&mut types).ok()?;
    // A document, a template and a document with macros: one main part each.
    [
        "wordprocessingml.document.main",
        "wordprocessingml.template.main",
        "ms-word.document.macroEnabled.main",
    ]
    .iter()
    .any(|main| types.contains(main))
    .then_some(DocKind::Docx)
}

/// Whether a compound file is an Office document under a password: its
/// directory names the encrypted package, in UTF-16 as every name there is.
fn holds_an_encrypted_package(bytes: &[u8]) -> bool {
    let name: Vec<u8> = "EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes).collect();
    bytes.windows(name.len()).any(|window| window == name)
}

/// A panic in a reader, as a reason the file was not read. Documents have a
/// thousand producers, and one malformed file in a folder of thousands must
/// not cost the other ones.
pub(crate) fn guarded<T>(what: &str, read: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(read)) {
        Ok(result) => result,
        Err(panic) => Err(format!("{what} failed on it: {}", panic_message(&*panic))),
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_offered_by_a_document_s_extension_or_none() {
        for offered_path in ["a/b.MD", "a/b.txt", "a/b.Pdf", "a/b.DOCX", "a/b.epub", "a/README", "a/invoice"] {
            assert!(offered(Path::new(offered_path)), "{offered_path}");
        }
        for passed in ["a/b.doc", "a/b.json", "a/b.pdf.part", "a/b.mp4"] {
            assert!(!offered(Path::new(passed)), "{passed}");
        }
        // A picture is offered where its text can be read, and only there.
        for picture in ["a/shot.PNG", "a/scan.tiff", "a/photo.jpeg", "a/photo.heic"] {
            assert_eq!(offered(Path::new(picture)), crate::ocr::available(), "{picture}");
        }
        assert!(!claims_a_document(Path::new("a/README")), "no extension claims nothing");
    }

    #[test]
    fn the_first_bytes_say_what_a_file_is() {
        assert_eq!(format_of(b"%PDF-1.7\n"), Format::Pdf);
        assert_eq!(format_of(b"PK\x03\x04\x14\x00"), Format::Zip);
        assert_eq!(format_of(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1\x00"), Format::Ole);
        assert_eq!(format_of(b"\x89PNG\r\n\x1a\n"), Format::Picture("a PNG image"));
        assert_eq!(format_of(b"\xFF\xD8\xFF\xE0\x00\x10JFIF"), Format::Picture("a JPEG image"));
        assert_eq!(format_of(b"II*\x00\x08\x00\x00\x00"), Format::Picture("a TIFF image"));
        assert_eq!(format_of(b"RIFF\x24\x00\x00\x00WEBPVP8 "), Format::Picture("a WebP image"));
        assert_eq!(format_of(b"RIFF\x24\x00\x00\x00WAVEfmt "), Format::Not("a sound or a video"));
        assert_eq!(format_of(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00"), Format::Picture("a HEIC image"));
        assert_eq!(format_of(b"\x00\x00\x00\x20ftypisom"), Format::Not("a video"));
        let mut bmp = b"BM\x36\x00\x0c\x00\x00\x00\x00\x00\x36\x00\x00\x00".to_vec();
        bmp.extend_from_slice(&40u32.to_le_bytes());
        assert_eq!(format_of(&bmp), Format::Picture("a BMP image"));
        // Two letters begin a note as well as a bitmap.
        assert_eq!(format_of(b"BMW service notes: the brakes were checked in May."), Format::Text);
        assert_eq!(format_of("# Заметка".as_bytes()), Format::Text);
        // A note about the PDF format is a note.
        assert_eq!(format_of(b"The header is %PDF-1.7, then objects."), Format::Text);
    }

    #[test]
    fn a_reader_is_recorded_when_one_read_it() {
        assert!(reader(DocKind::Text, false).is_empty());
        assert_eq!(reader(DocKind::Markdown, true), encoding::detector());
        assert!(reader(DocKind::Pdf, false).starts_with("pdf_oxide "));
        assert!(reads_the_same(DocKind::Text, ""));
        assert!(reads_the_same(DocKind::Text, &encoding::detector()));
        assert!(!reads_the_same(DocKind::Text, "chardetng 0"), "another detector may have guessed otherwise");
        assert!(!reads_the_same(DocKind::Pdf, "pdf_oxide 0.1.0"));
        assert!(
            !reads_the_same(DocKind::Epub, ""),
            "a book recorded with no reader was read by none of these rules"
        );
    }

    #[test]
    fn a_text_sample_does_not_cut_a_character() {
        let text = "ж".repeat(10);
        assert_eq!(text_sample(text.as_bytes(), 20), text.as_bytes(), "the whole file is its own sample");
        let sample = text_sample(&text.as_bytes()[..9], 20);
        assert!(std::str::from_utf8(sample).is_ok() && sample.len().is_multiple_of(2));
    }

    #[test]
    fn a_panic_in_a_reader_is_a_file_not_read() {
        let result: Result<(), String> = guarded("the PDF reader", || panic!("index out of bounds"));
        assert_eq!(result, Err("the PDF reader failed on it: index out of bounds".to_string()));
        let result: Result<(), String> = guarded("the Word reader", || panic!("{} pages", 3));
        assert_eq!(result, Err("the Word reader failed on it: 3 pages".to_string()));
    }
}
