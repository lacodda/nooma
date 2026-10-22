//! PDF in the library: read page by page, found on the page that answers.
//!
//! The PDFs in `tests/pdf` are printed from `tests/pdf/source/handbook.html`
//! by Chrome - a real font program and a real text layer, as most PDFs have -
//! with an outline, a title and encryption added by pypdf
//! (`source/make_fixtures.py`). Every word in them is synthetic.

use std::path::{Path, PathBuf};

use nooma_core::Library;

struct Fixture {
    _dir: tempfile::TempDir,
    folder: PathBuf,
    store: PathBuf,
}

impl Fixture {
    /// A folder with these PDFs from `tests/pdf` and a markdown note beside
    /// them.
    fn with(pdfs: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("documents");
        std::fs::create_dir_all(&folder).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdf");
        for name in pdfs {
            std::fs::copy(fixtures.join(name), folder.join(name)).unwrap();
        }
        std::fs::write(folder.join("note.md"), "# Note\n\nThe kettle needs descaling.\n").unwrap();
        let store = dir.path().join("store");
        Self { _dir: dir, folder, store }
    }

    fn library(&self) -> Library {
        let mut library = Library::open_at(&self.store).unwrap();
        if library.sources().is_empty() {
            library.add_source(&self.folder, Vec::new()).unwrap();
        }
        library
    }
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

#[test]
fn a_pdf_is_found_on_the_page_that_answers() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    let library = fixture.library();
    let report = library.update().unwrap();
    assert_eq!((report.documents, report.indexed), (2, 2));
    assert!(report.skipped.is_empty() && report.textless.is_empty(), "{report:?}");

    let hits = library.search("ежемесячную очистку бойлера", 10).unwrap().hits;
    let hit = &hits[0].hit;
    assert!(hit.path.ends_with("handbook.pdf"));
    assert_eq!(hit.kind, "pdf");
    assert_eq!(hit.page, Some(2));
    assert_eq!(hit.headings, vec!["Гарантия", "Сроки ремонта"], "the bookmarks leading to the page");
    assert_eq!(hit.title, "handbook", "the file name, not the metadata title");

    let hit = &library.search("gasket screwdriver", 10).unwrap().hits[0].hit;
    assert_eq!((hit.page, hit.headings.clone()), (Some(3), vec!["Repairs".to_string()]));
}

#[test]
fn a_russian_word_broken_across_lines_is_found_by_its_forms() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    let library = fixture.library();
    library.update().unwrap();
    // Set in a narrow hyphenated column on the page; stemmed as any word is.
    for query in ["обслуживание", "производителя", "рекомендуют"] {
        let hits = library.search(query, 10).unwrap().hits;
        assert_eq!(hits.first().map(|found| found.hit.page), Some(Some(2)), "{query}");
    }
}

#[test]
fn a_pdf_is_found_by_its_metadata_title() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    let library = fixture.library();
    library.update().unwrap();
    // "Coffee" is in the metadata title alone.
    let hits = library.search("coffee", 10).unwrap().hits;
    assert_eq!(hits.len(), 1);
    assert!(hits[0].hit.path.ends_with("handbook.pdf"));
}

#[test]
fn markdown_has_no_page() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    let library = fixture.library();
    library.update().unwrap();
    let hit = &library.search("kettle", 10).unwrap().hits[0].hit;
    assert_eq!((hit.kind.as_str(), hit.page), ("markdown", None));
    let json = serde_json::to_value(hit).unwrap();
    assert!(json.get("page").is_none(), "no page in the JSON of a note: {json}");
}

#[test]
fn a_locked_pdf_opens_or_says_why_it_cannot() {
    let fixture = Fixture::with(&["owner-locked.pdf", "password.pdf"]);
    let library = fixture.library();
    let report = library.update().unwrap();
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    assert!(report.skipped[0].path.ends_with("password.pdf"));
    assert_eq!(report.skipped[0].reason, "protected by a password");
    let hits = library.search("гарантийного", 10).unwrap().hits;
    assert!(hits[0].hit.path.ends_with("owner-locked.pdf"), "an owner password does not keep a reader out");
}

#[test]
fn a_scan_is_named_and_found_by_its_name() {
    let fixture = Fixture::with(&["scan.pdf"]);
    let library = fixture.library();
    let report = library.update().unwrap();
    assert_eq!(names(&report.textless), ["scan.pdf"]);
    assert_eq!(report.documents, 2, "a scan is in the index");
    let hit = &library.search("lease", 10).unwrap().hits[0].hit;
    assert!(hit.path.ends_with("scan.pdf"), "by the title in its metadata");
    assert_eq!(hit.page, Some(1));

    let again = library.update().unwrap();
    assert!(again.textless.is_empty(), "said when it is read, not on every update");
}

#[test]
fn a_pdf_is_read_again_when_the_reader_changes_and_a_note_is_not() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    let library = fixture.library();
    library.update().unwrap();
    assert_eq!(library.update().unwrap().indexed, 0);

    // What an older nooma, built with another pdf_oxide, would have written.
    let manifest = fixture.store.join("manifest.json");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    for entry in json["files"].as_object_mut().unwrap().values_mut() {
        if entry.get("reader").is_some() {
            entry["reader"] = "pdf_oxide 0.0.1".into();
        }
    }
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();

    let report = library.update().unwrap();
    assert_eq!((report.indexed, report.rebuilt), (1, None), "the PDF alone, without starting over");
    assert_eq!(library.update().unwrap().indexed, 0, "and then it is current");
}

/// A vault embeds a PDF by its whole name, extension and all; the note doing
/// it lends the PDF its title, as a note linking another does.
#[test]
fn a_note_embedding_a_pdf_lends_it_its_title() {
    let fixture = Fixture::with(&["handbook.pdf"]);
    std::fs::write(fixture.folder.join("barista.md"), "# Barista course\n\nThe manual: ![[handbook.pdf]]\n").unwrap();
    // `[[handbook]]` names a note called handbook, which there is none of -
    // not the PDF.
    std::fs::write(fixture.folder.join("other.md"), "# Zeppelin notes\n\nSee [[handbook]].\n").unwrap();
    let library = fixture.library();
    library.update().unwrap();
    let names = |query: &str| -> Vec<String> {
        library
            .search(query, 10)
            .unwrap()
            .hits
            .iter()
            .map(|found| found.hit.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    assert!(names("barista").contains(&"handbook.pdf".to_string()), "{:?}", names("barista"));
    assert!(!names("zeppelin").contains(&"handbook.pdf".to_string()), "{:?}", names("zeppelin"));
}

#[test]
fn a_pdf_may_be_larger_than_a_text_file() {
    let fixture = Fixture::with(&[]);
    // Over the limit for text, well under the one for PDF: the text file is
    // skipped for its size, the PDF is opened - and refused for what it is.
    // The limit is the one of the kind the bytes say: text named `.pdf` is
    // text.
    let big = vec![b'x'; 17 * 1024 * 1024];
    std::fs::write(fixture.folder.join("dump.txt"), &big).unwrap();
    std::fs::write(fixture.folder.join("dump.pdf"), &big).unwrap();
    std::fs::write(fixture.folder.join("book.pdf"), [b"%PDF-1.7\n".as_slice(), &big].concat()).unwrap();
    let report = fixture.library().update().unwrap();
    let reasons: Vec<(String, &str)> = report
        .skipped
        .iter()
        .map(|skipped| (skipped.path.file_name().unwrap().to_string_lossy().into_owned(), skipped.reason.as_str()))
        .collect();
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert!(reasons.contains(&("dump.txt".to_string(), "larger than 16 MB")), "{reasons:?}");
    assert!(reasons.contains(&("dump.pdf".to_string(), "larger than 16 MB")), "{reasons:?}");
    let book = reasons.iter().find(|(name, _)| name == "book.pdf").unwrap();
    assert!(!book.1.starts_with("larger than"), "{book:?}");
}
