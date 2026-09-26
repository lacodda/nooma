//! The vector store and the evaluation, with a stand-in model.
//!
//! The stand-in hashes words into buckets: a bag of words, deterministic and
//! instant, needing nothing on disk. It cannot tell meaning from spelling -
//! which is not what these tests are about. They are about what is stored,
//! under which key, what is computed again and what is not, and whether the
//! counting in an evaluation is right. The real model is measured in
//! `meaning.rs`.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nooma_core::eval::{EvalQuery, Evaluation, QuerySet};
use nooma_core::{Embedder, Error, Library, Progress, Role};

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

impl Embedder for Hashing {
    fn model_id(&self) -> &str {
        self.id
    }

    fn dimensions(&self) -> usize {
        64
    }

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
    let index = library.semantic(&Hashing::new("hash")).unwrap();
    assert_eq!(index.len(), 4, "the renamed note is searchable under its new name");
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

/// A search over part of the library would answer as if it were the whole,
/// and nobody reading the results could tell.
#[test]
fn a_library_with_chunks_the_model_has_not_read_is_not_searched() {
    let fixture = Fixture::new(&notes());
    let library = fixture.library();
    assert!(matches!(library.semantic(&Hashing::new("hash")), Err(Error::VectorsBehind { missing: 4, .. })));

    let mut model = Hashing::new("hash");
    library.update_vectors(&mut model, &quiet()).unwrap();
    std::fs::write(fixture.notes.join("new.md"), "# New\n\nA note written after the vectors.\n").unwrap();
    library.update().unwrap();
    assert!(matches!(library.semantic(&Hashing::new("hash")), Err(Error::VectorsBehind { missing: 1, .. })));
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
    let index = library.semantic(&Hashing::new("hash")).unwrap();

    let query = model.embed(&["descale kettle"], Role::Query).unwrap().pop().unwrap();
    let hits = index.search(&query, 10);
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
    assert!(library.semantic(&Hashing::new("second")).is_ok());
    assert!(matches!(library.semantic(&Hashing::new("first")), Err(Error::VectorsBehind { .. })));
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

    let mut model = Hashing::new("hash");
    let resumed = library.update_vectors(&mut model, &quiet()).unwrap();
    assert_eq!(resumed.embedded, 100 - kept, "only the rest is read");
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
    assert!(matches!(evaluation.semantic(&mut model), Err(Error::VectorsBehind { .. })));
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
