//! What the index lacks of the files under its sources, and why: kept between
//! updates, so it can be said at any time - not only by the update that met
//! the file - and a file refused for a reason that lasts is not tried again
//! until it changes.
//!
//! The files are synthetic: the fixtures of `tests/pdf` and `tests/formats`,
//! and text written here.

use std::path::{Path, PathBuf};

use nooma_core::{Library, Skipped, Why};

struct Folder {
    _dir: tempfile::TempDir,
    notes: PathBuf,
    store: PathBuf,
}

impl Folder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        let store = dir.path().join("store");
        Self { _dir: dir, notes, store }
    }

    fn copy(&self, fixture: &str, name: &str) -> PathBuf {
        let to = self.notes.join(name);
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(fixture), &to).unwrap();
        to
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let to = self.notes.join(name);
        std::fs::write(&to, bytes).unwrap();
        to
    }

    fn library(&self) -> Library {
        let mut library = Library::open_at(&self.store).unwrap();
        if library.sources().is_empty() {
            library.add_source(&self.notes, Vec::new()).unwrap();
        }
        library
    }
}

fn whys(skipped: &[Skipped]) -> Vec<(String, Why)> {
    skipped
        .iter()
        .map(|skipped| (skipped.path.file_name().unwrap().to_string_lossy().into_owned(), skipped.why))
        .collect()
}

#[test]
fn what_could_not_be_read_is_kept_with_why() {
    let folder = Folder::new();
    folder.copy("pdf/password.pdf", "tax-return.pdf");
    folder.copy("formats/locked.epub", "novel.epub");
    folder.copy("formats/kinds/archive.epub", "archive.epub");
    let huge = folder.write("huge.txt", &vec![b'a'; 17 * 1024 * 1024]);
    folder.write("damaged.pdf", b"%PDF-1.7\nnot really a PDF");
    // A file with no extension that is no document claims nothing, and is
    // nothing to look at.
    folder.write("program", b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00");
    let library = folder.library();

    let report = library.update().unwrap();
    let expected = vec![
        ("archive.epub".to_string(), Why::NotADocument),
        ("huge.txt".to_string(), Why::TooLarge),
        ("novel.epub".to_string(), Why::Locked),
        ("tax-return.pdf".to_string(), Why::Locked),
        ("damaged.pdf".to_string(), Why::Damaged),
    ];
    let mut reported = whys(&report.skipped);
    reported.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    assert_eq!(reported, expected);
    assert_eq!(report.documents, 1);

    let health = library.health().unwrap();
    assert_eq!(whys(&health.skipped), expected, "sorted by why, then by path");
    let too_large = health.skipped.iter().find(|skipped| skipped.why == Why::TooLarge).unwrap();
    assert_eq!(too_large.size, Some(17 * 1024 * 1024));
    assert_eq!(too_large.reason, "larger than 16 MB");
    assert_eq!(health.count(), 5);

    // Unchanged, they are not tried again - and the index still says so.
    let again = library.update().unwrap();
    assert!(again.skipped.is_empty(), "{:?}", again.skipped);
    assert_eq!(whys(&library.health().unwrap().skipped), expected);

    // Once a file changes, it is tried again; once it is gone, so is the
    // word about it.
    folder.write("tax-return.pdf", b"Tax return: filed in April.\n");
    std::fs::remove_file(&huge).unwrap();
    let changed = library.update().unwrap();
    assert_eq!(changed.indexed, 1);
    assert!(changed.skipped.is_empty());
    let health = library.health().unwrap();
    assert_eq!(
        whys(&health.skipped),
        vec![
            ("archive.epub".to_string(), Why::NotADocument),
            ("novel.epub".to_string(), Why::Locked),
            ("damaged.pdf".to_string(), Why::Damaged),
        ]
    );
    let found = library.search("filed april", 5).unwrap();
    assert_eq!(found.hits.len(), 1);
}

#[test]
fn a_source_dropped_takes_its_health_with_it() {
    let folder = Folder::new();
    folder.copy("pdf/password.pdf", "tax-return.pdf");
    let mut library = folder.library();
    library.update().unwrap();
    assert_eq!(library.health().unwrap().skipped.len(), 1);
    library.remove_source(&folder.notes).unwrap();
    assert!(library.health().unwrap().skipped.is_empty(), "a folder no longer searched says nothing");
}

#[test]
fn what_was_read_in_part_is_kept_too() {
    let folder = Folder::new();
    folder.copy("formats/encodings/windows-1251.txt", "сторож.txt");
    folder.copy("pdf/scan.pdf", "scan.pdf");
    let library = folder.library();
    library.update().unwrap();
    // A second update reads nothing; what the first learnt stays.
    assert_eq!(library.update().unwrap().indexed, 0);

    let health = library.health().unwrap();
    let guessed: Vec<(String, &str)> = health
        .guessed
        .iter()
        .map(|guessed| (guessed.path.file_name().unwrap().to_string_lossy().into_owned(), guessed.encoding.as_str()))
        .collect();
    assert_eq!(guessed, vec![("сторож.txt".to_string(), "windows-1251")]);
    // The scan's pages are pictures with no text OCR finds in them.
    assert_eq!(health.textless, vec![folder.notes.join("scan.pdf")]);
    assert_eq!(
        health.no_ocr.is_some(),
        !nooma_core::ocr::available(),
        "a scan found by name says why its text is not read, where it is not"
    );
    assert!(health.skipped.is_empty() && health.unavailable.is_empty());
}

#[test]
fn a_folder_out_of_reach_is_said_to_be() {
    let folder = Folder::new();
    let library = folder.library();
    library.update().unwrap();
    assert!(library.health().unwrap().unavailable.is_empty());
    let moved = folder.notes.with_file_name("notes-unplugged");
    std::fs::rename(&folder.notes, &moved).unwrap();
    assert_eq!(library.health().unwrap().unavailable, vec![folder.notes.clone()]);
}

/// A file that could not be opened may open next time - a program let it
/// go - so it is tried at every update, changed or not.
#[test]
fn a_file_that_could_not_be_opened_is_tried_again() {
    let folder = Folder::new();
    let held = folder.write("held.md", b"# Held\n\nThe greenhouse vents open at noon.\n");
    let library = folder.library();
    let guard = lock(&held);

    let report = library.update().unwrap();
    assert_eq!(whys(&report.skipped), vec![("held.md".to_string(), Why::Unopened)]);
    let again = library.update().unwrap();
    assert_eq!(whys(&again.skipped), vec![("held.md".to_string(), Why::Unopened)], "not tried again");

    drop(guard);
    let released = library.update().unwrap();
    assert_eq!(released.indexed, 1);
    assert!(library.health().unwrap().skipped.is_empty());
    assert_eq!(library.search("greenhouse", 5).unwrap().hits.len(), 1);
}

/// Hold a file so that nooma cannot open it, until the guard is dropped.
#[cfg(windows)]
fn lock(path: &Path) -> impl Drop {
    use std::os::windows::fs::OpenOptionsExt;
    struct Held(#[allow(dead_code)] std::fs::File);
    impl Drop for Held {
        fn drop(&mut self) {}
    }
    // Shared with nobody, as a program writing a file holds it.
    Held(std::fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap())
}

#[cfg(unix)]
fn lock(path: &Path) -> impl Drop {
    use std::os::unix::fs::PermissionsExt;
    struct Held(PathBuf);
    impl Drop for Held {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
    Held(path.to_path_buf())
}
