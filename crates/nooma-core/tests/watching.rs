//! Keeping the index up with the folders: an update over the paths that
//! changed, a watcher that names them, and the pace of an update nobody
//! waits for.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use nooma_core::{Library, Pace, Scope};

struct Folder {
    _dir: tempfile::TempDir,
    notes: PathBuf,
    store: PathBuf,
}

impl Folder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(notes.join("home")).unwrap();
        std::fs::create_dir_all(notes.join("work")).unwrap();
        std::fs::write(notes.join("home/kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        std::fs::write(notes.join("home/bike.md"), "# Bicycle\n\nOil the chain after rain.\n").unwrap();
        std::fs::write(notes.join("work/lease.md"), "# Lease\n\nThe warehouse lease ends in March.\n").unwrap();
        let store = dir.path().join("store");
        Self { _dir: dir, notes, store }
    }

    fn library(&self) -> Library {
        let mut library = Library::open_at(&self.store).unwrap();
        if library.sources().is_empty() {
            library.add_source(&self.notes, Vec::new()).unwrap();
        }
        library
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.notes.join(rel)
    }
}

fn paths(paths: &[PathBuf]) -> Scope {
    Scope::Paths(paths.iter().cloned().collect())
}

fn titles(library: &Library, query: &str) -> Vec<String> {
    library.search(query, 10).unwrap().hits.into_iter().map(|hit| hit.hit.title).collect()
}

fn update(library: &Library, scope: &Scope) -> nooma_core::UpdateReport {
    library.update_scoped(scope, Pace::foreground(&|_| {})).unwrap()
}

#[test]
fn an_update_over_changed_paths_reads_those_and_keeps_the_rest() {
    let folder = Folder::new();
    let library = folder.library();
    library.update().unwrap();

    // Two files changed on disk; the scope names one of them.
    std::fs::write(folder.path("home/kettle.md"), "# Kettle\n\nThe kettle is descaled with vinegar now.\n").unwrap();
    std::fs::write(folder.path("work/lease.md"), "# Lease\n\nThe warehouse lease was renewed.\n").unwrap();
    let report = update(&library, &paths(&[folder.path("home/kettle.md")]));
    assert_eq!((report.indexed, report.documents), (1, 3), "{report:?}");
    assert_eq!(titles(&library, "vinegar"), ["Kettle"]);
    assert!(titles(&library, "renewed").is_empty(), "a file outside the scope is kept as it was");
    assert_eq!(titles(&library, "ends march"), ["Lease"]);

    // A whole update catches what the scope did not name.
    assert_eq!(library.update().unwrap().indexed, 1);
    assert_eq!(titles(&library, "renewed"), ["Lease"]);
}

#[test]
fn a_folder_named_covers_everything_under_it_new_or_gone() {
    let folder = Folder::new();
    let library = folder.library();
    library.update().unwrap();

    std::fs::create_dir_all(folder.path("home/garden")).unwrap();
    std::fs::write(folder.path("home/garden/roses.md"), "# Roses\n\nPrune the roses in February.\n").unwrap();
    std::fs::remove_file(folder.path("home/bike.md")).unwrap();
    let report = update(&library, &paths(&[folder.path("home")]));
    assert_eq!((report.indexed, report.removed, report.documents), (1, 1, 3), "{report:?}");
    assert_eq!(titles(&library, "prune roses"), ["Roses"]);
    assert!(titles(&library, "chain").is_empty());

    // A folder renamed is two paths: the one gone, the one come.
    std::fs::rename(folder.path("home"), folder.path("house")).unwrap();
    let report = update(&library, &paths(&[folder.path("home"), folder.path("house")]));
    assert_eq!((report.indexed, report.removed, report.documents), (2, 2, 3), "{report:?}");
    let found = library.search("prune roses", 5).unwrap();
    assert_eq!(found.hits[0].hit.path, folder.path("house/garden/roses.md"));
}

#[test]
fn a_file_of_rules_that_changed_is_its_whole_folder_changed() {
    let folder = Folder::new();
    let library = folder.library();
    library.update().unwrap();

    let rules = folder.path("work/.nooma-ignore");
    std::fs::write(&rules, "lease.md\n").unwrap();
    let report = update(&library, &paths(std::slice::from_ref(&rules)));
    assert_eq!(report.removed, 1, "{report:?}");
    assert!(titles(&library, "warehouse").is_empty(), "a file the rules leave out leaves the index");

    std::fs::remove_file(&rules).unwrap();
    assert_eq!(update(&library, &paths(&[rules])).indexed, 1);
    assert_eq!(titles(&library, "warehouse"), ["Lease"]);
}

#[test]
fn a_scope_walks_down_by_the_rules_a_whole_walk_keeps() {
    let folder = Folder::new();
    std::fs::write(folder.path(".nooma-ignore"), "drafts/\n").unwrap();
    let library = folder.library();
    library.update().unwrap();

    // The rule is in the folder above the one that changed: a scope that
    // started its walk at the changed path would not know it.
    std::fs::create_dir_all(folder.path("drafts")).unwrap();
    std::fs::write(folder.path("drafts/secret.md"), "# Draft\n\nNot for the index.\n").unwrap();
    let report = update(&library, &paths(&[folder.path("drafts/secret.md")]));
    assert_eq!(report.indexed, 0, "{report:?}");
    assert!(titles(&library, "index").is_empty());
}

#[test]
fn an_update_that_found_nothing_changed_writes_nothing() {
    let folder = Folder::new();
    let library = folder.library();
    library.update().unwrap();
    let manifest = folder.store.join("manifest.json");
    let before = std::fs::read(&manifest).unwrap();
    let indexed_at = library.status().unwrap().indexed_at;
    std::thread::sleep(Duration::from_millis(1100));

    std::fs::write(folder.path("home/build.log"), "not a document").unwrap();
    let report = update(&library, &paths(&[folder.path("home/build.log")]));
    assert_eq!((report.indexed, report.removed, report.documents), (0, 0, 3));
    let report = library.update().unwrap();
    assert_eq!((report.indexed, report.removed), (0, 0));
    assert_eq!(std::fs::read(&manifest).unwrap(), before, "the manifest was written again");
    assert_eq!(library.status().unwrap().indexed_at, indexed_at);

    // A file saved with the same bytes is a change to its time, which the
    // manifest keeps so the file is not hashed again.
    std::fs::write(folder.path("home/bike.md"), "# Bicycle\n\nOil the chain after rain.\n").unwrap();
    let mtime = std::fs::metadata(folder.path("home/bike.md")).unwrap().modified().unwrap();
    filetime_later(&folder.path("home/bike.md"), mtime);
    assert_eq!(library.update().unwrap().indexed, 0);
    assert_ne!(std::fs::read(&manifest).unwrap(), before);
}

/// Move a file's modification time a few seconds on, so a file system that
/// keeps coarse times sees it changed.
fn filetime_later(path: &Path, from: std::time::SystemTime) {
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(from + Duration::from_secs(5)).unwrap();
}

#[test]
fn a_rebuild_reads_everything_whatever_the_scope() {
    let folder = Folder::new();
    let library = folder.library();
    let report = update(&library, &paths(&[folder.path("home/kettle.md")]));
    assert_eq!(report.documents, 3, "the first update is a whole one: {report:?}");
}

#[test]
fn an_update_nobody_waits_for_holds_where_it_is_told_to() {
    let folder = Folder::new();
    let library = folder.library();
    let held = AtomicBool::new(true);
    let asked = AtomicUsize::new(0);
    let hold = || {
        asked.fetch_add(1, Ordering::SeqCst);
        while held.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let progress = |_| {};
    std::thread::scope(|scope| {
        let running = scope.spawn(|| library.update_scoped(&Scope::Everything, Pace::background(&progress, &hold)).unwrap());
        std::thread::sleep(Duration::from_millis(400));
        assert!(!running.is_finished(), "the update did not hold");
        assert!(asked.load(Ordering::SeqCst) >= 1);
        assert_eq!(library.status().unwrap().documents, 0, "nothing was written while held");
        held.store(false, Ordering::SeqCst);
        assert_eq!(running.join().unwrap().documents, 3);
    });
    assert_eq!(asked.load(Ordering::SeqCst), 3, "held before each file it read");
    // One someone waits for is never held.
    std::fs::write(folder.path("home/kettle.md"), "# Kettle\n\nVinegar.\n").unwrap();
    held.store(true, Ordering::SeqCst);
    let report = library.update_scoped(&Scope::Everything, Pace::foreground(&progress)).unwrap();
    assert_eq!(report.indexed, 1);
}

#[test]
fn scopes_join_into_one() {
    let a = Scope::Paths(BTreeSet::from([PathBuf::from("a")]));
    let b = Scope::Paths(BTreeSet::from([PathBuf::from("b")]));
    assert_eq!(a.clone().and(b), Scope::Paths(BTreeSet::from([PathBuf::from("a"), PathBuf::from("b")])));
    assert_eq!(a.clone().and(Scope::Everything), Scope::Everything);
    assert_eq!(Scope::Everything.and(a), Scope::Everything);
}

/// The system's own notifications, end to end: a note saved is named, a
/// hidden folder saved into is not.
#[test]
fn a_watcher_names_what_changed() {
    let folder = Folder::new();
    std::fs::create_dir_all(folder.path(".obsidian")).unwrap();
    let library = folder.library();
    library.update().unwrap();
    let (told, heard) = mpsc::channel();
    let watcher = library.watch(move |scope| told.send(scope).unwrap()).unwrap();
    assert!(watcher.unwatched().is_empty(), "{:?}", watcher.unwatched());
    // FSEvents starts reporting a moment after the stream is made.
    std::thread::sleep(Duration::from_millis(500));

    std::fs::write(folder.path(".obsidian/workspace.json"), "{}").unwrap();
    std::fs::write(folder.path("home/kettle.md"), "# Kettle\n\nThe kettle is descaled with vinegar now.\n").unwrap();
    let mut named = BTreeSet::new();
    while !named.contains(&folder.path("home/kettle.md")) {
        match heard.recv_timeout(Duration::from_secs(20)).expect("the watcher said nothing") {
            Scope::Paths(paths) => named.extend(paths),
            Scope::Everything => panic!("the system lost count of a single save"),
        }
    }
    assert!(named.iter().all(|path| !path.starts_with(folder.path(".obsidian"))), "{named:?}");
    let report = update(&library, &Scope::Paths(named));
    assert_eq!(report.indexed, 1);
    assert_eq!(titles(&library, "vinegar"), ["Kettle"]);

    // Dropped, it says nothing more.
    drop(watcher);
    while heard.recv_timeout(Duration::from_millis(800)).is_ok() {}
    std::fs::write(folder.path("home/bike.md"), "# Bicycle\n\nNew chain.\n").unwrap();
    assert!(heard.recv_timeout(Duration::from_secs(2)).is_err(), "a dropped watcher still watched");
}

#[test]
fn a_folder_that_is_not_there_is_not_watched() {
    let folder = Folder::new();
    let library = folder.library();
    std::fs::rename(&folder.notes, folder.notes.with_file_name("elsewhere")).unwrap();
    let watcher = library.watch(|_| {}).unwrap();
    assert_eq!(watcher.unwatched().len(), 1);
    assert_eq!(watcher.unwatched()[0].0, folder.notes);
}
