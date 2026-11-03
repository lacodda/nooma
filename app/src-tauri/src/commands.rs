//! The commands the window can call.
//!
//! Everything crossing this boundary comes from the webview, so a command
//! validates what it is handed rather than trusting it. The one that matters
//! most is opening a file: the window may open what the library indexed and
//! nothing else, so a compromised page cannot use nooma to launch an
//! arbitrary path.
//!
//! The window shows one list, and asks for it twice. The words alone answer
//! in milliseconds. The whole answer - the words and the meaning, ranked
//! together - needs the model, which the window also uses to compute vectors
//! in the background: that work borrows the model one small batch at a time
//! and lets a waiting search go first, so a search waits at most a batch for
//! it, and the words' answer stands in until it comes. A row the meaning found
//! in another language is then asked why, one at a time: the sentence that
//! answered, read by the model as a search reads its query.
//!
//! The index is kept up to date here, not by the page: one thread of the
//! window's own runs every update, below the person's programs and holding
//! while the machine is on its battery. It runs a whole one when the window
//! opens, when a folder is added and when the window comes back into focus a
//! while after the last, and one over the changed paths whenever the watcher
//! names them; asked while it runs, it runs once more after, over everything
//! it was asked about meanwhile. The page is told how it goes by
//! `index-state`.
//!
//! The row selected is shown in a preview beside the list: its chunk whole,
//! among its neighbours in the document, the query's words marked - and how
//! the document would be opened at it, so the button can say so. Opening one
//! opens it there, in the program the system has for it (see
//! [`crate::opening`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use nooma_core::document::DocKind;
use nooma_core::embed::OnnxEmbedder;
use nooma_core::library::indexing_threads;
use nooma_core::model::ModelSpec;
use nooma_core::{
    Answer, Embedder, Finder, Health, LanguageAxes, Library, Model, Pace, Passage, Preview, Progress, Ranked, Role, Scope, SemanticIndex, Status, UpdateReport,
    VectorReport, Watcher, background, hybrid, why,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager as _, State};
use tauri_plugin_opener::OpenerExt;

use crate::opening::{self, Opening, Os, System};

/// The most results the window asks for; more than a screen is noise.
const MAX_RESULTS: usize = 50;

/// The most chunks a preview shows on either side of the one found: the
/// preview reads on as far as this a step at a time, and the program opened
/// at the place reads on from there.
const MAX_AROUND: usize = 60;

/// The longest the background update steps aside for searches before it takes
/// the model anyway: a guard against a count that never falls, not a pace.
const YIELD_LIMIT: Duration = Duration::from_secs(5);

/// How long after the last whole update the window coming into focus asks
/// for another: what a watcher can miss - a burst too large for it, a share
/// that does not report, a drive plugged back in - is caught when the person
/// comes back to search, at the cost of a walk.
const FRESHEN_AFTER: Duration = Duration::from_secs(120);

/// A model, as the window keeps it: loaded once, shared by the background
/// update and every search.
type Kept = Box<dyn Embedder + Send>;

/// Where the library and the model live, what is kept open between
/// searches, and what is running.
pub struct AppState {
    root: Option<PathBuf>,
    models: Option<PathBuf>,
    /// Opened on the first search and kept: opening costs more than a search.
    finder: Mutex<Option<Finder>>,
    /// The model, loaded on first need.
    embedder: Mutex<Option<Kept>>,
    /// Searches waiting for the model, or using it. The background update
    /// lets them go first between batches: a mutex need not be fair - on
    /// macOS it is not - and an update that takes the model straight back
    /// starved a search for batch after batch.
    searching: AtomicUsize,
    /// The vector index, kept for searching and read again when an update
    /// has replaced it on disk.
    meaning: Mutex<Option<SemanticIndex>>,
    /// Whether vectors are being computed, and how far it has got.
    computing: Mutex<Option<Progress>>,
    /// An update of the vectors was asked for while one ran.
    again: AtomicBool,
    fetching: AtomicBool,
    /// The updates of the index, asked for and run one at a time.
    indexer: Indexer,
    /// The folders watched, until the next whole update watches them anew.
    watcher: Mutex<Option<Watcher>>,
    /// What the machine opens documents with.
    system: Box<dyn System + Send + Sync>,
}

/// What the window's indexing is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Indexing {
    Idle,
    /// Files read so far, of those this update reads.
    Running {
        done: usize,
        total: usize,
    },
    /// Held while the machine runs on its battery.
    Held,
    /// Another nooma - the CLI, most likely - is writing this library; its
    /// update is this one.
    Busy,
    Failed {
        message: String,
    },
}

/// The window's indexing, as the `index-state` event carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexState {
    #[serde(flatten)]
    pub indexing: Indexing,
    /// Updates finished since the window opened: the page reads the status
    /// and the health again when it moves.
    pub passes: u64,
    /// Whether the folders are watched for changes.
    pub watching: bool,
    /// Counts every change, so the page keeps the newest of a state it was
    /// sent and one it asked for.
    pub seq: u64,
}

/// The updates of the index the window runs: asked for from anywhere, run
/// on one thread, one at a time.
struct Indexer {
    /// What is asked for and not yet running: everything asked meanwhile,
    /// joined.
    asked: Mutex<Option<Scope>>,
    wake: Condvar,
    state: Mutex<IndexState>,
    /// When a whole update was last asked for.
    whole_asked: Mutex<Option<Instant>>,
}

impl Indexer {
    fn new() -> Self {
        Self {
            asked: Mutex::new(None),
            wake: Condvar::new(),
            state: Mutex::new(IndexState {
                indexing: Indexing::Idle,
                passes: 0,
                watching: false,
                seq: 0,
            }),
            whole_asked: Mutex::new(None),
        }
    }

    fn ask(&self, scope: Scope) {
        if scope == Scope::Everything {
            *self.whole_asked.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        }
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        *asked = Some(match asked.take() {
            Some(before) => before.and(scope),
            None => scope,
        });
        // An update asked for is under way as far as anyone asking is
        // concerned: a window opened a moment before its first update starts
        // is not up to date.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.indexing == Indexing::Idle {
            state.indexing = Indexing::Running { done: 0, total: 0 };
            state.seq += 1;
        }
        self.wake.notify_one();
    }

    /// Ask for a whole update unless one was asked for a moment ago.
    fn freshen(&self) {
        let recent = self
            .whole_asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|asked| asked.elapsed() < FRESHEN_AFTER);
        if !recent {
            self.ask(Scope::Everything);
        }
    }

    /// What to update next, waiting until something is asked for.
    fn next(&self) -> Scope {
        let mut asked = self.asked.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(scope) = asked.take() {
                return scope;
            }
            asked = self.wake.wait(asked).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// What to update next, if anything is asked for: for tests, which run
    /// updates on their own thread.
    #[cfg(test)]
    fn take(&self) -> Option<Scope> {
        self.asked.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn state(&self) -> IndexState {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Change the state, and return it as it is now.
    fn change(&self, change: impl FnOnce(&mut IndexState)) -> IndexState {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut state);
        state.seq += 1;
        state.clone()
    }
}

impl AppState {
    /// The library in the user's data directory, shared with the `nooma` CLI —
    /// or in `NOOMA_STORE`, as the CLI reads it, for a demo or a test that
    /// must not touch the real one. Models likewise, from `NOOMA_MODELS`.
    pub fn new() -> Self {
        let dir = |name: &str| std::env::var_os(name).filter(|dir| !dir.is_empty()).map(PathBuf::from);
        Self::with(dir("NOOMA_STORE"), dir("NOOMA_MODELS"), None)
    }

    /// A library in a given directory, for tests.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self::with(Some(root.into()), None, None)
    }

    /// A library in a given directory searched by a given model, for tests
    /// that need no model on disk.
    pub fn with_model(root: impl Into<PathBuf>, embedder: Kept) -> Self {
        Self::with(Some(root.into()), None, Some(embedder))
    }

    fn with(root: Option<PathBuf>, models: Option<PathBuf>, embedder: Option<Kept>) -> Self {
        Self {
            root,
            models,
            finder: Mutex::new(None),
            embedder: Mutex::new(embedder),
            searching: AtomicUsize::new(0),
            meaning: Mutex::new(None),
            computing: Mutex::new(None),
            again: AtomicBool::new(false),
            fetching: AtomicBool::new(false),
            indexer: Indexer::new(),
            watcher: Mutex::new(None),
            system: Box::new(Os::default()),
        }
    }

    /// The same, with a machine that says what a test tells it to.
    #[cfg(test)]
    fn on(mut self, system: impl System + Send + Sync + 'static) -> Self {
        self.system = Box::new(system);
        self
    }

    fn library(&self) -> Result<Library, String> {
        let library = match &self.root {
            Some(root) => Library::open_at(root),
            None => Library::open(),
        };
        library.map_err(|e| e.to_string())
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        match &self.models {
            Some(dir) => Ok(dir.clone()),
            None => nooma_core::model::default_models_dir().map_err(|e| e.to_string()),
        }
    }

    /// The model the window searches with: the one loaded, or the catalogue's.
    fn spec(&self) -> &'static ModelSpec {
        ModelSpec::default_model()
    }

    /// Whether there is a model to run: one given, or one on disk.
    fn has_model(&self) -> bool {
        let given = self.embedder.lock().is_ok_and(|kept| kept.is_some());
        given || self.models_dir().is_ok_and(|dir| self.spec().missing(&dir).is_empty())
    }

    /// The model's identity - id, recipe, length - without loading it.
    fn model(&self) -> Described {
        match self.embedder.lock().ok().as_ref().and_then(|kept| kept.as_ref()) {
            Some(embedder) => Described::of(embedder.as_ref()),
            None => Described::of(self.spec()),
        }
    }

    /// Run something with the model, loading it first if it is on disk and
    /// not loaded yet; `None` when there is no model.
    fn with_embedder<T>(&self, run: impl FnOnce(&mut dyn Embedder) -> Result<T, String>) -> Result<Option<T>, String> {
        let mut held = self.embedder.lock().map_err(|_| "the model is poisoned".to_string())?;
        if held.is_none() {
            let dir = self.models_dir()?;
            if !self.spec().missing(&dir).is_empty() {
                return Ok(None);
            }
            let loaded = OnnxEmbedder::load(self.spec(), &dir, indexing_threads()).map_err(|e| e.to_string())?;
            *held = Some(Box::new(loaded));
        }
        let embedder = held.as_mut().expect("loaded above");
        run(embedder.as_mut()).map(Some)
    }

    /// The kept vector index, read again if an update replaced it.
    fn meaning_index(&self, library: &Library) -> Result<MutexGuard<'_, Option<SemanticIndex>>, String> {
        let mut kept = self.meaning.lock().map_err(|_| "the vector index is poisoned".to_string())?;
        if kept.as_ref().is_none_or(|index| !index.is_current()) {
            *kept = library.semantic(&self.model()).map_err(|e| e.to_string())?;
        }
        Ok(kept)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// One search counted in [`AppState::searching`], until it is dropped -
/// returned or failed.
struct Searching<'a>(&'a AtomicUsize);

impl<'a> Searching<'a> {
    fn count(searching: &'a AtomicUsize) -> Self {
        searching.fetch_add(1, Ordering::AcqRel);
        Self(searching)
    }
}

impl Drop for Searching<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A model's identity, copied out so the index can be opened while the
/// model itself is busy computing vectors.
#[derive(Debug, Clone)]
struct Described {
    id: String,
    recipe: String,
    dimensions: usize,
    /// The directions its vectors are compared without: the graph the
    /// window builds and searches is in the space the command line's is.
    language: Option<LanguageAxes>,
}

impl Described {
    fn of(model: &dyn Model) -> Self {
        Self {
            id: model.model_id().to_string(),
            recipe: model.recipe(),
            dimensions: model.dimensions(),
            language: model.language().cloned(),
        }
    }
}

impl Model for Described {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn recipe(&self) -> String {
        self.recipe.clone()
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn language(&self) -> Option<&LanguageAxes> {
        self.language.as_ref()
    }
}

/// The kept model as the background update sees it: borrowed one batch at a
/// time and given back between, when a waiting search takes it.
struct Shared<'a> {
    state: &'a AppState,
    model: Described,
}

impl Model for Shared<'_> {
    fn model_id(&self) -> &str {
        self.model.model_id()
    }

    fn recipe(&self) -> String {
        self.model.recipe()
    }

    fn dimensions(&self) -> usize {
        self.model.dimensions()
    }

    fn language(&self) -> Option<&LanguageAxes> {
        self.model.language()
    }
}

impl Embedder for Shared<'_> {
    fn embed(&mut self, texts: &[&str], role: Role) -> nooma_core::Result<Vec<Vec<f32>>> {
        let asked = Instant::now();
        while self.state.searching.load(Ordering::Acquire) > 0 && asked.elapsed() < YIELD_LIMIT {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.state
            .with_embedder(|embedder| embedder.embed(texts, role).map_err(|e| e.to_string()))
            .map_err(nooma_core::Error::Embedding)?
            .ok_or_else(|| nooma_core::Error::Embedding("the model is gone".to_string()))
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

/// The documents that hold the words of a query, ranked as the whole answer
/// ranks them: what the window shows until the meaning has answered too.
///
/// Answers from the stored index: the window keeps it current by updating
/// when it opens, and a search on every keystroke must not walk the disk.
#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Result<Found, String> {
    search_in(&state, &query, limit)
}

/// What a search returns.
#[derive(Debug, Serialize)]
pub struct Found {
    /// The documents, best first, one per document.
    pub hits: Vec<Ranked>,
    /// Whether the hits the words found hold every word of the query.
    pub all_words: bool,
    /// Whether the meaning answered too; `false` when the words answered
    /// alone.
    pub by_meaning: bool,
    /// Documents the vector index does not cover yet.
    pub behind: usize,
    /// Milliseconds from the question to the answer, the model included.
    pub took_ms: u64,
}

impl Found {
    fn of(answer: Answer, by_meaning: bool, behind: usize, started: Instant) -> Self {
        Self {
            hits: answer.hits,
            all_words: answer.all_words,
            by_meaning,
            behind,
            took_ms: millis(started),
        }
    }
}

fn millis(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The kept full-text finder, opened if it is not; `None` before the first
/// update.
fn finder_of(state: &AppState) -> Result<MutexGuard<'_, Option<Finder>>, String> {
    let mut kept = state.finder.lock().map_err(|_| "the search state is poisoned".to_string())?;
    if kept.is_none() {
        *kept = state.library()?.finder().map_err(|e| e.to_string())?;
    }
    Ok(kept)
}

pub fn search_in(state: &AppState, query: &str, limit: Option<usize>) -> Result<Found, String> {
    let started = Instant::now();
    let limit = limit.unwrap_or(MAX_RESULTS).min(MAX_RESULTS);
    let kept = finder_of(state)?;
    let answer = match kept.as_ref() {
        Some(finder) => hybrid::search(finder, None, query, limit).map_err(|e| e.to_string())?,
        // Nothing has been indexed yet; the next search tries again.
        None => Answer::default(),
    };
    Ok(Found::of(answer, false, 0, started))
}

/// The whole answer to a query: the words and the meaning, ranked as one
/// list; `None` when there is no model or no vectors yet, and the words'
/// answer is the whole one.
#[tauri::command]
pub async fn search_hybrid(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Result<Option<Found>, String> {
    meaning_in(&state, &query, Role::Query, limit)
}

/// The documents that say what a pasted passage says.
#[tauri::command]
pub async fn similar(state: State<'_, AppState>, text: String, limit: Option<usize>) -> Result<Option<Found>, String> {
    // A passage is compared with passages: read as one, not as a question.
    meaning_in(&state, &text, Role::Passage, limit)
}

pub fn meaning_in(state: &AppState, text: &str, role: Role, limit: Option<usize>) -> Result<Option<Found>, String> {
    // Counted from the first line: asking whether there is a model, and
    // which, takes the model's lock as much as reading the query does.
    let _counted = Searching::count(&state.searching);
    let started = Instant::now();
    let limit = limit.unwrap_or(MAX_RESULTS).min(MAX_RESULTS);
    if text.trim().is_empty() || !state.has_model() {
        return Ok(None);
    }
    let library = state.library()?;
    // The index is checked before the model is touched: with no vectors
    // there is nothing to wait for the model for.
    if state.meaning_index(&library)?.is_none() {
        return Ok(None);
    }
    let Some(vector) = state.with_embedder(|embedder| {
        let mut vectors = embedder.embed(&[text], role).map_err(|e| e.to_string())?;
        Ok(vectors.pop().unwrap_or_default())
    })?
    else {
        return Ok(None);
    };
    let kept = state.meaning_index(&library)?;
    let Some(index) = kept.as_ref() else { return Ok(None) };
    let finder = finder_of(state)?;
    let Some(finder) = finder.as_ref() else { return Ok(None) };
    let answer = match role {
        Role::Query => hybrid::search(finder, Some((index, &vector)), text, limit),
        Role::Passage => hybrid::similar(finder, index, text, &vector, limit, None),
    }
    .map_err(|e| e.to_string())?;
    Ok(Some(Found::of(answer, true, index.behind(), started)))
}

/// The sentence of a row's chunk that answered: the fragment to show
/// instead of the chunk's opening, and where the sentence is in it.
#[derive(Debug, Serialize)]
pub struct Explained {
    pub fragment: String,
    /// Byte offsets into the UTF-8 fragment, as highlights are.
    pub closest: [usize; 2],
}

/// Why a row the meaning found in another language was found: the sentence
/// of its chunk closest to the query - or to the pasted passage, when
/// `passage` is set. `None` when there is no model, no vectors, or no such
/// chunk any more.
#[tauri::command]
pub async fn explain(state: State<'_, AppState>, text: String, passage: bool, path: PathBuf, line: u32) -> Result<Option<Explained>, String> {
    let role = if passage { Role::Passage } else { Role::Query };
    explain_in(&state, &text, role, &path, line)
}

pub fn explain_in(state: &AppState, text: &str, role: Role, path: &Path, line: u32) -> Result<Option<Explained>, String> {
    // A search like any other, for the background update: it waits for the
    // model as one does.
    let _counted = Searching::count(&state.searching);
    if text.trim().is_empty() || !state.has_model() {
        return Ok(None);
    }
    let library = state.library()?;
    // Held in the order a search holds them: the vector index, then the
    // finder, then the model.
    let kept = state.meaning_index(&library)?;
    let Some(index) = kept.as_ref() else { return Ok(None) };
    let finder = finder_of(state)?;
    let Some(finder) = finder.as_ref() else { return Ok(None) };
    let Some(mut hit) = finder.chunks(&[(path.to_path_buf(), line)]).map_err(|e| e.to_string())?.pop().flatten() else {
        return Ok(None);
    };
    let told = state.with_embedder(|embedder| {
        let vector = embedder.embed(&[text], role).map_err(|e| e.to_string())?.pop().unwrap_or_default();
        why::explain(finder, index, embedder, &vector, &mut hit).map_err(|e| e.to_string())
    })?;
    Ok(match (told, hit.closest) {
        (Some(true), Some(closest)) => Some(Explained {
            fragment: hit.fragment,
            closest: [closest.start, closest.end],
        }),
        _ => None,
    })
}

/// Progress of an update, as the `index-progress` and `vectors-progress`
/// events carry it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProgressEvent {
    /// Files read so far, or passages embedded.
    pub done: usize,
    /// Files this update reads, or passages it embeds.
    pub total: usize,
}

impl From<Progress> for ProgressEvent {
    fn from(progress: Progress) -> Self {
        Self {
            done: progress.done,
            total: progress.total,
        }
    }
}

/// What the window's indexing is doing now: the page asks once, when it has
/// begun to listen to `index-state`.
#[tauri::command]
pub fn index_state(state: State<'_, AppState>) -> IndexState {
    state.indexer.state()
}

/// What the index lacks of the files under the folders, and what it holds
/// of them only in part.
#[tauri::command]
pub fn health(state: State<'_, AppState>) -> Result<Health, String> {
    health_of(&state)
}

pub fn health_of(state: &AppState) -> Result<Health, String> {
    state.library()?.health().map_err(|e| e.to_string())
}

/// Start keeping the index up to date: the thread that runs every update,
/// and the first, whole one, when there are folders to read.
pub fn start(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.library().is_ok_and(|library| !library.sources().is_empty()) {
        state.indexer.ask(Scope::Everything);
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("nooma-index".to_string())
        .spawn(move || {
            background::lower_this_thread();
            let state = app.state::<AppState>();
            loop {
                let scope = state.indexer.next();
                let tell = |changed: &IndexState| {
                    let _ = app.emit("index-state", changed);
                };
                let result = pass_in(&state, &scope, &tell);
                let ok = result.is_ok();
                if ok && scope == Scope::Everything {
                    let asking = app.clone();
                    let watching = watch_in(&state, move |changed| asking.state::<AppState>().indexer.ask(changed));
                    tell(&state.indexer.change(|now| now.watching = watching));
                }
                // The vectors follow the chunks: after the first update, in
                // case they were left behind, and after any that moved them.
                let first = state.indexer.state().passes == 1;
                if let Ok(report) = result
                    && (first || report.indexed > 0 || report.removed > 0)
                {
                    start_vectors(&app);
                }
            }
        })
        .expect("the indexing thread starts");
}

/// The window came into focus: catch up with what a watcher may have missed,
/// unless a whole update was asked for a moment ago.
pub fn focused(state: &AppState) {
    if state.library().is_ok_and(|library| !library.sources().is_empty()) {
        state.indexer.freshen();
    }
}

/// Watch the library's folders, asking for an update over what changes;
/// whether any folder is watched.
pub fn watch_in(state: &AppState, ask: impl Fn(Scope) + Send + 'static) -> bool {
    let Ok(library) = state.library() else { return false };
    let mut kept = state.watcher.lock().unwrap_or_else(PoisonError::into_inner);
    // The old one goes first: two would name every change twice.
    *kept = None;
    let Ok(watcher) = library.watch(ask) else { return false };
    let watching = watcher.unwatched().len() < library.sources().len();
    *kept = Some(watcher);
    watching
}

/// One update of the index, at the window's pace, telling how it goes.
pub fn pass_in(state: &AppState, scope: &Scope, tell: &(dyn Fn(&IndexState) + Sync)) -> Result<UpdateReport, String> {
    tell(&state.indexer.change(|now| now.indexing = Indexing::Running { done: 0, total: 0 }));
    let running = |progress: Progress| {
        tell(&state.indexer.change(|now| {
            now.indexing = Indexing::Running {
                done: progress.done,
                total: progress.total,
            };
        }));
    };
    let progress = throttled(&running);
    let held = || tell(&state.indexer.change(|now| now.indexing = Indexing::Held));
    let hold = || background::hold(&held);
    let result = update_in(state, scope, Pace::background(&progress, &hold));
    tell(&state.indexer.change(|now| {
        now.passes += 1;
        now.indexing = match &result {
            Ok(_) => Indexing::Idle,
            Err(error) if error == "busy" => Indexing::Busy,
            Err(error) => Indexing::Failed { message: error.clone() },
        };
    }));
    result
}

/// A progress callback that passes on a hundredth of the work, the first and
/// the last: a bar redrawn per file costs more than the files on a fast disk.
fn throttled(emit: &(dyn Fn(Progress) + Sync)) -> impl Fn(Progress) + Sync + '_ {
    let last = AtomicUsize::new(0);
    move |progress: Progress| {
        let step = (progress.total / 100).max(1);
        let previous = last.load(Ordering::Relaxed);
        if progress.done == 0 || progress.done == progress.total || progress.done >= previous + step {
            last.store(progress.done, Ordering::Relaxed);
            emit(progress);
        }
    }
}

pub fn update_in(state: &AppState, scope: &Scope, pace: Pace<'_>) -> Result<UpdateReport, String> {
    let library = state.library()?;
    let result = library.update_scoped(scope, pace);
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

/// How an update of the vectors ended, as the `vectors-done` event carries it.
#[derive(Debug, Clone, Serialize)]
pub struct VectorsDone {
    /// What it did, when it finished.
    pub report: Option<VectorReport>,
    /// Why it stopped, when it did not; `busy` when another nooma is
    /// computing the same vectors.
    pub error: Option<String>,
}

/// Compute the vectors the library lacks, in the background.
///
/// Returns at once: progress comes as `vectors-progress`, the end as
/// `vectors-done`. Asked again while it runs, it runs once more after,
/// since the library may have changed under the running one.
#[tauri::command]
pub fn update_vectors(app: AppHandle) -> bool {
    start_vectors(&app)
}

/// See [`update_vectors`].
pub fn start_vectors(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    if !state.has_model() {
        return false;
    }
    if !begin_vectors(&state) {
        state.again.store(true, Ordering::Release);
        return true;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        background::lower_this_thread();
        let state = app.state::<AppState>();
        loop {
            state.again.store(false, Ordering::Release);
            let held = || {
                let _ = app.emit("vectors-held", ());
            };
            let result = vectors_in(
                &state,
                &|progress| {
                    let _ = app.emit("vectors-progress", ProgressEvent::from(progress));
                },
                &|| background::hold(&held),
            );
            let done = match result {
                Ok(report) => VectorsDone {
                    report: Some(report),
                    error: None,
                },
                Err(error) => VectorsDone {
                    report: None,
                    error: Some(error),
                },
            };
            let failed = done.error.is_some();
            let _ = app.emit("vectors-done", done);
            if failed || !state.again.load(Ordering::Acquire) {
                break;
            }
        }
        end_vectors(&state);
    });
    true
}

/// Claim the one update of the vectors a window runs; `false` when one runs.
fn begin_vectors(state: &AppState) -> bool {
    let Ok(mut computing) = state.computing.lock() else { return false };
    if computing.is_some() {
        return false;
    }
    *computing = Some(Progress { done: 0, total: 0 });
    true
}

fn end_vectors(state: &AppState) {
    if let Ok(mut computing) = state.computing.lock() {
        *computing = None;
    }
}

/// Bring the vectors up to date with the model the window keeps, holding
/// before each batch as `hold` says.
pub fn vectors_in(state: &AppState, emit: &(dyn Fn(Progress) + Sync), hold: &(dyn Fn() + Sync)) -> Result<VectorReport, String> {
    let library = state.library()?;
    // Loaded here, off the page's thread, if no search has loaded it yet.
    let Some(model) = state.with_embedder(|embedder| Ok(Described::of(&*embedder)))? else {
        return Err("the model is not on this machine".to_string());
    };
    let mut shared = Shared { state, model };
    let record = |progress: Progress| {
        if let Ok(mut computing) = state.computing.lock()
            && computing.is_some()
        {
            *computing = Some(progress);
        }
    };
    let throttled = throttled(emit);
    let progress = |progress: Progress| {
        record(progress);
        throttled(progress);
    };
    let result = library.update_vectors_paced(&mut shared, Pace::background(&progress, hold));
    match result {
        Ok(report) => Ok(report),
        Err(nooma_core::Error::Busy) => Err("busy".to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// What the window knows about search by meaning.
#[derive(Debug, Serialize)]
pub struct MeaningStatus {
    /// The model.
    pub model: &'static str,
    /// Its size on disk once fetched, in bytes.
    pub bytes: u64,
    /// Whether it is on this machine.
    pub present: bool,
    /// Whether it is being fetched.
    pub fetching: bool,
    /// Passages in the vector index; `None` when there is none yet.
    pub passages: Option<usize>,
    /// Documents the index does not cover yet.
    pub behind: usize,
    /// How far a running update of the vectors has got.
    pub computing: Option<ProgressEvent>,
}

#[tauri::command]
pub fn meaning_status(state: State<'_, AppState>) -> Result<MeaningStatus, String> {
    meaning_status_of(&state)
}

pub fn meaning_status_of(state: &AppState) -> Result<MeaningStatus, String> {
    let spec = state.spec();
    let library = state.library()?;
    let (passages, behind) = match state.meaning_index(&library)?.as_ref() {
        Some(index) => (Some(index.len()), index.behind()),
        None => (None, 0),
    };
    Ok(MeaningStatus {
        model: spec.id,
        bytes: spec.bytes(),
        present: state.has_model(),
        fetching: state.fetching.load(Ordering::Acquire),
        passages,
        behind,
        computing: state.computing.lock().ok().and_then(|computing| computing.map(ProgressEvent::from)),
    })
}

/// Progress of fetching the model, as the `model-progress` event carries it.
#[derive(Debug, Clone, Serialize)]
pub struct FetchEvent {
    /// The file being fetched.
    pub file: String,
    /// Bytes of the whole model on disk so far.
    pub done: u64,
    /// The whole model's size.
    pub total: u64,
}

/// Fetch the model - the one thing nooma does over the network, and only
/// when the person asks for it here or with `nooma model fetch`.
///
/// Returns at once: progress comes as `model-progress`, the end as
/// `model-done` carrying the error, if any.
#[tauri::command]
pub fn fetch_model(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    if state.fetching.swap(true, Ordering::AcqRel) {
        return Ok(false);
    }
    let dir = match state.models_dir() {
        Ok(dir) => dir,
        Err(error) => {
            state.fetching.store(false, Ordering::Release);
            return Err(error);
        }
    };
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let spec = state.spec();
        let total = spec.bytes();
        // Bytes of the files finished before the current one.
        let mut before = 0u64;
        let mut current = String::new();
        let mut current_total = 0u64;
        let mut shown = Instant::now() - Duration::from_secs(1);
        let result = nooma_fetch::fetch(spec, &dir, &mut |progress| {
            if progress.file != current {
                before += current_total;
                current = progress.file.to_string();
                current_total = progress.total;
            }
            if shown.elapsed() >= Duration::from_millis(200) || progress.done == progress.total {
                shown = Instant::now();
                let _ = app.emit(
                    "model-progress",
                    FetchEvent {
                        file: progress.file.to_string(),
                        done: (before + progress.done).min(total),
                        total,
                    },
                );
            }
        });
        state.fetching.store(false, Ordering::Release);
        let _ = app.emit("model-done", result.err().map(|error| error.to_string()));
    });
    Ok(true)
}

/// Add a folder to search, and read it: a whole update is asked for, which
/// watches it once done.
#[tauri::command]
pub fn add_source(state: State<'_, AppState>, path: PathBuf) -> Result<Status, String> {
    let status = add_source_in(&state, &path)?;
    state.indexer.ask(Scope::Everything);
    Ok(status)
}

pub fn add_source_in(state: &AppState, path: &Path) -> Result<Status, String> {
    let mut library = state.library()?;
    library.add_source(path, Vec::new()).map_err(|e| e.to_string())?;
    library.status().map_err(|e| e.to_string())
}

/// A found document around the chunk found, and how it would be opened
/// there: what the preview shows.
#[derive(Debug, Serialize)]
pub struct Shown {
    #[serde(flatten)]
    pub preview: Preview,
    pub opens: Opening,
}

/// The chunk starting at a line of a found document, with up to `before`
/// chunks before it and `after` after, the words of `query` marked - none for
/// a pasted passage. `None` when the index does not hold the document, or
/// there is no index yet.
#[tauri::command]
pub async fn preview(
    state: State<'_, AppState>,
    path: PathBuf,
    line: u32,
    query: Option<String>,
    before: usize,
    after: usize,
) -> Result<Option<Shown>, String> {
    preview_in(&state, &path, line, query.as_deref(), before, after)
}

pub fn preview_in(state: &AppState, path: &Path, line: u32, query: Option<&str>, before: usize, after: usize) -> Result<Option<Shown>, String> {
    let preview = {
        let kept = finder_of(state)?;
        let Some(finder) = kept.as_ref() else { return Ok(None) };
        finder
            .preview(path, line, query, before.min(MAX_AROUND), after.min(MAX_AROUND))
            .map_err(|e| e.to_string())?
    };
    // The finder is let go before the system is asked: a search waits for
    // the finder, not for the shell.
    let Some(preview) = preview else { return Ok(None) };
    let opens = opening_of(state, &preview.path, &preview.kind, &preview.passages[preview.found]);
    Ok(Some(Shown { preview, opens }))
}

/// How a document would be opened at a chunk of it.
fn opening_of(state: &AppState, path: &Path, kind: &str, passage: &Passage) -> Opening {
    let place = opening::Place {
        path,
        kind: DocKind::from_name(kind).unwrap_or(DocKind::Text),
        line: passage.line,
        page: passage.page,
        heading: passage.headings.last().map(String::as_str),
    };
    opening::plan(&place, state.system.as_ref())
}

/// How a found document would be opened at the chunk starting at a line of
/// it; `None` when the index does not hold it.
pub fn opening_in(state: &AppState, path: &Path, line: u32) -> Result<Option<Opening>, String> {
    let preview = {
        let kept = finder_of(state)?;
        let Some(finder) = kept.as_ref() else { return Ok(None) };
        finder.preview(path, line, None, 0, 0).map_err(|e| e.to_string())?
    };
    Ok(preview.map(|preview| opening_of(state, &preview.path, &preview.kind, &preview.passages[preview.found])))
}

/// Open a found document - at the chunk starting at `line`, when it is
/// given and the program that opens the document can be told the place.
#[tauri::command]
pub fn open_document(app: AppHandle, state: State<'_, AppState>, path: PathBuf, line: Option<u32>) -> Result<(), String> {
    let path = indexed_path(&state, &path)?;
    let file = |path: &Path| app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string());
    // A place the index no longer knows is not a reason not to open the
    // file: it opens at its start.
    let opening = match line {
        Some(line) => opening_in(&state, &path, line).ok().flatten(),
        None => None,
    };
    match opening {
        Some(opening) => opening.open(&path, |link| app.opener().open_url(link, None::<&str>).map_err(|e| e.to_string()), file),
        None => file(&path),
    }
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

    /// A whole update someone waits for.
    fn updated(state: &AppState) -> UpdateReport {
        update_in(state, &Scope::Everything, Pace::foreground(&|_| {})).unwrap()
    }

    #[test]
    fn a_folder_added_is_searchable_after_an_update() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("a.md"), "# Greenhouse\n\nOpen the vents at noon.\n").unwrap();
        let (_store, state) = library_with(notes.path());
        assert!(search_in(&state, "vents", None).unwrap().hits.is_empty(), "nothing is read before an update");
        let told = Mutex::new(Vec::new());
        let report = pass_in(&state, &Scope::Everything, &|state| told.lock().unwrap().push(state.clone())).unwrap();
        assert_eq!(report.documents, 1);
        assert_eq!(search_in(&state, "vent", None).unwrap().hits[0].hit.title, "Greenhouse");

        let told = told.into_inner().unwrap();
        let kinds: Vec<&Indexing> = told.iter().map(|state| &state.indexing).collect();
        assert_eq!(kinds.first(), Some(&&Indexing::Running { done: 0, total: 0 }));
        assert_eq!(
            kinds[kinds.len() - 2],
            &Indexing::Running { done: 1, total: 1 },
            "the last progress is the whole: {kinds:?}"
        );
        assert_eq!(told.last().map(|state| (&state.indexing, state.passes)), Some((&Indexing::Idle, 1)));
        assert!(told.windows(2).all(|pair| pair[0].seq < pair[1].seq), "every change is counted");
        assert_eq!(state.indexer.state(), *told.last().unwrap());
    }

    #[test]
    fn what_is_asked_while_an_update_runs_is_one_update_after() {
        let store = tempfile::tempdir().unwrap();
        let state = AppState::at(store.path());
        let paths = |names: &[&str]| Scope::Paths(names.iter().map(PathBuf::from).collect());
        assert_eq!(state.indexer.take(), None);
        assert_eq!(state.indexer.state().indexing, Indexing::Idle);
        state.indexer.ask(paths(&["a.md"]));
        assert_eq!(
            state.indexer.state().indexing,
            Indexing::Running { done: 0, total: 0 },
            "an update asked for is under way"
        );
        state.indexer.ask(paths(&["b.md"]));
        assert_eq!(state.indexer.take(), Some(paths(&["a.md", "b.md"])));
        state.indexer.ask(paths(&["a.md"]));
        state.indexer.ask(Scope::Everything);
        assert_eq!(state.indexer.take(), Some(Scope::Everything));

        // Coming into focus a moment after a whole update asks for nothing;
        // a while after, it asks for another.
        state.indexer.freshen();
        assert_eq!(state.indexer.take(), None);
        *state.indexer.whole_asked.lock().unwrap() = Some(Instant::now() - FRESHEN_AFTER);
        state.indexer.freshen();
        assert_eq!(state.indexer.take(), Some(Scope::Everything));
    }

    #[test]
    fn a_failed_update_says_why_and_counts_as_one() {
        let store = tempfile::tempdir().unwrap();
        // A library whose sources file is not one cannot be opened.
        std::fs::write(store.path().join("sources.json"), "not json").unwrap();
        let state = AppState::at(store.path());
        assert!(pass_in(&state, &Scope::Everything, &|_| {}).is_err());
        let now = state.indexer.state();
        assert!(matches!(now.indexing, Indexing::Failed { .. }), "{now:?}");
        assert_eq!(now.passes, 1);
    }

    /// The watcher asks the indexer, and the update over what it named finds
    /// the note saved: the window's path, end to end, without the window.
    #[test]
    fn a_note_saved_in_a_watched_folder_is_found_after_the_update_it_asks_for() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("a.md"), "# Greenhouse\n\nOpen the vents at noon.\n").unwrap();
        let (_store, state) = library_with(notes.path());
        updated(&state);
        let state = std::sync::Arc::new(state);
        let asking = std::sync::Arc::clone(&state);
        assert!(watch_in(&state, move |scope| asking.indexer.ask(scope)));
        std::thread::sleep(Duration::from_millis(500));

        let roses = notes.path().join("roses.md");
        std::fs::write(&roses, "# Roses\n\nPrune the roses in February.\n").unwrap();
        let asked = Instant::now();
        let scope = loop {
            match state.indexer.take() {
                Some(Scope::Paths(paths)) if paths.contains(&roses) => break Scope::Paths(paths),
                Some(Scope::Everything) => panic!("the watcher lost count of a single save"),
                _ => {}
            }
            assert!(asked.elapsed() < Duration::from_secs(20), "the watcher asked for nothing");
            std::thread::sleep(Duration::from_millis(50));
        };
        let report = pass_in(&state, &scope, &|_| {}).unwrap();
        assert_eq!((report.indexed, report.documents), (1, 2));
        assert_eq!(search_in(&state, "prune roses", None).unwrap().hits[0].hit.title, "Roses");
        // Watched anew, the old watcher is gone: one save is asked for once.
        let asking = std::sync::Arc::clone(&state);
        assert!(watch_in(&state, move |scope| asking.indexer.ask(scope)));
        *state.watcher.lock().unwrap() = None;
    }

    #[test]
    fn the_health_is_what_the_library_says() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("damaged.pdf"), "%PDF-1.7\nnot really").unwrap();
        let (_store, state) = library_with(notes.path());
        updated(&state);
        let health = health_of(&state).unwrap();
        assert_eq!(health.skipped.len(), 1);
        assert_eq!(health.skipped[0].why, nooma_core::Why::Damaged);
    }

    /// A machine whose shell opens every file with one program.
    struct OpensWith(&'static str);

    impl System for OpensWith {
        fn program_for(&self, _path: &Path, _kind: DocKind) -> Option<opening::Program> {
            Some(opening::Program {
                executable: PathBuf::from(self.0),
                name: "Code".to_string(),
            })
        }

        fn vaults(&self) -> Vec<(String, PathBuf)> {
            Vec::new()
        }
    }

    #[test]
    fn the_selected_row_is_shown_among_its_neighbours_with_where_it_opens() {
        let notes = tempfile::tempdir().unwrap();
        let note = "# Greenhouse

## Vents

Open the vents at noon.

## Watering

Water the tomatoes in the evening.

## Heating

The heater goes on at night.
";
        std::fs::write(notes.path().join("greenhouse.md"), note).unwrap();
        let store = tempfile::tempdir().unwrap();
        let state = AppState::at(store.path()).on(OpensWith("/apps/Code.exe"));
        add_source_in(&state, notes.path()).unwrap();
        let path = notes.path().join("greenhouse.md");
        assert!(preview_in(&state, &path, 1, None, 1, 1).unwrap().is_none(), "nothing before the first update");
        updated(&state);

        let hit = search_in(&state, "tomatoes", None).unwrap().hits.remove(0).hit;
        let shown = preview_in(&state, &hit.path, hit.line, Some("tomatoes"), 1, 1).unwrap().unwrap();
        let found = &shown.preview.passages[shown.preview.found];
        assert_eq!((found.line, found.highlights.len()), (hit.line, 1));
        assert_eq!(shown.preview.passages.len(), 3, "the vents and the heater beside it");
        assert_eq!(shown.opens.program.as_deref(), Some("Code"));
        assert_eq!(shown.opens.at, Some(opening::At::Line { line: hit.line }));

        // As the page reads it: the preview and how it opens, side by side,
        // and nothing of what would be run.
        let json = serde_json::to_value(&shown).unwrap();
        assert_eq!(json["found"], 1);
        assert_eq!(json["opens"], serde_json::json!({"program": "Code", "at": {"kind": "line", "line": hit.line}}));

        assert_eq!(opening_in(&state, &hit.path, hit.line).unwrap(), Some(shown.opens));
        assert!(preview_in(&state, &notes.path().join("absent.md"), 1, None, 1, 1).unwrap().is_none());
        assert_eq!(opening_in(&state, &notes.path().join("absent.md"), 1).unwrap(), None);
    }

    #[test]
    fn a_preview_reads_on_no_further_than_it_may() {
        let notes = tempfile::tempdir().unwrap();
        let long: String = (0..(MAX_AROUND + 5))
            .map(|i| {
                format!(
                    "## Part {i}

Words of part {i}.

"
                )
            })
            .collect();
        std::fs::write(notes.path().join("long.md"), long).unwrap();
        let (_store, state) = library_with(notes.path());
        updated(&state);
        let shown = preview_in(&state, &notes.path().join("long.md"), 1, None, 0, usize::MAX).unwrap().unwrap();
        assert_eq!(shown.preview.passages.len(), MAX_AROUND + 1);
        assert_eq!(shown.preview.after, 4);
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
        updated(&state);
        assert_eq!(search_in(&state, "shared", Some(1000)).unwrap().hits.len(), MAX_RESULTS);
    }

    /// Words hashed into buckets: a stand-in model that needs nothing on
    /// disk, and reads a batch of passages slowly on purpose when asked to -
    /// a question it reads at once, as the real model nearly does.
    struct Hashing {
        delay: Duration,
    }

    impl Model for Hashing {
        fn model_id(&self) -> &str {
            "hash"
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
                std::thread::sleep(self.delay);
            }
            Ok(texts
                .iter()
                .map(|text| {
                    let mut vector = vec![0.0f32; 64];
                    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
                        vector[bucket(&word.to_lowercase())] += 1.0;
                    }
                    nooma_core::embed::normalize(&mut vector);
                    vector
                })
                .collect())
        }
    }

    fn bucket(word: &str) -> usize {
        // FNV-1a: enough to spread words over buckets without a dependency.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in word.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
        (hash % 64) as usize
    }

    fn library_with_model(notes: &Path, delay: Duration) -> (tempfile::TempDir, AppState) {
        let store = tempfile::tempdir().unwrap();
        let state = AppState::with_model(store.path(), Box::new(Hashing { delay }));
        add_source_in(&state, notes).unwrap();
        updated(&state);
        (store, state)
    }

    #[test]
    fn search_by_meaning_answers_once_the_vectors_are_computed() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        std::fs::write(notes.path().join("bike.md"), "# Bicycle\n\nOil the chain after rain.\n").unwrap();
        let (_store, state) = library_with_model(notes.path(), Duration::ZERO);

        assert!(meaning_in(&state, "descale kettle", Role::Query, None).unwrap().is_none(), "no vectors yet");
        assert_eq!(meaning_status_of(&state).unwrap().passages, None);

        let report = vectors_in(&state, &|_| {}, &|| {}).unwrap();
        assert_eq!(report.embedded, 2);
        let found = meaning_in(&state, "descale kettle", Role::Query, None).unwrap().unwrap();
        assert_eq!(found.hits[0].hit.title, "Kettle");
        assert_eq!(found.behind, 0);
        assert!(found.by_meaning && found.all_words);
        let similar = meaning_in(&state, "Oil the chain after a ride in the rain", Role::Passage, Some(1))
            .unwrap()
            .unwrap();
        assert_eq!(similar.hits.len(), 1);
        assert_eq!(similar.hits[0].hit.title, "Bicycle");
        assert!(similar.hits[0].words.is_none(), "a passage is not asked of the words");

        let status = meaning_status_of(&state).unwrap();
        assert_eq!((status.passages, status.behind), (Some(2), 0));
        assert!(status.computing.is_none());
    }

    /// The words answer first and alone; the whole answer is the same
    /// documents and more, one row each, and says which half found what.
    #[test]
    fn the_whole_answer_is_one_list_from_both_halves() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        std::fs::write(notes.path().join("bike.md"), "# Bicycle\n\nOil the chain after rain.\n").unwrap();
        let (_store, state) = library_with_model(notes.path(), Duration::ZERO);
        vectors_in(&state, &|_| {}, &|| {}).unwrap();

        let words = search_in(&state, "descale kettle", None).unwrap();
        assert!(!words.by_meaning);
        assert_eq!(words.hits.len(), 1, "only the kettle holds the words");
        assert!(words.hits[0].meaning.is_none());

        let whole = meaning_in(&state, "descale kettle", Role::Query, None).unwrap().unwrap();
        let titles: Vec<&str> = whole.hits.iter().map(|ranked| ranked.hit.title.as_str()).collect();
        assert_eq!(titles, ["Kettle", "Bicycle"], "each document once, the one both halves found first");
        let kettle = &whole.hits[0];
        assert_eq!((kettle.words.map(|p| p.rank), kettle.meaning.map(|p| p.rank)), (Some(1), Some(1)));
        assert!(whole.hits[1].words.is_none());
    }

    /// Asked why a row was found, the window gets the sentence of its chunk
    /// closest to the question, marked, in place of the chunk's opening.
    #[test]
    fn a_row_is_told_the_sentence_that_answered() {
        let notes = tempfile::tempdir().unwrap();
        let kitchen = notes.path().join("kitchen.md");
        std::fs::write(
            &kitchen,
            "# Kitchen

The espresso grinder sits on the left. Wipe the drip tray on Fridays. Descale the kettle with citric acid.
",
        )
        .unwrap();
        let (_store, state) = library_with_model(notes.path(), Duration::ZERO);
        assert!(
            explain_in(&state, "descale kettle", Role::Query, &kitchen, 3).unwrap().is_none(),
            "no vectors yet"
        );
        vectors_in(&state, &|_| {}, &|| {}).unwrap();

        let path = indexed_path(&state, &kitchen).unwrap();
        let told = explain_in(&state, "descale kettle citric", Role::Query, &path, 3).unwrap().expect("a sentence");
        assert_eq!(&told.fragment[told.closest[0]..told.closest[1]], "Descale the kettle with citric acid.");
        assert!(
            explain_in(&state, "descale kettle", Role::Query, &notes.path().join("gone.md"), 1)
                .unwrap()
                .is_none()
        );
    }

    /// A search counted as waiting holds the background update off the model
    /// until it is done: the hand-over does not rest on the mutex being fair.
    #[test]
    fn the_background_update_steps_aside_for_a_waiting_search() {
        let notes = tempfile::tempdir().unwrap();
        let (_store, state) = library_with_model(notes.path(), Duration::ZERO);
        let model = Described {
            id: "hash".to_string(),
            recipe: "hash".to_string(),
            dimensions: 64,
            language: None,
        };
        state.searching.fetch_add(1, Ordering::AcqRel);
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let background = scope.spawn(|| {
                let mut shared = Shared { state: &state, model };
                shared.embed(&["a passage"], Role::Passage).unwrap();
                done.store(true, Ordering::Release);
            });
            std::thread::sleep(Duration::from_millis(300));
            assert!(!done.load(Ordering::Acquire), "the update took the model while a search waited");
            state.searching.fetch_sub(1, Ordering::AcqRel);
            background.join().unwrap();
        });
        assert!(done.load(Ordering::Acquire));
    }

    /// And a search is counted from the moment it asks for the model until
    /// it is done with it - including while it waits.
    #[test]
    fn a_search_is_counted_while_it_waits_for_the_model() {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        let (_store, state) = library_with_model(notes.path(), Duration::ZERO);
        vectors_in(&state, &|_| {}, &|| {}).unwrap();
        let held = state.embedder.lock().unwrap();
        std::thread::scope(|scope| {
            let search = scope.spawn(|| meaning_in(&state, "descale kettle", Role::Query, None).unwrap());
            let asked = Instant::now();
            while state.searching.load(Ordering::Acquire) == 0 && asked.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(state.searching.load(Ordering::Acquire), 1, "a search waiting for the model is not counted");
            drop(held);
            assert!(search.join().unwrap().is_some());
        });
        assert_eq!(state.searching.load(Ordering::Acquire), 0, "a search done is not counted");
    }

    /// The background update holds the model one batch at a time, so a
    /// search waiting for it waits about a batch, never the whole update.
    #[test]
    fn a_search_waits_for_one_batch_of_the_background_update_not_all_of_it() {
        let notes = tempfile::tempdir().unwrap();
        for i in 0..80 {
            std::fs::write(
                notes.path().join(format!("note-{i:02}.md")),
                format!("# Note {i}\n\nNote number {i} about kettles.\n"),
            )
            .unwrap();
        }
        std::fs::write(notes.path().join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        // 81 passages in batches of 16: six batches of 300 ms, about two
        // seconds in all.
        let (_store, state) = library_with_model(notes.path(), Duration::from_millis(300));
        vectors_in(&state, &|_| {}, &|| {}).unwrap();
        // Then 200 more: thirteen batches, about four seconds of reading in
        // the background, against the one batch of 300 ms a search may have
        // to wait out.
        for i in 0..200 {
            std::fs::write(notes.path().join(format!("more-{i:03}.md")), format!("# More {i}\n\nAnother note, {i}.\n")).unwrap();
        }
        updated(&state);

        std::thread::scope(|scope| {
            let background = scope.spawn(|| vectors_in(&state, &|_| {}, &|| {}).unwrap());
            std::thread::sleep(Duration::from_millis(400));
            assert!(state.computing.lock().unwrap().is_none(), "vectors_in alone does not claim the window's update");
            let asked = Instant::now();
            let found = meaning_in(&state, "descale kettle", Role::Query, None).unwrap().unwrap();
            let waited = asked.elapsed();
            // A batch and change on a loaded machine; holding the model for
            // the whole update would make it the seconds left of it.
            assert!(waited < Duration::from_millis(1500), "the search waited {waited:?}");
            assert!(!background.is_finished(), "the update was still running when the search answered");
            assert_eq!(found.hits[0].hit.title, "Kettle");
            background.join().unwrap();
        });
    }
}
