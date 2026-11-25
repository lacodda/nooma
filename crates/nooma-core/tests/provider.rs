//! The provider a neighbour asks: one folder of notes, searched by its words.
//!
//! What is defended here is the seam itself. A caller holds a `dyn FullText`
//! and cannot tell nooma's provider from its own; a folder answers from what
//! an update read and never from the disk; and a store follows the folder it
//! was opened for. The half by meaning is in `provider_meaning.rs`.
//!
//! Every note is written by the test.

use std::path::Path;

use nooma_core::provider::{Standing, Step, Updated};
use nooma_core::{Answer, Folder, FullText, Result};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "home/kettle.md", "# Kettle\n\nDescale the kettle with citric acid once a month.\n");
    write(
        dir.path(),
        "work/lease.md",
        "# Lease\n\nThe warehouse lease is renewed in March, invoice 4471.\n",
    );
    write(dir.path(), "drafts/scratch.md", "# Scratch\n\nA kettle of fish, not worth finding.\n");
    dir
}

fn titles(answer: &Answer) -> Vec<&str> {
    answer.hits.iter().map(|ranked| ranked.hit.title.as_str()).collect()
}

/// The stand-in for a caller's own search: it reads the files line by line
/// each time it is asked, and holds nothing.
struct Lines<'a>(&'a Path);

impl FullText for Lines<'_> {
    fn update(&self) -> Result<Updated> {
        Ok(Updated::default())
    }

    fn search(&self, _query: &str, _limit: usize) -> Result<Answer> {
        Ok(Answer::default())
    }

    fn standing(&self) -> Result<Standing> {
        Ok(Standing {
            documents: std::fs::read_dir(self.0).map(Iterator::count).unwrap_or(0),
            indexed_at: None,
            stale: None,
            meaning: None,
        })
    }
}

#[test]
fn a_folder_is_found_by_its_words_once_it_is_read() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let folder = Folder::open(notes.path(), store.path(), Vec::new()).unwrap();

    // Nothing is read by opening, and nothing read is an answer, not an
    // error.
    assert_eq!(folder.search("kettle", 10).unwrap(), Answer::default());
    assert_eq!(folder.standing().unwrap().documents, 0);

    let updated = folder.update().unwrap();
    assert_eq!(updated.files.indexed, 3);
    assert_eq!(updated.embedded, 0, "no model was given");

    let answer = folder.search("invoice 4471", 10).unwrap();
    assert_eq!(titles(&answer), ["Lease"]);
    assert!(answer.all_words);
    let row = &answer.hits[0];
    assert!(row.words.is_some() && row.meaning.is_none());
    assert!(!row.hit.highlights.is_empty(), "the words that matched are marked");
    assert!(row.hit.path.starts_with(folder.folder()));

    let standing = folder.standing().unwrap();
    assert_eq!(standing.documents, 3);
    assert!(standing.indexed_at.is_some());
    assert_eq!(standing.meaning, None, "words alone");
}

/// A search answers from what the last update read: a file written since is
/// not found until the next one, and a file removed leaves with it.
#[test]
fn a_search_answers_from_the_last_update() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let folder = Folder::open(notes.path(), store.path(), Vec::new()).unwrap();
    folder.update().unwrap();

    write(
        notes.path(),
        "garden/greenhouse.md",
        "# Greenhouse\n\nThe tomatoes want the vents open by noon.\n",
    );
    std::fs::remove_file(notes.path().join("work/lease.md")).unwrap();
    assert!(folder.search("tomatoes", 10).unwrap().hits.is_empty());
    assert_eq!(titles(&folder.search("warehouse", 10).unwrap()), ["Lease"]);

    let updated = folder.update().unwrap();
    assert_eq!((updated.files.indexed, updated.files.removed), (1, 1));
    assert_eq!(titles(&folder.search("tomatoes", 10).unwrap()), ["Greenhouse"]);
    assert!(folder.search("warehouse", 10).unwrap().hits.is_empty());
}

/// The whole point of a trait: the caller holds a provider, not nooma.
#[test]
fn a_caller_holds_either_provider_alike() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let folder = Folder::open(notes.path(), store.path(), Vec::new()).unwrap();
    let lines = Lines(notes.path());

    let providers: [&dyn FullText; 2] = [&folder, &lines];
    let mut found = Vec::new();
    for provider in providers {
        provider.update().unwrap();
        let mut answer = provider.search("descale", 5).unwrap();
        // Neither was given a model, so neither can say why.
        for ranked in &mut answer.hits {
            assert!(!provider.explain("descale", &mut ranked.hit).unwrap());
        }
        found.push(answer.hits.len());
    }
    assert_eq!(found, [1, 0]);
}

/// A store is one folder's. Opened for another, or with other exclusions,
/// it follows, and what no longer belongs leaves at the next update.
#[test]
fn a_store_follows_the_folder_it_is_opened_for() {
    let notes = vault();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "boat.md", "# Boat\n\nThe kettle on the boat runs on gas.\n");
    let store = tempfile::tempdir().unwrap();

    let folder = Folder::open(notes.path(), store.path(), vec!["drafts/".to_string()]).unwrap();
    folder.update().unwrap();
    assert_eq!(titles(&folder.search("kettle", 10).unwrap()), ["Kettle"], "the draft is left out");
    drop(folder);

    // The same folder and exclusions: nothing to read again.
    let again = Folder::open(notes.path(), store.path(), vec!["drafts/".to_string()]).unwrap();
    let updated = again.update().unwrap();
    assert_eq!((updated.files.indexed, updated.files.removed), (0, 0));
    drop(again);

    // Other exclusions: the draft joins.
    let whole = Folder::open(notes.path(), store.path(), Vec::new()).unwrap();
    whole.update().unwrap();
    assert_eq!(whole.search("kettle", 10).unwrap().hits.len(), 2);
    drop(whole);

    // Another folder: the first one's notes leave.
    let moved = Folder::open(other.path(), store.path(), Vec::new()).unwrap();
    assert_eq!(moved.library().sources().len(), 1);
    moved.update().unwrap();
    assert_eq!(titles(&moved.search("kettle", 10).unwrap()), ["Boat"]);
}

#[test]
fn an_update_says_how_far_it_has_got() {
    let notes = vault();
    let store = tempfile::tempdir().unwrap();
    let folder = Folder::open(notes.path(), store.path(), Vec::new()).unwrap();

    let seen = std::sync::Mutex::new(Vec::new());
    folder.update_with(&|step| seen.lock().unwrap().push(step)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(
        seen.iter().any(|step| matches!(step, Step::Files(at) if at.total == 3 && at.done == 3)),
        "{seen:?}"
    );
    assert!(!seen.iter().any(|step| matches!(step, Step::Passages(_))), "no model, no passages");
}

/// What a window keeps in its state has to cross threads.
#[test]
fn a_folder_is_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Folder>();
}
