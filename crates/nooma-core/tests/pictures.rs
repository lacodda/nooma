//! Pictures and scans: a screenshot found by its words, a photo of a cat
//! passed over, a PDF page that is a picture of text read with the others.
//!
//! The pictures are lacodda-ocr's (`crates/lacodda-ocr/tests/images`), made
//! by its `make_images.py`; `tests/pdf/half-scanned.pdf` is a typed page
//! followed by one of them. On Windows they need the recognizers for
//! English and Russian installed, and do not skip themselves without them.

use std::path::{Path, PathBuf};

use nooma_core::document::DocKind;
use nooma_core::reading::{self, Unread};
use nooma_core::{Library, ocr};

fn picture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lacodda-ocr/tests/images").join(name)
}

fn pdf(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdf").join(name)
}

/// A folder with a screenshot, a photo of a page, a picture with nothing to
/// read, a typed PDF with a scanned page, and a note.
fn folder() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("notes");
    std::fs::create_dir_all(source.join("shots")).unwrap();
    std::fs::copy(picture("screen.png"), source.join("shots/backup settings.png")).unwrap();
    std::fs::copy(picture("letter-ru.png"), source.join("repair.png")).unwrap();
    std::fs::copy(picture("no-text.jpg"), source.join("sunset.jpg")).unwrap();
    std::fs::copy(pdf("half-scanned.pdf"), source.join("handbook and letter.pdf")).unwrap();
    std::fs::write(source.join("plan.md"), "# Plan\n\nCall the service centre about the coffee machine.\n").unwrap();
    let store = dir.path().join("store");
    (dir, source, store)
}

#[cfg(windows)]
mod on_windows {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    #[test]
    fn there_is_ocr_here() {
        let ocr = ocr::ocr().expect("the recognizers are installed");
        for language in ["en-US", "ru"] {
            assert!(ocr.languages().iter().any(|known| known == language), "{language} in {:?}", ocr.languages());
        }
        assert!(reading::offered(Path::new("a/shot.png")));
    }

    #[test]
    fn a_picture_with_text_is_a_document_of_what_it_says() {
        let opened = reading::read_path(&picture("letter-ru.png")).unwrap();
        assert_eq!(opened.kind, DocKind::Image);
        assert_eq!(opened.reader, ocr::reader());
        assert_eq!(opened.document.title, "letter-ru");
        assert!(opened.document.chunks.iter().all(|chunk| chunk.recognized && chunk.page.is_none()));
        let text: Vec<String> = opened.document.chunks.iter().flat_map(|chunk| words(&chunk.text)).collect();
        for word in ["гарантийный", "уплотнителя", "накипи"] {
            assert!(text.iter().any(|w| w == word), "{word} in {text:?}");
        }
        // Cut as text is: its paragraphs are paragraphs of the chunk.
        let first = &opened.document.chunks[0].text;
        assert!(first.split("\n\n").count() >= 4, "{first}");
    }

    #[test]
    fn a_picture_with_no_text_is_no_document_and_says_what_looked() {
        assert_eq!(reading::read_path(&picture("no-text.jpg")), Err(Unread::NoText(ocr::reader())));
    }

    #[test]
    fn a_scanned_page_is_read_among_typed_ones() {
        let opened = reading::read_path(&pdf("half-scanned.pdf")).unwrap();
        assert_eq!(opened.kind, DocKind::Pdf);
        assert_eq!(opened.reader, format!("{}; {}", nooma_core::pdf::reader(), ocr::reader()));
        assert_eq!(opened.unrecognized, None);
        let typed: Vec<_> = opened.document.chunks.iter().filter(|chunk| chunk.page == Some(1)).collect();
        let scanned: Vec<_> = opened.document.chunks.iter().filter(|chunk| chunk.page == Some(2)).collect();
        assert!(
            !typed.is_empty() && typed.iter().all(|chunk| !chunk.recognized),
            "the typed page is read as typed"
        );
        assert!(!scanned.is_empty() && scanned.iter().all(|chunk| chunk.recognized));
        let text: Vec<String> = scanned.iter().flat_map(|chunk| words(&chunk.text)).collect();
        assert!(text.iter().any(|w| w == "кофемашина"), "{text:?}");
    }

    #[test]
    fn a_blank_scan_stays_textless() {
        let opened = reading::read_path(&pdf("scan.pdf")).unwrap();
        assert!(opened.document.chunks.iter().all(|chunk| chunk.text.is_empty() && !chunk.recognized));
    }

    #[test]
    fn a_library_finds_pictures_by_their_words_and_passes_over_the_rest() {
        let (_dir, source, store) = folder();
        let mut library = Library::open_at(&store).unwrap();
        library.add_source(&source, Vec::new()).unwrap();
        let report = library.update().unwrap();
        assert_eq!(report.documents, 4, "the sunset is no document: {report:?}");
        assert!(report.skipped.is_empty(), "a picture with no text is not reported: {:?}", report.skipped);
        assert!(report.textless.is_empty());
        let mut recognized: Vec<String> = report
            .recognized
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        recognized.sort();
        assert_eq!(recognized, ["backup settings.png", "handbook and letter.pdf", "repair.png"]);

        let hits = library.search("backup drive archive", 5).unwrap().hits;
        let first = &hits.first().expect("the screenshot is found").hit;
        assert!(first.path.ends_with("shots/backup settings.png"), "{:?}", first.path);
        assert_eq!(first.kind, "image");
        assert!(first.recognized && first.page.is_none());

        let hits = library.search("уплотнитель паровой трубки", 5).unwrap().hits;
        let paths: Vec<_> = hits
            .iter()
            .map(|hit| (hit.hit.path.file_name().unwrap().to_string_lossy().into_owned(), hit.hit.page))
            .collect();
        assert!(paths.contains(&("repair.png".to_string(), None)), "{paths:?}");
        assert!(paths.contains(&("handbook and letter.pdf".to_string(), Some(2))), "{paths:?}");

        // Nothing changed, nothing is looked at again: not the pictures read,
        // and not the one passed over.
        let again = library.update().unwrap();
        assert_eq!((again.indexed, again.documents), (0, 4), "{again:?}");
    }

    #[test]
    fn a_picture_passed_over_is_looked_at_again_by_another_reader() {
        let (_dir, source, store) = folder();
        let mut library = Library::open_at(&store).unwrap();
        library.add_source(&source, Vec::new()).unwrap();
        library.update().unwrap();
        // As if the sunset had been looked at before a language was added.
        let manifest = store.join("manifest.json");
        let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        let passed = json["passed"].as_object_mut().expect("the sunset is passed over");
        assert_eq!(passed.len(), 1, "{passed:?}");
        for entry in passed.values_mut() {
            assert_eq!(entry["reader"], ocr::reader());
            entry["reader"] = "lacodda-ocr 0.0.1 (en-US)".into();
        }
        std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
        // Looked at again - and passed over again, by the reader of now.
        let again = library.update().unwrap();
        assert_eq!((again.indexed, again.documents), (0, 4), "{again:?}");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        let readers: Vec<_> = json["passed"].as_object().unwrap().values().map(|entry| entry["reader"].clone()).collect();
        assert_eq!(readers, [serde_json::Value::from(ocr::reader())]);
    }
}

#[cfg(not(windows))]
mod elsewhere {
    use super::*;

    #[test]
    fn pictures_are_not_offered_where_there_is_no_ocr() {
        assert!(!ocr::available());
        assert!(!reading::offered(Path::new("a/shot.png")));
        assert!(matches!(reading::read_path(&picture("screen.png")), Err(Unread::NotADocument(what)) if what == "a PNG image"));
        assert_eq!(reading::reader(DocKind::Pdf, false), nooma_core::pdf::reader());
    }

    #[test]
    fn a_scan_is_found_by_its_name_alone() {
        let (_dir, source, store) = folder();
        let mut library = Library::open_at(&store).unwrap();
        library.add_source(&source, Vec::new()).unwrap();
        let report = library.update().unwrap();
        assert_eq!(report.documents, 2, "the note and the PDF: {report:?}");
        assert!(report.recognized.is_empty());
        let opened = reading::read_path(&pdf("half-scanned.pdf")).unwrap();
        assert!(opened.document.chunks.iter().all(|chunk| chunk.page == Some(1) && !chunk.recognized));
    }
}
