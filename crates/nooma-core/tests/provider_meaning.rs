//! The provider's half by meaning, with a stand-in model.
//!
//! The stand-in hashes words into buckets, as the one in `vectors.rs` does:
//! instant, and blind to meaning - which is not what is tested here. What is
//! tested is that a folder given a model answers by both halves, says how the
//! half by meaning stands, and lends the model to a search while an update
//! holds it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nooma_core::provider::Step;
use nooma_core::{Embedder, Folder, FullText, LanguageAxes, Model, Role};

/// Words hashed into 64 buckets, normalized; counts the passages it reads.
struct Hashing {
    passages: Arc<AtomicUsize>,
}

impl Model for Hashing {
    fn model_id(&self) -> &str {
        "hashing"
    }

    fn dimensions(&self) -> usize {
        64
    }

    fn language(&self) -> Option<&LanguageAxes> {
        None
    }
}

impl Embedder for Hashing {
    fn embed(&mut self, texts: &[&str], role: Role) -> nooma_core::Result<Vec<Vec<f32>>> {
        if role == Role::Passage {
            self.passages.fetch_add(texts.len(), Ordering::AcqRel);
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

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "kettle.md",
        "# Kettle\n\nThe handbook came in the box. Limescale leaves with citric acid. The lid is washed by hand.\n",
    );
    write(dir.path(), "lease.md", "# Lease\n\nThe warehouse lease is renewed in March.\n");
    dir
}

fn folder(notes: &Path, store: &Path) -> (Folder, Arc<AtomicUsize>) {
    let passages = Arc::new(AtomicUsize::new(0));
    let model = Hashing { passages: passages.clone() };
    (Folder::open(notes, store, Vec::new()).unwrap().with_embedder(Box::new(model)), passages)
}

#[test]
fn a_folder_given_a_model_answers_by_both_halves() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let (folder, passages) = folder(notes.path(), store.path());

    // The model is here and has read nothing yet.
    let before = folder.standing().unwrap().meaning.unwrap();
    assert_eq!((before.model.as_str(), before.ready), ("hashing", false));

    let seen = std::sync::Mutex::new(Vec::new());
    let updated = folder.update_with(&|step| seen.lock().unwrap().push(step)).unwrap();
    assert_eq!(updated.embedded, 2);
    assert!(
        seen.into_inner()
            .unwrap()
            .iter()
            .any(|step| matches!(step, Step::Passages(at) if at.total == 2))
    );

    let after = folder.standing().unwrap().meaning.unwrap();
    assert_eq!((after.ready, after.behind), (true, 0));

    let answer = folder.search("warehouse lease", 10).unwrap();
    let first = &answer.hits[0];
    assert_eq!(first.hit.title, "Lease");
    assert!(first.words.is_some() && first.meaning.is_some(), "both halves found it");

    // Nothing changed: the model is not asked again.
    let read = passages.load(Ordering::Acquire);
    assert_eq!(folder.update().unwrap().embedded, 0);
    assert_eq!(passages.load(Ordering::Acquire), read);
}

/// A note written after the vectors were computed is found by its words and
/// counted as not covered, until an update reads it.
#[test]
fn the_meaning_says_what_it_does_not_cover() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let (folder, _) = folder(notes.path(), store.path());
    folder.update().unwrap();

    // The words alone are brought up to date, as another process would.
    write(notes.path(), "boat.md", "# Boat\n\nThe anchor chain is forty metres.\n");
    folder.library().update().unwrap();
    assert_eq!(folder.standing().unwrap().meaning.unwrap().behind, 1);
    let found = folder.search("anchor chain", 10).unwrap();
    assert_eq!(found.hits[0].hit.title, "Boat");
    assert!(found.hits[0].meaning.is_none());

    folder.update().unwrap();
    assert_eq!(folder.standing().unwrap().meaning.unwrap().behind, 0);
    assert!(folder.search("anchor chain", 10).unwrap().hits[0].meaning.is_some());
}

/// A row is pointed at the sentence that answered, when asked.
#[test]
fn a_row_is_explained_by_its_closest_sentence() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let (folder, _) = folder(notes.path(), store.path());
    folder.update().unwrap();

    let query = "limescale citric acid";
    let mut answer = folder.search(query, 10).unwrap();
    let row = answer.hits.iter_mut().find(|ranked| ranked.hit.title == "Kettle").unwrap();
    assert!(folder.explain(query, &mut row.hit).unwrap());
    let closest = row.hit.closest.clone().expect("a sentence is marked");
    assert_eq!(&row.hit.fragment[closest], "Limescale leaves with citric acid.");
}

/// A search asked while an update holds the model is answered: the update
/// lends the model between batches rather than keeping it to the end.
#[test]
fn a_search_is_answered_while_an_update_runs() {
    let notes = vault();
    for n in 0..80 {
        write(
            notes.path(),
            &format!("many/note-{n}.md"),
            &format!("# Note {n}\n\nParagraph number {n} about nothing much.\n"),
        );
    }
    let store = tempfile::tempdir().unwrap();
    let (folder, _) = folder(notes.path(), store.path());
    folder.library().update().unwrap();

    std::thread::scope(|scope| {
        let updating = scope.spawn(|| folder.update().unwrap());
        for _ in 0..20 {
            folder.search("warehouse lease", 5).unwrap();
        }
        assert_eq!(updating.join().unwrap().embedded, 82);
    });
    assert_eq!(folder.search("warehouse lease", 5).unwrap().hits[0].hit.title, "Lease");
}
