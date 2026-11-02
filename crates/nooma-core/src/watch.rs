//! Watching the folders a library searches, so its index follows them.
//!
//! The system says which paths changed - ReadDirectoryChangesW on Windows,
//! inotify on Linux, FSEvents on macOS, through `notify` - and that is all it
//! is trusted to say: where to look. An update
//! ([`Library::update_scoped`]) reads the files there as it reads any, by
//! their size, time and hash. A watcher that misses a change - its buffer
//! overflowed in a burst of thousands, the folder is on a share that does not
//! report, the drive was unplugged and is back - costs freshness until the
//! next whole update, never a wrong index.
//!
//! Changes come in bursts: an editor saves a note as a write, a rename and
//! another write; a copy writes a book in a thousand pieces. They are
//! gathered until the folders have been quiet for a moment - or, under a
//! steady stream, for a few seconds at most - and handed over as one scope.
//!
//! What no update would read is not handed over at all: hidden folders, a
//! vault's `.obsidian` saved every few seconds and a repository's `.git`
//! among them, and the library's own files, should they be under a source.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use notify::{EventKind, RecursiveMode, Watcher as _};

use crate::error::{Error, Result};
use crate::library::{Library, Scope};

/// How long the folders have to be quiet before what changed is handed over.
const QUIET: Duration = Duration::from_millis(500);

/// The longest changes are gathered under a steady stream of them.
const LONGEST: Duration = Duration::from_secs(5);

/// The files that say what a folder leaves out, hidden by their names and
/// looked at all the same.
const RULE_FILES: [&str; 3] = [".gitignore", ".ignore", ".nooma-ignore"];

/// The folders of a library, watched until this is dropped.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
    unwatched: Vec<(PathBuf, String)>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("unwatched", &self.unwatched).finish_non_exhaustive()
    }
}

impl Watcher {
    /// The sources that are not watched, with why: a folder that is not
    /// there, or one the system refused to watch - Linux has a limit on how
    /// many folders one person may.
    pub fn unwatched(&self) -> &[(PathBuf, String)] {
        &self.unwatched
    }
}

impl Library {
    /// Watch the folders, telling `changed` what changed in them, a burst at
    /// a time, on a thread of the watcher's own. It is told
    /// [`Scope::Everything`] when the system lost count of what changed.
    ///
    /// The sources are the ones the library has now: one added later is
    /// watched by a watcher made after it.
    pub fn watch(&self, mut changed: impl FnMut(Scope) + Send + 'static) -> Result<Watcher> {
        let places = Places::of(self);
        let (sender, events) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::recommended_watcher(sender).map_err(|error| Error::io(self.root(), std::io::Error::other(error)))?;
        let mut unwatched = Vec::new();
        for source in self.sources() {
            if !source.path.is_dir() {
                unwatched.push((source.path.clone(), "the folder cannot be reached".to_string()));
                continue;
            }
            if let Err(error) = watcher.watch(&source.path, RecursiveMode::Recursive) {
                unwatched.push((source.path.clone(), error.to_string()));
            }
        }
        std::thread::Builder::new()
            .name("nooma-watch".to_string())
            .spawn(move || {
                // Ends when the watcher is dropped, and its sender with it.
                while let Ok(first) = events.recv() {
                    let mut burst = Burst::default();
                    burst.add(first, &places);
                    let began = Instant::now();
                    loop {
                        let left = LONGEST.saturating_sub(began.elapsed());
                        if left.is_zero() {
                            break;
                        }
                        match events.recv_timeout(QUIET.min(left)) {
                            Ok(event) => burst.add(event, &places),
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    if let Some(scope) = burst.scope() {
                        changed(scope);
                    }
                }
            })
            .map_err(|error| Error::io(self.root(), error))?;
        Ok(Watcher { _watcher: watcher, unwatched })
    }
}

/// Changes gathered in one burst.
#[derive(Debug, Default)]
struct Burst {
    paths: BTreeSet<PathBuf>,
    everything: bool,
}

impl Burst {
    fn add(&mut self, event: notify::Result<notify::Event>, places: &Places) {
        let event = match event {
            Ok(event) => event,
            // The system failed to say something: what it failed to say may
            // be anything.
            Err(_) => {
                self.everything = true;
                return;
            }
        };
        if event.need_rescan() {
            self.everything = true;
            return;
        }
        // Reading a file changes nothing in it.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        self.paths.extend(event.paths.iter().filter_map(|path| places.place(path)));
    }

    fn scope(self) -> Option<Scope> {
        match (self.everything, self.paths.is_empty()) {
            (true, _) => Some(Scope::Everything),
            (false, true) => None,
            (false, false) => Some(Scope::Paths(self.paths)),
        }
    }
}

/// Where a path the system names is, as the library knows its sources.
#[derive(Debug)]
struct Places {
    /// Each source as it was added, and as the system may name it instead:
    /// macOS reports `/private/var/...` for a folder added as `/var/...`.
    sources: Vec<(PathBuf, Option<PathBuf>)>,
    /// The library's own files, likewise.
    store: (PathBuf, Option<PathBuf>),
}

impl Places {
    fn of(library: &Library) -> Self {
        let both = |path: &Path| {
            let named = std::fs::canonicalize(path).ok().filter(|named| named != path);
            (path.to_path_buf(), named)
        };
        Self {
            sources: library.sources().iter().map(|source| both(&source.path)).collect(),
            store: both(library.root()),
        }
    }

    /// The path as it is under its source, or `None` when no update would
    /// read it: under no source, among the library's own files, or in a
    /// hidden folder - or hidden itself, unless it is a file of rules.
    fn place(&self, path: &Path) -> Option<PathBuf> {
        let under = |(added, named): &(PathBuf, Option<PathBuf>)| -> Option<PathBuf> {
            if let Ok(inside) = path.strip_prefix(added) {
                return Some(added.join(inside));
            }
            let inside = path.strip_prefix(named.as_ref()?).ok()?;
            Some(added.join(inside))
        };
        if under(&self.store).is_some() {
            return None;
        }
        let (source, placed) = self.sources.iter().find_map(|source| under(source).map(|placed| (&source.0, placed)))?;
        let inside = placed.strip_prefix(source).ok()?;
        let parts: Vec<&std::ffi::OsStr> = inside.iter().collect();
        let hidden = parts.iter().enumerate().any(|(at, part)| {
            let part = part.to_string_lossy();
            part.starts_with('.') && !(at + 1 == parts.len() && RULE_FILES.contains(&part.as_ref()))
        });
        (!hidden).then_some(placed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            sources: vec![
                (PathBuf::from("/home/a/notes"), None),
                (PathBuf::from("/var/x/books"), Some(PathBuf::from("/private/var/x/books"))),
            ],
            store: (PathBuf::from("/home/a/notes/.store"), None),
        }
    }

    #[test]
    fn a_path_is_placed_under_its_source_as_the_source_was_added() {
        let places = places();
        assert_eq!(places.place(Path::new("/home/a/notes/plan.md")), Some(PathBuf::from("/home/a/notes/plan.md")));
        assert_eq!(
            places.place(Path::new("/private/var/x/books/novel.epub")),
            Some(PathBuf::from("/var/x/books/novel.epub")),
            "the system's name for a folder is the source's"
        );
        assert_eq!(places.place(Path::new("/home/a/notes")), Some(PathBuf::from("/home/a/notes")));
        assert_eq!(places.place(Path::new("/home/a/elsewhere.md")), None);
        assert_eq!(
            places.place(Path::new("/home/a/notes-old/plan.md")),
            None,
            "a path is its parts, not its letters"
        );
    }

    #[test]
    fn what_no_update_reads_is_not_placed() {
        let places = places();
        assert_eq!(places.place(Path::new("/home/a/notes/.obsidian/workspace.json")), None);
        assert_eq!(places.place(Path::new("/home/a/notes/.git/index")), None);
        assert_eq!(places.place(Path::new("/home/a/notes/.draft.md")), None);
        assert_eq!(places.place(Path::new("/home/a/notes/.store/manifest.json")), None, "the library's own files");
        assert_eq!(
            places.place(Path::new("/home/a/notes/work/.nooma-ignore")),
            Some(PathBuf::from("/home/a/notes/work/.nooma-ignore")),
            "a file of rules changes what its folder holds"
        );
        assert_eq!(places.place(Path::new("/home/a/notes/.git/.gitignore")), None, "rules inside a hidden folder");
    }

    #[test]
    fn a_burst_is_one_scope_and_a_lost_count_is_everything() {
        let places = places();
        let event = |kind: EventKind, path: &str| Ok(notify::Event::new(kind).add_path(PathBuf::from(path)));
        let mut burst = Burst::default();
        burst.add(event(EventKind::Access(notify::event::AccessKind::Any), "/home/a/notes/read.md"), &places);
        assert_eq!(Burst::default().scope(), None);
        burst.add(event(EventKind::Any, "/home/a/notes/a.md"), &places);
        burst.add(event(EventKind::Any, "/home/a/notes/a.md"), &places);
        burst.add(event(EventKind::Any, "/home/a/notes/.obsidian/x.json"), &places);
        burst.add(event(EventKind::Any, "/private/var/x/books/b.pdf"), &places);
        assert_eq!(
            burst.scope(),
            Some(Scope::Paths(BTreeSet::from([
                PathBuf::from("/home/a/notes/a.md"),
                PathBuf::from("/var/x/books/b.pdf")
            ])))
        );

        let mut lost = Burst::default();
        lost.add(event(EventKind::Any, "/home/a/notes/a.md"), &places);
        lost.add(Ok(notify::Event::new(EventKind::Other).set_flag(notify::event::Flag::Rescan)), &places);
        assert_eq!(lost.scope(), Some(Scope::Everything));
        let mut failed = Burst::default();
        failed.add(Err(notify::Error::generic("the watch failed")), &places);
        assert_eq!(failed.scope(), Some(Scope::Everything));
    }
}
