//! The commands the window can call.
//!
//! Everything crossing this boundary comes from the webview, so a command
//! validates what it is handed rather than trusting it. The one that matters
//! most is opening a file: the window may open what the library indexed and
//! nothing else, so a compromised page cannot use nooma to launch an
//! arbitrary path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nooma_core::{Finder, Hit, Library, Progress, Status, UpdateReport};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

/// The most results the window asks for; more than a screen is noise.
const MAX_RESULTS: usize = 50;

/// Where the library lives, the index kept open for searching, and whether
/// an update is running.
pub struct AppState {
    root: Option<PathBuf>,
    /// Opened on the first search and kept: opening costs more than a search.
    finder: Mutex<Option<Finder>>,
    updating: AtomicBool,
}

impl AppState {
    /// The library in the user's data directory, shared with the `nooma` CLI —
    /// or in `NOOMA_STORE`, as the CLI reads it, for a demo or a test that
    /// must not touch the real one.
    pub fn new() -> Self {
        Self {
            root: std::env::var_os("NOOMA_STORE").filter(|dir| !dir.is_empty()).map(PathBuf::from),
            finder: Mutex::new(None),
            updating: AtomicBool::new(false),
        }
    }

    /// A library in a given directory, for tests.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: Some(root.into()),
            finder: Mutex::new(None),
            updating: AtomicBool::new(false),
        }
    }

    fn library(&self) -> Result<Library, String> {
        let library = match &self.root {
            Some(root) => Library::open_at(root),
            None => Library::open(),
        };
        library.map_err(|e| e.to_string())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// What the status bar shows.
#[tauri::command]
pub fn status(state: State<'_, AppState>) -> Result<Status, String> {
    status_of(&state)
}

pub fn status_of(state: &AppState) -> Result<Status, String> {
    state.library()?.status().map_err(|e| e.to_string())
}

/// The results for a query, best first.
///
/// Answers from the stored index: the window keeps it current by updating
/// when it opens, and a search on every keystroke must not walk the disk.
#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Result<Found, String> {
    search_in(&state, &query, limit)
}

/// What a search returns: the hits, and how long finding them took.
#[derive(Debug, Serialize)]
pub struct Found {
    /// The documents, best first.
    pub hits: Vec<Hit>,
    /// Milliseconds spent in the index.
    pub took_ms: u64,
}

pub fn search_in(state: &AppState, query: &str, limit: Option<usize>) -> Result<Found, String> {
    let started = std::time::Instant::now();
    let limit = limit.unwrap_or(MAX_RESULTS).min(MAX_RESULTS);
    let mut kept = state.finder.lock().map_err(|_| "the search state is poisoned".to_string())?;
    if kept.is_none() {
        *kept = state.library()?.finder().map_err(|e| e.to_string())?;
    }
    let hits = match kept.as_ref() {
        Some(finder) => finder.search(query, limit).map_err(|e| e.to_string())?,
        // Nothing has been indexed yet; the next search tries again.
        None => Vec::new(),
    };
    Ok(Found {
        hits,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Progress of an update, as the `index-progress` event carries it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProgressEvent {
    /// Files read so far.
    pub done: usize,
    /// Files this update reads.
    pub total: usize,
}

/// Bring the index up to date, reporting progress as `index-progress`.
///
/// One update at a time: a second call while one runs is answered with
/// `None` rather than queued, since the running one already covers it.
#[tauri::command]
pub async fn update(app: AppHandle, state: State<'_, AppState>) -> Result<Option<UpdateReport>, String> {
    if state.updating.swap(true, Ordering::AcqRel) {
        return Ok(None);
    }
    let result = update_in(&state, &|progress| {
        let _ = app.emit(
            "index-progress",
            ProgressEvent {
                done: progress.done,
                total: progress.total,
            },
        );
    });
    state.updating.store(false, Ordering::Release);
    result.map(Some)
}

pub fn update_in(state: &AppState, emit: &(dyn Fn(Progress) + Sync)) -> Result<UpdateReport, String> {
    let library = state.library()?;
    // A progress bar redrawn per file costs more than the files on a fast
    // disk; a hundredth of the work, the first and the last are enough.
    let last = AtomicUsize::new(0);
    let throttled = |progress: Progress| {
        let step = (progress.total / 100).max(1);
        let previous = last.load(Ordering::Relaxed);
        if progress.done == 0 || progress.done == progress.total || progress.done >= previous + step {
            last.store(progress.done, Ordering::Relaxed);
            emit(progress);
        }
    };
    let result = library.update_with(&throttled);
    // The kept finder follows commits on its own, but an update may have
    // created the index it could not open before; the next search reopens.
    if let Ok(mut kept) = state.finder.lock() {
        *kept = None;
    }
    match result {
        Ok(report) => Ok(report),
        // Another nooma - the CLI, most likely - is writing this library.
        // Its update is this update; the window says so rather than failing.
        Err(nooma_core::Error::Busy) => Err("busy".to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Add a folder to search. The window updates the index after.
#[tauri::command]
pub fn add_source(state: State<'_, AppState>, path: PathBuf) -> Result<Status, String> {
    add_source_in(&state, &path)
}

pub fn add_source_in(state: &AppState, path: &Path) -> Result<Status, String> {
    let mut library = state.library()?;
    library.add_source(path, Vec::new()).map_err(|e| e.to_string())?;
    library.status().map_err(|e| e.to_string())
}

/// Open a found document in its default application.
#[tauri::command]
pub fn open_document(app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    let path = indexed_path(&state, &path)?;
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// Show a found document in the file manager.
#[tauri::command]
pub fn reveal_document(app: AppHandle, state: State<'_, AppState>, path: PathBuf) -> Result<(), String> {
    let path = indexed_path(&state, &path)?;
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

/// The path, if it is a file under one of the library's sources.
pub fn indexed_path(state: &AppState, path: &Path) -> Result<PathBuf, String> {
    let library = state.library()?;
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    // `..` would let a path that starts under a source end outside it.
    let escapes = absolute.components().any(|c| matches!(c, std::path::Component::ParentDir));
    let inside = library.sources().iter().any(|source| absolute.starts_with(&source.path));
    if escapes || !inside || !absolute.is_file() {
        return Err(format!("{} is not a document in this library", absolute.display()));
    }
    Ok(absolute)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_with(notes: &Path) -> (tempfile::TempDir, AppState) {
        let store = tempfile::tempdir().unwrap();
        let state = AppState::at(store.path());
        add_source_in(&state, notes).unwrap();
        (store, state)
    }

    #[test]
    fn a_folder_added_is_searchable_after_an_update() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("a.md"), "# Greenhouse\n\nOpen the vents at noon.\n").unwrap();
        let (_store, state) = library_with(notes.path());
        assert!(search_in(&state, "vents", None).unwrap().hits.is_empty(), "nothing is read before an update");
        let seen = std::sync::Mutex::new(Vec::new());
        let report = update_in(&state, &|p| seen.lock().unwrap().push((p.done, p.total))).unwrap();
        assert_eq!(report.documents, 1);
        assert_eq!(seen.lock().unwrap().last(), Some(&(1, 1)), "the last progress is the whole");
        assert_eq!(search_in(&state, "vent", None).unwrap().hits[0].title, "Greenhouse");
    }

    #[test]
    fn only_a_document_under_a_source_can_be_opened() {
        let notes = tempfile::tempdir().unwrap();
        let inside = notes.path().join("a.md");
        std::fs::write(&inside, "text").unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let (_store, state) = library_with(notes.path());

        assert_eq!(indexed_path(&state, &inside).unwrap(), std::path::absolute(&inside).unwrap());
        assert!(indexed_path(&state, outside.path()).is_err());
        assert!(indexed_path(&state, &notes.path().join("..").join("elsewhere.md")).is_err());
        assert!(indexed_path(&state, notes.path()).is_err(), "a folder is not a document");
    }

    #[test]
    fn the_window_never_asks_for_more_than_a_screen() {
        let notes = tempfile::tempdir().unwrap();
        for i in 0..60 {
            std::fs::write(notes.path().join(format!("{i}.txt")), "shared word\n").unwrap();
        }
        let (_store, state) = library_with(notes.path());
        update_in(&state, &|_| {}).unwrap();
        assert_eq!(search_in(&state, "shared", Some(1000)).unwrap().hits.len(), MAX_RESULTS);
    }
}
