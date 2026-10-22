//! Reading the documents nooma reads besides markdown and PDF: Word
//! documents and books, text in the encodings it is found in, and files
//! whose names say one thing and bytes another.
//!
//! The fixtures in `tests/formats` are written by the programs such files
//! come from - Word itself, pandoc, Python's codecs - and `source/` holds how;
//! a reader checked against files its own author made checks that author's
//! habits. Every word in them is synthetic.

use std::path::{Path, PathBuf};

use nooma_core::Library;
use nooma_core::document::{Chunk, DocKind, Document};
use nooma_core::reading::{self, Opened, Unread};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/formats").join(name)
}

fn read(name: &str) -> Opened {
    reading::read_path(&fixture(name)).unwrap_or_else(|unread| panic!("{name}: {unread:?}"))
}

fn unread(name: &str) -> Unread {
    reading::read_path(&fixture(name)).expect_err(name)
}

/// The text of every chunk, one after another.
fn text(document: &Document) -> String {
    document.chunks.iter().map(|chunk| chunk.text.as_str()).collect::<Vec<_>>().join("\n\n")
}

/// The chunk holding a phrase.
fn chunk_with<'a>(document: &'a Document, phrase: &str) -> &'a Chunk {
    document
        .chunks
        .iter()
        .find(|chunk| chunk.text.contains(phrase))
        .unwrap_or_else(|| panic!("no chunk holds {phrase:?} in {:#?}", document.chunks))
}

fn headings(chunk: &Chunk) -> Vec<&str> {
    chunk.headings.iter().map(String::as_str).collect()
}

#[test]
fn a_word_document_is_read_under_its_headings() {
    for name in ["lease.docx", "numbered.docx"] {
        let opened = read(name);
        assert_eq!(opened.kind, DocKind::Docx);
        assert!(opened.reader.starts_with("office_oxide "));
        let doc = &opened.document;
        assert_eq!(doc.title, name.trim_end_matches(".docx"), "named by its file");
        assert_eq!(doc.aliases, vec!["Office lease, Harbour Street"], "and found by the title in its properties");

        // Word wrote the style ids in the language of its interface - `1`,
        // `2` - and the headings are headings all the same.
        assert_eq!(headings(chunk_with(doc, "ceramic tiles")), vec!["Terms"], "{name}");
        assert_eq!(headings(chunk_with(doc, "1250 euros")), vec!["Terms", "Rent"], "{name}");
        assert_eq!(
            headings(chunk_with(doc, "Loading bay · 48 square metres")),
            vec!["Terms", "Rooms"],
            "a table row is a line"
        );
        assert_eq!(headings(chunk_with(doc, "Запрещено курить на складе")), vec!["Условия аренды"]);
        assert!(doc.chunks.iter().all(|chunk| chunk.page.is_none()));
    }
}

#[test]
fn a_footnote_is_read_where_it_is_pointed_at() {
    let doc = read("lease.docx").document;
    let rent = chunk_with(&doc, "1250 euros");
    assert!(rent.text.contains("Late payment adds a penalty of two percent."), "{rent:#?}");
}

#[test]
fn a_word_document_reads_as_it_stands() {
    for name in ["lease.docx", "numbered.docx"] {
        let all = text(&read(name).document);
        assert!(all.contains("paid quarterly by bank transfer"), "the revision as it stands");
        assert!(!all.contains("monthly"), "the deleted text is not read");
        assert!(!all.contains("Confidential draft"), "the header is not the text");
        assert!(!all.contains("Check the names"), "a comment is not the text");
        assert!(!all.contains("PAGEREF") && !all.contains("TOC \\o"), "field codes are not the text");
        assert_eq!(
            all.matches("Signed copies are kept by the landlord.").count(),
            1,
            "a text box, stored twice, is read once"
        );
        // The contents repeat the headings with their pages; none of it is
        // a chunk's text.
        assert!(
            !all.lines()
                .any(|line| line.ends_with(" 1") && (line.contains("Terms") || line.contains("Rent"))),
            "{name}: the contents were read:\n{all}"
        );
    }
}

#[test]
fn a_word_document_under_a_password_says_so() {
    assert_eq!(unread("locked.docx"), Unread::Failed("protected by a password".to_string()));
}

#[test]
fn an_old_word_document_under_a_new_name_is_said_to_be_one() {
    match unread("old-format.docx") {
        Unread::NotADocument(what) => assert!(what.contains("old binary kind"), "{what}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_book_is_read_in_its_order_under_its_contents() {
    // An EPUB 3, with a navigation document, and an EPUB 2, with an NCX.
    for name in ["book.epub", "book-epub2.epub"] {
        let opened = read(name);
        assert_eq!(opened.kind, DocKind::Epub);
        let doc = &opened.document;
        assert_eq!(doc.title, "The Lighthouse Keeper\u{2019}s Notebook", "{name}: titled by its title");
        assert!(
            doc.aliases.contains(&"Marta Fixture".to_string()),
            "{name}: found by its author: {:?}",
            doc.aliases
        );

        assert_eq!(headings(chunk_with(doc, "lit at four")), vec!["Winter on the cape"], "{name}");
        assert_eq!(
            headings(chunk_with(doc, "twice a minute")),
            vec!["Winter on the cape", "The fog signal"],
            "{name}"
        );
        assert_eq!(
            headings(chunk_with(doc, "Diesel · 400 litres")),
            vec!["Winter on the cape", "Supplies"],
            "{name}"
        );
        assert_eq!(headings(chunk_with(doc, "свежую треску")), vec!["Летние ночи", "Гости"], "{name}");
        assert_eq!(headings(chunk_with(doc, "copper roof")), vec!["Repairs"], "{name}");

        let all = text(doc);
        assert!(all.contains("into the log."), "{name}");
        assert!(!all.contains("log.1"), "{name}: the mark pointing at a note is not a word");
        assert!(all.contains("The log book is kept in the drawer"), "{name}: the note itself is");
        let order: Vec<usize> = ["lit at four", "Летом маяк", "copper roof"]
            .iter()
            .map(|phrase| all.find(phrase).unwrap())
            .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{name}: in reading order");
    }
}

#[test]
fn a_book_without_contents_is_headed_by_its_own_headings() {
    let doc = read("no-contents.epub").document;
    assert_eq!(headings(chunk_with(&doc, "twice a minute")), vec!["Winter on the cape", "The fog signal"]);
    assert_eq!(headings(chunk_with(&doc, "свежую треску")), vec!["Летние ночи", "Гости"]);
}

#[test]
fn a_book_locked_by_drm_says_so_and_obfuscated_fonts_do_not() {
    assert_eq!(unread("locked.epub"), Unread::Failed("protected by DRM".to_string()));
    let doc = read("fonts.epub").document;
    assert!(text(&doc).contains("twice a minute"));
}

#[test]
fn text_is_read_in_its_encoding_and_a_guess_is_said_to_be_one() {
    let russian = "Договор аренды склада на Портовой улице.";
    // file, a phrase it holds, and the encoding it was guessed to be in -
    // `None` when nothing was guessed.
    let cases: [(&str, &str, Option<&[&str]>); 10] = [
        ("windows-1251.txt", russian, Some(&["windows-1251"])),
        ("koi8-r.txt", russian, Some(&["KOI8-R", "KOI8-U"])),
        ("ibm866.txt", russian, Some(&["IBM866"])),
        ("windows-1252.txt", "Größe der Rechnung", Some(&["windows-1252"])),
        ("shift_jis.txt", "倉庫の鍵は管理人が保管している。", Some(&["Shift_JIS"])),
        ("utf-16le-no-mark.txt", "watchman walks the yard", Some(&["UTF-16LE"])),
        ("utf-16be-mark.txt", russian, None),
        ("utf-8-mark.md", russian, None),
        ("utf-8-cut.txt", "Сторож обходит территорию", None),
        ("utf-8-damaged.txt", "watchman walks the yard", Some(&["UTF-8"])),
    ];
    for (name, phrase, guessed) in cases {
        let opened = read(&format!("encodings/{name}"));
        assert!(text(&opened.document).contains(phrase), "{name}: {:?}", text(&opened.document));
        match guessed {
            Some(names) => assert!(opened.guessed.is_some_and(|encoding| names.contains(&encoding)), "{name}: {:?}", opened.guessed),
            None => assert_eq!(opened.guessed, None, "{name}"),
        }
    }
    assert_eq!(read("encodings/utf-8-mark.md").kind, DocKind::Markdown);
    assert_eq!(read("encodings/utf-8-mark.md").document.title, "Склад");
    assert!(matches!(unread("encodings/binary.txt"), Unread::NotADocument(_)));
}

#[test]
fn a_file_is_what_its_bytes_say_whatever_its_name() {
    assert_eq!(read("kinds/invoice").kind, DocKind::Pdf, "a PDF saved with no extension");
    assert_eq!(read("kinds/report.pdf").kind, DocKind::Docx, "a Word document renamed to .pdf");
    assert_eq!(read("kinds/notes").kind, DocKind::Text, "a note with no extension");
    assert_eq!(unread("kinds/picture.pdf"), Unread::NotADocument("a PNG image".to_string()));
    assert_eq!(unread("kinds/photo"), Unread::NotADocument("a PNG image".to_string()));
    assert!(matches!(unread("kinds/archive.epub"), Unread::NotADocument(what) if what.contains("zip archive")));
}

/// A copy of the fixtures as a library's one source.
fn library_over_fixtures() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("files");
    copy_dir(&fixture(""), &source);
    std::fs::remove_dir_all(source.join("source")).unwrap();
    let mut library = Library::open_at(dir.path().join("store")).unwrap();
    library.add_source(&source, Vec::new()).unwrap();
    (dir, library)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn a_library_reports_what_is_not_a_document_only_when_its_name_said_it_was() {
    let (_dir, library) = library_over_fixtures();
    let report = library.update().unwrap();
    let skipped: Vec<String> = report
        .skipped
        .iter()
        .map(|skipped| skipped.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    for name in ["picture.pdf", "archive.epub", "locked.docx", "old-format.docx", "locked.epub", "binary.txt"] {
        assert!(skipped.contains(&name.to_string()), "{name} not in {skipped:?}");
    }
    assert!(
        !skipped.contains(&"photo".to_string()),
        "a picture with no extension claimed nothing: {skipped:?}"
    );
    let guessed: Vec<&str> = report.guessed.iter().map(|guessed| guessed.encoding.as_str()).collect();
    assert!(guessed.contains(&"windows-1251") && guessed.contains(&"UTF-16LE"), "{guessed:?}");

    let found = |query: &str| -> Vec<String> {
        library
            .search(query, 10)
            .unwrap()
            .hits
            .into_iter()
            .map(|found| format!("{} {}", found.hit.kind, found.hit.path.file_name().unwrap().to_string_lossy()))
            .collect()
    };
    assert!(found("ceramic tiles grout").contains(&"docx lease.docx".to_string()));
    assert!(found("fog horn compressor").contains(&"epub book.epub".to_string()));
    assert!(
        found("Marta Fixture").iter().any(|hit| hit.starts_with("epub ")),
        "a book is found by its author"
    );
    assert!(found("сторож обходит территорию").contains(&"text windows-1251.txt".to_string()));
    assert!(found("espresso warranty").contains(&"pdf invoice".to_string()));
}

#[test]
fn what_another_reader_read_is_read_again_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("files");
    std::fs::create_dir(&source).unwrap();
    for name in ["lease.docx", "book.epub", "encodings/windows-1251.txt", "encodings/utf-8-mark.md"] {
        std::fs::copy(fixture(name), source.join(Path::new(name).file_name().unwrap())).unwrap();
    }
    let store = dir.path().join("store");
    let mut library = Library::open_at(&store).unwrap();
    library.add_source(&source, Vec::new()).unwrap();
    library.update().unwrap();
    assert_eq!(library.update().unwrap().indexed, 0);

    // What an older nooma, with other readers, would have written: every
    // file read by something other than plain UTF-8 says so.
    let manifest = store.join("manifest.json");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    for entry in json["files"].as_object_mut().unwrap().values_mut() {
        if entry.get("reader").is_some() {
            entry["reader"] = "an older reader 0".into();
        }
    }
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();

    let report = library.update().unwrap();
    assert_eq!((report.indexed, report.rebuilt), (3, None), "the Word document, the book and the guessed text");
    assert_eq!(library.update().unwrap().indexed, 0, "and then they are current");
}

#[test]
fn a_file_found_to_be_no_document_is_not_opened_again_until_it_changes() {
    let (dir, library) = library_over_fixtures();
    library.update().unwrap();
    let totals = std::sync::Mutex::new(Vec::new());
    library.update_with(&|progress| totals.lock().unwrap().push(progress.total)).unwrap();
    // Only what is reported every time is read every time: the files whose
    // names claim a document they are not.
    let reported = 6;
    let totals = totals.into_inner().unwrap();
    assert_eq!(totals.last(), Some(&reported), "{totals:?}");

    // A picture with no extension that becomes a note is read as one.
    let photo = dir.path().join("files/kinds/photo");
    std::fs::write(&photo, "The photo album is in the bottom drawer of the chart table.").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&photo)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();
    let report = library.update().unwrap();
    assert_eq!(report.indexed, 1);
    assert!(!library.search("photo album drawer", 5).unwrap().hits.is_empty());
}
