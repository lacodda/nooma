//! The vector store, its index and the evaluation, with a stand-in model.
//!
//! The stand-in hashes words into buckets: a bag of words, deterministic and
//! instant, needing nothing on disk. It cannot tell meaning from spelling -
//! which is not what these tests are about. They are about what is stored,
//! under which key, what is computed again and what is not, what the index
//! covers and says it covers, and whether the counting in an evaluation is
//! right. The real model is measured in `meaning.rs`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nooma_core::eval::{EvalQuery, Evaluation, QuerySet};
use nooma_core::{Embedder, Error, Library, Model, Progress, Role};

/// Words hashed into 64 buckets, normalized. Counts every passage it reads.
struct Hashing {
    id: &'static str,
    read: Rc<Cell<usize>>,
    /// Fail on this call, counting from one.
    fail_on_call: Option<usize>,
    calls: usize,
}

impl Hashing {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            read: Rc::new(Cell::new(0)),
            fail_on_call: None,
            calls: 0,
        }
    }
}

impl Model for Hashing {
    fn model_id(&self) -> &str {
        self.id
    }

    fn dimensions(&self) -> usize {
        64
    }
}

impl Embedder for Hashing {
    fn embed(&mut self, texts: &[&str], role: Role) -> nooma_core::Result<Vec<Vec<f32>>> {
        self.calls += 1;
        if self.fail_on_call == Some(self.calls) {
            return Err(Error::Embedding("the stand-in was told to fail".into()));
        }
        if role == Role::Passage {
            self.read.set(self.read.get() + texts.len());
        }
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0f32; 64];
                for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
                    let bucket = blake3::hash(word.to_lowercase().as_bytes()).as_bytes()[0] as usize % 64;
                    vector[bucket] += 1.0;
                }
                nooma_core::embed::normalize(&mut vector);
                vector
            })
            .collect())
    }
}

fn quiet() -> impl Fn(Progress) + Sync {
    |_| {}
}

struct Fixture {
    _dir: tempfile::TempDir,
    notes: PathBuf,
    store: PathBuf,
}

impl Fixture {
    /// A folder of notes, each on its own subject.
    fn new(notes: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("notes");
        for (name, text) in notes {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let store = dir.path().join("store");
        Self { notes: root, store, _dir: dir }
    }

    fn library(&self) -> Library {
        let mut library = Library::open_at(&self.store).unwrap();
        if library.sources().is_empty() {
            library.add_source(&self.notes, Vec::new()).unwrap();
        }
        library.update().unwrap();
        library
    }
}

fn notes() -> Vec<(&'static str, &'static str)> {
    vec![
        ("kettle.md", "# Kettle\n\nDescale the kettle with citric acid once a month.\n"),
        ("bicycle.md", "# Bicycle\n\nThe chain needs oil after every rainy ride.\n"),
        ("garden/tomatoes.md", "# Tomatoes\n\nWater the tomatoes in the morning, never at noon.\n"),
        ("чай.md", "# Чай\n\nЗелёный чай заваривают водой около восьмидесяти градусов.\n"),
    ]
}

/// Rewrite a file so its size or time changes for certain.
fn rewrite(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
}

#[test]
fn every_passage_is_read_once_and_kept() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");

    let first = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(first.chunks, 4);
    assert_eq!(first.embedded, 4);
    assert_eq!(model.read.get(), 4);

    let second = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(second.embedded, 0, "nothing changed, so nothing is read");
    assert_eq!(model.read.get(), 4);
}

/// A vector belongs to the text, not to the file: a renamed note is the same
/// passage, and computing it again would be paying twice for one answer.
#[test]
fn a_renamed_note_keeps_its_vectors() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();

    std::fs::rename(fixture.notes.join("kettle.md"), fixture.notes.join("kitchen-kettle.md")).unwrap();
    library.update().unwrap();
    let after = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(after.embedded, 0);
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert_eq!(index.len(), 4);
    assert_eq!(index.behind(), 0);
    let query = model.embed(&["descale kettle"], Role::Query).unwrap().pop().unwrap();
    let hits = index.search(&query, 10).unwrap();
    assert!(hits[0].path.ends_with("kitchen-kettle.md"), "the renamed note is found under its new name");
    assert!(hits.iter().all(|hit| !hit.path.ends_with("kettle.md")), "and never under its old one");
}

#[test]
fn an_edited_note_is_read_again_and_no_other() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();

    rewrite(
        &fixture.notes.join("bicycle.md"),
        "# Bicycle\n\nThe chain wants oil after a wet ride, and the brakes a look.\n",
    );
    library.update().unwrap();
    let after = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(after.embedded, 1);
}

/// A search over part of the library is still worth answering - the vectors
/// of a large library take an hour - as long as it says what it does not
/// cover yet.
#[test]
fn an_index_says_how_many_documents_it_does_not_cover() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    assert!(library.semantic(&Hashing::new("hash")).unwrap().is_none(), "no vectors, no index");

    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    std::fs::write(fixture.notes.join("new.md"), "# New\n\nA note written after the vectors.\n").unwrap();
    rewrite(&fixture.notes.join("bicycle.md"), "# Bicycle\n\nThe brakes squeal in the rain.\n");
    library.update().unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert_eq!(index.behind(), 2, "one new note and one changed");

    let query = model.embed(&["note written after"], Role::Query).unwrap().pop().unwrap();
    assert!(index.search(&query, 10).unwrap().iter().all(|hit| !hit.path.ends_with("new.md")));
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert_eq!(index.behind(), 0);
    assert!(index.search(&query, 1).unwrap()[0].path.ends_with("new.md"));
}

/// A deleted file can be neither opened nor shown, so it is never a result -
/// not even before the vectors are brought up to date.
#[test]
fn a_removed_document_is_not_found_before_the_vectors_catch_up() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();

    std::fs::remove_file(fixture.notes.join("bicycle.md")).unwrap();
    library.update().unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    let query = model.embed(&["chain oil rainy ride"], Role::Query).unwrap().pop().unwrap();
    let hits = index.search(&query, 10).unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|hit| !hit.path.ends_with("bicycle.md")));
    assert_eq!(index.behind(), 0, "a removed document is not one the index is behind on");
}

/// Reading the index again costs a search; writing it again costs every
/// window that keeps one open a reload. An update that changed nothing
/// writes nothing.
#[test]
fn an_update_that_changes_nothing_leaves_the_index_file_alone() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert!(index.is_current());

    library.update_vectors(&mut model, &quiet()).unwrap();
    assert!(index.is_current(), "nothing changed, so the file was not written");

    rewrite(&fixture.notes.join("kettle.md"), "# Kettle\n\nDescale it with vinegar instead.\n");
    library.update().unwrap();
    library.update_vectors(&mut model, &quiet()).unwrap();
    assert!(!index.is_current(), "a kept index learns that it is out of date");
}

/// Moving a file changes no passage, and so nothing for the model to read -
/// but it changes where every chunk is, and the index must say the new
/// place.
#[test]
fn an_index_follows_a_moved_chunk_without_reading_it_again() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    std::fs::create_dir_all(fixture.notes.join("bikes")).unwrap();
    std::fs::rename(fixture.notes.join("bicycle.md"), fixture.notes.join("bikes").join("bicycle.md")).unwrap();
    library.update().unwrap();
    let report = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(report.embedded, 0);
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    let query = model.embed(&["chain oil rainy ride"], Role::Query).unwrap().pop().unwrap();
    let hit = &index.search(&query, 1).unwrap()[0];
    assert_eq!(hit.path, fixture.notes.join("bikes").join("bicycle.md"));
    assert_eq!(index.behind(), 0);
}

#[test]
fn the_closest_document_comes_first_once_per_document() {
    let fixture = Fixture::new(&[
        ("kettle.md", "# Kettle\n\nDescale the kettle.\n\n## Again\n\nDescale the kettle with acid.\n"),
        ("bicycle.md", "# Bicycle\n\nOil the chain.\n"),
    ]);
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();

    let query = model.embed(&["descale kettle"], Role::Query).unwrap().pop().unwrap();
    let hits = index.search(&query, 10).unwrap();
    assert_eq!(hits.len(), 2, "two chunks of the kettle note are one result");
    assert!(hits[0].path.ends_with("kettle.md"));
    assert!(hits[0].score > hits[1].score);
    assert!(hits[0].score <= 1.0 + 1e-5, "a cosine of unit vectors");
}

/// Each model keeps its own store, so two can be measured over the same
/// library; removing one leaves the other as it was.
#[test]
fn two_models_keep_their_vectors_side_by_side() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut first = Hashing::new("first");
    let mut second = Hashing::new("second");
    library.update_vectors(&mut first, &quiet()).unwrap();
    library.update_vectors(&mut second, &quiet()).unwrap();
    assert!(fixture.store.join("vectors/first/vectors.bin").exists());
    assert!(fixture.store.join("vectors/second/vectors.bin").exists());

    assert!(library.remove_vectors("first").unwrap());
    assert!(library.semantic(&Hashing::new("second")).unwrap().is_some());
    assert!(library.semantic(&Hashing::new("first")).unwrap().is_none());
    assert!(!library.remove_vectors("first").unwrap());
}

/// Computing vectors is the expensive step, and it is written down as it
/// goes: an update that fails half way keeps the half it paid for.
#[test]
fn an_update_that_fails_keeps_what_it_computed() {
    let many: Vec<(String, String)> = (0..100)
        .map(|i| (format!("note-{i:03}.md"), format!("# Note {i}\n\nThis is note number {i}.\n")))
        .collect();
    let borrowed: Vec<(&str, &str)> = many.iter().map(|(name, text)| (name.as_str(), text.as_str())).collect();
    let fixture = Fixture::new(&borrowed);
    let library = fixture.library();

    let mut failing = Hashing::new("hash");
    failing.fail_on_call = Some(2);
    assert!(library.update_vectors(&mut failing, &quiet()).is_err());
    let kept = failing.read.get();
    assert!(kept > 0 && kept < 100, "the first batch was read: {kept}");

    // What it computed is searchable already, and the index says how much
    // it does not cover.
    let partial = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert_eq!(partial.len(), kept);
    assert_eq!(partial.behind(), 100 - kept);

    let mut model = Hashing::new("hash");
    let resumed = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(resumed.embedded, 100 - kept, "only the rest is read");
    let whole = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    assert_eq!((whole.len(), whole.behind()), (100, 0));
}

/// The index is derived from the store and can be thrown away: the next
/// update builds it again from the vectors already computed, reading
/// nothing with the model.
#[test]
fn a_lost_or_damaged_index_is_built_again_from_the_store() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index_file = fixture.store.join("vectors").join("hash").join("index.bin");

    std::fs::write(&index_file, b"not an index").unwrap();
    assert!(library.semantic(&Hashing::new("hash")).unwrap().is_none(), "a damaged index is not searched");
    let report = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(report.embedded, 0);
    assert_eq!(library.semantic(&Hashing::new("hash")).unwrap().unwrap().len(), 4);

    std::fs::remove_file(&index_file).unwrap();
    library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(model.read.get(), 4, "the model read every passage once, in the first update");
    assert_eq!(library.semantic(&Hashing::new("hash")).unwrap().unwrap().len(), 4);
}

/// Hits read from the full-text index: the chunk's own title and text, the
/// score the cosine, the example's own file left out.
#[test]
fn hits_by_meaning_carry_the_chunk_and_can_leave_one_file_out() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    let finder = library.finder().unwrap().unwrap();

    let vector = model.embed(&["Descale the kettle with citric acid"], Role::Passage).unwrap().pop().unwrap();
    let hits = index.hits(&finder, &vector, 2, None).unwrap();
    assert_eq!(hits[0].title, "Kettle");
    assert!(hits[0].fragment.contains("citric acid"));
    assert!(hits[0].highlights.is_empty());
    assert!(hits[0].score > 0.5 && hits[0].score <= 1.0 + 1e-5);

    let kettle = hits[0].path.clone();
    let others = index.hits(&finder, &vector, 2, Some(&kettle)).unwrap();
    assert_eq!(others.len(), 2, "the limit still holds without the left-out file");
    assert!(others.iter().all(|hit| hit.path != kettle));
}

fn query(text: &str, expect: &str, group: &str) -> EvalQuery {
    EvalQuery {
        query: text.to_string(),
        expect: vec![expect.to_string()],
        group: Some(group.to_string()),
    }
}

#[test]
fn an_evaluation_counts_where_each_answer_landed() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let set = QuerySet {
        queries: vec![
            query("descale kettle", "kettle.md", "en"),
            query("tomatoes water", "garden/tomatoes.md", "en"),
            query("зелёный чай", "чай.md", "ru"),
            query("something nobody wrote", "bicycle.md", "en"),
        ],
    };
    let evaluation = Evaluation::new(&library, set).unwrap();

    let fulltext = evaluation.fulltext().unwrap();
    assert_eq!(fulltext.overall.queries, 4);
    assert_eq!(fulltext.outcomes[0].rank, Some(1));
    assert_eq!(
        fulltext.outcomes[1].top.first().map(String::as_str),
        Some("garden/tomatoes.md"),
        "named inside the source, with /"
    );
    assert_eq!(fulltext.outcomes[3].rank, None);
    assert!((fulltext.overall.hit_at_1 - 0.75).abs() < 1e-9);
    assert_eq!(fulltext.groups["ru"].queries, 1);
    assert_eq!(fulltext.groups["en"].queries, 3);

    let mut model = Hashing::new("hash");
    assert!(matches!(evaluation.semantic(&mut model), Err(Error::VectorsBehind { documents: 4, .. })));
    library.update_vectors(&mut model, &quiet()).unwrap();
    let semantic = evaluation.semantic(&mut model).unwrap();
    assert_eq!(semantic.engine, "hash");
    assert_eq!(semantic.outcomes[0].rank, Some(1));
}

/// A misspelt path in a query set would count as a miss for every engine,
/// and read as all of them being worse than they are.
#[test]
fn a_query_set_naming_a_document_the_library_lacks_is_refused() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    let set = QuerySet {
        queries: vec![query("kettle", "kettel.md", "en")],
    };
    let error = Evaluation::new(&library, set).unwrap_err();
    assert!(error.to_string().contains("kettel.md"), "{error}");
}

/// A hit by meaning points at the chunk that matched, not the top of its
/// file: its line, its headings, its own opening for a fragment.
#[test]
fn a_hit_by_meaning_is_the_chunk_that_matched() {
    let fixture = Fixture::new(&[(
        "kitchen.md",
        "# Kitchen\n\n## Espresso\n\nDial the grinder in at eighteen grams.\n\n## Kettle\n\nDescale the kettle with citric acid.\n",
    )]);
    let library = fixture.library();
    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    let index = library.semantic(&Hashing::new("hash")).unwrap().unwrap();
    let finder = library.finder().unwrap().unwrap();

    let vector = model.embed(&["descale kettle citric acid"], Role::Query).unwrap().pop().unwrap();
    let hits = index.hits(&finder, &vector, 1, None).unwrap();
    assert_eq!(hits[0].headings, vec!["Kitchen".to_string(), "Kettle".to_string()]);
    assert_eq!(hits[0].line, 9, "the line its text starts on");
    assert!(hits[0].fragment.starts_with("Descale"), "{}", hits[0].fragment);
}
