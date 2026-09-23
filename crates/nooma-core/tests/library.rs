//! The document library end to end: folders in, search results out.
//!
//! Every test works on a copy of the synthetic corpus in `tests/corpus`, so a
//! test that edits or deletes a file cannot disturb another. None of the text
//! is real: the corpus is written for these tests, in English and in Russian,
//! because a search that only works in one of them is not the product.

use std::path::{Path, PathBuf};

use nooma_core::{Error, Library};

struct Fixture {
    _dir: tempfile::TempDir,
    corpus: PathBuf,
    store: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let corpus = dir.path().join("corpus");
        copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus"), &corpus);
        let store = dir.path().join("store");
        Self { _dir: dir, corpus, store }
    }

    fn library(&self) -> Library {
        Library::open_at(&self.store).unwrap()
    }

    /// A library with the whole corpus as its one source, indexed.
    fn indexed(&self) -> Library {
        let mut library = self.library();
        library.add_source(&self.corpus, Vec::new()).unwrap();
        library.update().unwrap();
        library
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.corpus.join(rel)
    }
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

fn found(library: &Library, query: &str) -> Vec<String> {
    library
        .search(query, 10)
        .unwrap()
        .into_iter()
        .map(|hit| hit.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// Rewrite a file so its size or time changes for certain: the manifest
/// skips a file whose size and modification time both match.
fn rewrite(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}

#[test]
fn the_first_update_reads_every_document() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    library.add_source(&fixture.corpus, Vec::new()).unwrap();
    let report = library.update().unwrap();
    assert_eq!(report.documents, 9);
    assert_eq!(report.indexed, 9);
    assert_eq!(report.rebuilt, None, "a first run is not a rebuild");
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn a_russian_word_finds_its_other_forms() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    // The note says "Договор", "Договоры" and "Гарантийные"; none of these
    // query forms occurs in it letter for letter.
    assert_eq!(found(&library, "договоров").first().map(String::as_str), Some("договоры.md"));
    assert!(found(&library, "гарантии").contains(&"договоры.md".to_string()));
}

#[test]
fn an_english_word_finds_its_other_forms() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    assert_eq!(found(&library, "warranties claimed").first().map(String::as_str), Some("receipts.md"));
}

#[test]
fn yo_is_found_by_ye() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    assert_eq!(found(&library, "елочные"), vec!["встреча.md"]);
}

#[test]
fn a_code_is_found_whole() {
    let fixture = Fixture::new();
    // The parts of the code, apart and out of order: a code is a sequence.
    std::fs::write(
        fixture.path("home/plans.txt"),
        "Plans for 0114 and 2025, inv.
",
    )
    .unwrap();
    let library = fixture.indexed();
    assert_eq!(found(&library, "INV-2025-0114"), vec!["receipts.md"]);
    assert_eq!(found(&library, "E4012"), vec!["release-notes.md"]);
}

#[test]
fn every_word_must_match_while_some_document_has_them_all() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    // "greenhouse" is in two documents, "vents" in one: both words together
    // name only the garden file.
    assert_eq!(found(&library, "greenhouse vents"), vec!["garden.txt"]);
}

#[test]
fn when_no_document_has_every_word_any_word_will_do() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let hits = found(&library, "greenhouse zeppelin");
    assert!(hits.contains(&"garden.txt".to_string()), "{hits:?}");
}

#[test]
fn a_hit_is_one_document_at_its_best_chunk() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let hits = library.search("steam wand leaking", 10).unwrap();
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.title, "Kitchen appliances");
    assert_eq!(hit.headings, vec!["Kitchen appliances", "Espresso setup"]);
    assert_eq!(hit.line, 9, "the chunk starts at its first paragraph, under the heading");
    assert_eq!(hit.kind, "markdown");
    assert!(hit.tags.contains(&"appliances".to_string()));
    // The highlights point at the matching words inside the fragment.
    let marked: Vec<&str> = hit.highlights.iter().map(|r| &hit.fragment[r.clone()]).collect();
    assert!(marked.iter().any(|w| w.eq_ignore_ascii_case("steam")), "{marked:?} in {:?}", hit.fragment);
}

#[test]
fn a_heading_inside_a_code_fence_is_not_a_heading() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let hits = library.search("retry", 10).unwrap();
    assert_eq!(hits[0].headings, vec!["Release notes draft"]);
}

#[test]
fn a_tag_finds_its_note() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    assert_eq!(found(&library, "housing"), vec!["moving-checklist.md"]);
    assert_eq!(found(&library, "юридическое"), vec!["договоры.md"]);
}

/// A note is found by the title of a note that links to it — the vault's
/// own sense of what it is about.
#[test]
fn a_note_is_found_by_what_links_to_it() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    // "Appliance receipts" links to the checklist; the checklist itself never
    // says "receipts".
    let hits = found(&library, "receipts");
    assert!(hits.contains(&"moving-checklist.md".to_string()), "{hits:?}");
    assert_eq!(hits.first().map(String::as_str), Some("receipts.md"), "the note's own words still win");
}

#[test]
fn nothing_changed_means_nothing_read() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let report = library.update().unwrap();
    assert_eq!((report.indexed, report.removed, report.documents), (0, 0, 9));
}

#[test]
fn an_edited_file_is_read_again_and_found_by_its_new_words() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    rewrite(&fixture.path("home/garden.txt"), "Plant the zucchini in May.\n");
    let report = library.update().unwrap();
    assert_eq!(report.indexed, 1);
    assert_eq!(found(&library, "zucchini"), vec!["garden.txt"]);
    assert!(found(&library, "vents").is_empty(), "the old words must be gone");
}

#[test]
fn a_touched_file_with_the_same_bytes_is_not_reindexed() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let path = fixture.path("home/garden.txt");
    let text = std::fs::read_to_string(&path).unwrap();
    rewrite(&path, &text);
    assert_eq!(library.update().unwrap().indexed, 0);
}

#[test]
fn a_deleted_file_leaves_the_index() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    std::fs::remove_file(fixture.path("home/garden.txt")).unwrap();
    let report = library.update().unwrap();
    assert_eq!((report.removed, report.documents), (1, 8));
    assert!(found(&library, "vents").is_empty());
}

#[test]
fn a_new_link_reindexes_its_target() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    assert!(found(&library, "sunday").contains(&"2025-03-02.md".to_string()));
    assert!(!found(&library, "sunday").contains(&"garden.txt".to_string()));
    // A journal entry starts linking the release notes: they are now found
    // by the entry's title, without having changed themselves.
    rewrite(&fixture.path("journal/2025-03-02.md"), "# Sunday\n\nReread [[release-notes]].\n");
    let report = library.update().unwrap();
    assert_eq!(report.indexed, 2, "the edited note and the note it now links to");
    assert!(found(&library, "sunday").contains(&"release-notes.md".to_string()));
}

#[test]
fn a_removed_source_leaves_the_index_at_the_next_update() {
    let fixture = Fixture::new();
    let mut library = fixture.indexed();
    library.remove_source(&fixture.corpus).unwrap();
    let report = library.update().unwrap();
    assert_eq!((report.removed, report.documents), (9, 0));
    assert!(found(&library, "greenhouse").is_empty());
}

#[test]
fn an_unreachable_source_keeps_its_documents() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let moved = fixture.corpus.with_file_name("unplugged");
    std::fs::rename(&fixture.corpus, &moved).unwrap();
    let report = library.update().unwrap();
    assert_eq!(report.unavailable, vec![fixture.corpus.clone()]);
    assert_eq!((report.removed, report.documents), (0, 9));
    assert_eq!(found(&library, "vents"), vec!["garden.txt"]);
}

#[test]
fn exclusions_hidden_folders_and_ignore_files_are_honoured() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.path(".obsidian")).unwrap();
    std::fs::write(fixture.path(".obsidian/workspace.md"), "hiddenword\n").unwrap();
    std::fs::write(fixture.path("journal/.nooma-ignore"), "2025-03-09.md\n").unwrap();
    let mut library = fixture.library();
    library.add_source(&fixture.corpus, vec!["work/".to_string()]).unwrap();
    let report = library.update().unwrap();
    assert_eq!(report.documents, 5, "9 less three in work/ less one ignored");
    assert!(found(&library, "hiddenword").is_empty());
    assert!(found(&library, "договоров").is_empty());
    assert!(found(&library, "набережной").is_empty());
}

#[test]
fn text_that_is_not_utf8_is_skipped_and_said_so() {
    let fixture = Fixture::new();
    // "привет" in Windows-1251.
    std::fs::write(fixture.path("home/legacy.txt"), [0xEF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]).unwrap();
    let mut library = fixture.library();
    library.add_source(&fixture.corpus, Vec::new()).unwrap();
    let report = library.update().unwrap();
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].path.ends_with("legacy.txt"));
    assert_eq!(report.documents, 9);
}

#[test]
fn overlapping_sources_are_refused() {
    let fixture = Fixture::new();
    let mut library = fixture.library();
    library.add_source(&fixture.path("home"), Vec::new()).unwrap();
    assert!(matches!(library.add_source(&fixture.corpus, Vec::new()), Err(Error::SourceOverlap { .. })));
    assert!(matches!(
        library.add_source(&fixture.path("home"), Vec::new()),
        Err(Error::SourceOverlap { .. })
    ));
    library.add_source(&fixture.path("work"), Vec::new()).unwrap();
    assert_eq!(fixture.library().sources().len(), 2, "sources persist");
}

#[test]
fn a_library_of_another_format_is_refused_then_rebuilt() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let manifest = fixture.store.join("manifest.json");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    json["chunker_version"] = serde_json::json!(0);
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();

    let error = library.search("greenhouse", 10).unwrap_err();
    assert!(error.is_stale_index(), "{error}");
    assert!(library.status().unwrap().stale.is_some());

    let report = library.update().unwrap();
    assert!(report.rebuilt.is_some());
    assert_eq!(report.indexed, 9);
    assert!(!found(&library, "greenhouse").is_empty());
}

#[test]
fn a_second_writer_is_told_the_library_is_busy() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    let index = tantivy::Index::open_in_dir(fixture.store.join("fulltext")).unwrap();
    let _held: tantivy::IndexWriter = index.writer(15_000_000).unwrap();
    assert!(matches!(library.update(), Err(Error::Busy)));
    // Reading is not writing: the stored index still answers.
    assert_eq!(found(&library, "vents"), vec!["garden.txt"]);
}

#[test]
fn an_empty_query_or_library_finds_nothing_without_failing() {
    let fixture = Fixture::new();
    assert!(fixture.library().search("anything", 10).unwrap().is_empty());
    let library = fixture.indexed();
    assert!(library.search("   ", 10).unwrap().is_empty());
    assert!(library.search("greenhouse", 0).unwrap().is_empty());
    // Syntax a person types by accident is not an error.
    assert!(library.search("\"unclosed (", 10).is_ok());
}

/// The target's backlinks field holds the linking note's title, so renaming
/// the linking note has to reach a target whose own bytes never changed.
#[test]
fn a_linking_note_renamed_reaches_its_targets() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    rewrite(&fixture.path("home/receipts.md"), "# Purchase papers\n\nSee [[moving-checklist]].\n");
    library.update().unwrap();
    let hits = found(&library, "papers");
    assert!(hits.contains(&"moving-checklist.md".to_string()), "{hits:?}");
    assert!(!found(&library, "appliance receipts").contains(&"moving-checklist.md".to_string()));
}

/// A second note with the same name makes a bare link ambiguous, and an
/// ambiguous link points nowhere: the first note loses the backlink although
/// neither it nor the linking note changed.
#[test]
fn a_new_namesake_takes_a_backlink_away() {
    let fixture = Fixture::new();
    let library = fixture.indexed();
    assert!(found(&library, "receipts").contains(&"moving-checklist.md".to_string()));
    std::fs::write(fixture.path("work/moving-checklist.md"), "# Office move\n").unwrap();
    library.update().unwrap();
    let hits = library.search("receipts", 10).unwrap();
    assert!(!hits.iter().any(|hit| hit.path.ends_with("home/moving-checklist.md")), "{hits:?}");
}
