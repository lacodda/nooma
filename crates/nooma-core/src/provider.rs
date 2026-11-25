//! Full text for a neighbour: one folder of notes, asked by its words and by
//! its meaning.
//!
//! An editor keeps its own index of what its notes are - names, headings,
//! links, tags - and has no reason to keep a second one of what they say.
//! This is the seam it asks through. [`FullText`] is the question: bring
//! yourself up to date, answer a query, say how you stand. [`Folder`] is
//! nooma's answer to it - the library, the two halves and the one list of
//! [`crate::hybrid`], for one folder with a store of its own. A search that
//! reads the files line by line is another answer to the same question, and
//! the one a caller falls back on; so the question is a trait, and the caller
//! holds whichever provider it has.
//!
//! # Words always, meaning when there is a model
//!
//! Without the `semantic` feature, or without a model, a [`Folder`] answers
//! by words alone, ranked as the whole answer ranks them. Given a model it
//! answers by both. The model is never fetched here: this crate opens no
//! connection, and a model on disk was put there by `nooma model fetch`.
//!
//! # Nobody waits for the model twice
//!
//! Computing the vectors of a large folder for the first time takes an hour,
//! and a search asked meanwhile needs the same model. The update borrows it a
//! batch at a time and steps aside for a search that is waiting, so a search
//! waits for one batch at most.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::error::{Error, Result};
use crate::hybrid::{self, Answer};
use crate::library::{Finder, Hit, Library, Progress, Source, UpdateReport};

/// Something that searches the text of a folder of notes.
pub trait FullText {
    /// Bring what is known of the folder up to date with its files: only the
    /// files that changed are read.
    ///
    /// Fails with [`Error::Busy`] when another process is updating the same
    /// store; what is stored still answers.
    fn update(&self) -> Result<Updated>;

    /// The documents that answer a query, best first, one row per document,
    /// at most `limit`. From what is stored: a search does not read the
    /// folder, [`FullText::update`] does.
    fn search(&self, query: &str, limit: usize) -> Result<Answer>;

    /// Point a row the meaning found at the sentence of its chunk that
    /// answered: see [`crate::why`]. Returns whether it did. Asked per row
    /// and apart from the search, since it costs a reading of the chunk's
    /// sentences; a provider that cannot say leaves the row as it was.
    fn explain(&self, _query: &str, _hit: &mut Hit) -> Result<bool> {
        Ok(false)
    }

    /// What it holds and what it can answer by, without changing anything.
    fn standing(&self) -> Result<Standing>;
}

/// What an update did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Updated {
    /// What was read of the files.
    pub files: UpdateReport,
    /// Passages the model read; none when the provider answers by words
    /// alone.
    pub embedded: usize,
}

/// How far an update has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Reading the files that changed.
    Files(Progress),
    /// The model reading the passages it has no vector for.
    Passages(Progress),
}

/// What a provider holds and can answer by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// Documents it can find.
    pub documents: usize,
    /// When an update last changed what it holds, as seconds since the
    /// epoch; `None` before the first.
    pub indexed_at: Option<i64>,
    /// Why nothing can be found until the next update, if so: what is stored
    /// was written by other rules.
    pub stale: Option<String>,
    /// The half by meaning; `None` when the provider answers by words alone.
    pub meaning: Option<Meaning>,
}

/// How the half by meaning stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meaning {
    /// The model that answers.
    pub model: String,
    /// Whether there are vectors to answer from.
    pub ready: bool,
    /// Documents its vectors do not cover yet; a search finds those by
    /// their words.
    pub behind: usize,
}

/// nooma's own provider: one folder, read as the library reads its sources,
/// with a store that is this folder's alone.
pub struct Folder {
    library: Library,
    folder: PathBuf,
    /// Opened once and kept: it sees every update as it is committed.
    finder: Mutex<Option<Finder>>,
    #[cfg(feature = "semantic")]
    reading: Option<meaning::Reading>,
}

impl std::fmt::Debug for Folder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Folder")
            .field("folder", &self.folder)
            .field("store", &self.library.root())
            .finish_non_exhaustive()
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Folder {
    /// The provider for `folder`, keeping what it builds in `store` - a
    /// directory of the caller's, created if it is not there.
    ///
    /// `exclude` leaves paths out, in `.gitignore` syntax relative to the
    /// folder. Nothing is read until the first [`FullText::update`].
    ///
    /// A store answers for one folder. Opened for another, or with other
    /// exclusions, it follows: the documents that no longer belong leave at
    /// the next update.
    pub fn open(folder: &Path, store: &Path, exclude: Vec<String>) -> Result<Self> {
        let mut library = Library::open_at(store)?;
        let folder = std::path::absolute(folder).map_err(|e| Error::io(folder, e))?;
        let wanted = Source { path: folder.clone(), exclude };
        if library.sources() != std::slice::from_ref(&wanted) {
            for source in library.sources().to_vec() {
                library.remove_source(&source.path)?;
            }
            library.add_source(&wanted.path, wanted.exclude)?;
        }
        Ok(Self {
            library,
            folder,
            finder: Mutex::new(None),
            #[cfg(feature = "semantic")]
            reading: None,
        })
    }

    /// The folder it searches, as an absolute path.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The library underneath, for what the trait does not ask: a preview,
    /// the health of the folder's files.
    pub fn library(&self) -> &Library {
        &self.library
    }

    /// [`FullText::update`], telling how far it has got.
    pub fn update_with(&self, progress: &(dyn Fn(Step) + Sync)) -> Result<Updated> {
        let files = self.library.update_with(&|at| progress(Step::Files(at)))?;
        if files.rebuilt.is_some() {
            // An index built again may be another directory than the one
            // the kept finder reads.
            *locked(&self.finder) = None;
        }
        #[cfg(feature = "semantic")]
        let embedded = match &self.reading {
            Some(reading) => reading.update(&self.library, &|at| progress(Step::Passages(at)))?,
            None => 0,
        };
        #[cfg(not(feature = "semantic"))]
        let embedded = 0;
        Ok(Updated { files, embedded })
    }

    fn finder(&self) -> Result<MutexGuard<'_, Option<Finder>>> {
        let mut kept = locked(&self.finder);
        if kept.is_none() {
            *kept = self.library.finder()?;
        }
        Ok(kept)
    }
}

impl FullText for Folder {
    fn update(&self) -> Result<Updated> {
        self.update_with(&|_| {})
    }

    fn search(&self, query: &str, limit: usize) -> Result<Answer> {
        let kept = self.finder()?;
        // Before the first update there is nothing to find, which is an
        // answer and not a failure.
        let Some(finder) = kept.as_ref() else {
            return Ok(Answer::default());
        };
        #[cfg(feature = "semantic")]
        if let Some(reading) = &self.reading {
            return reading.search(&self.library, finder, query, limit);
        }
        hybrid::words(finder, query, limit)
    }

    #[cfg(feature = "semantic")]
    fn explain(&self, query: &str, hit: &mut Hit) -> Result<bool> {
        let kept = self.finder()?;
        match (kept.as_ref(), &self.reading) {
            (Some(finder), Some(reading)) => reading.explain(&self.library, finder, query, hit),
            _ => Ok(false),
        }
    }

    fn standing(&self) -> Result<Standing> {
        let status = self.library.status()?;
        #[cfg(feature = "semantic")]
        let meaning = self.reading.as_ref().map(|reading| reading.standing(&self.library)).transpose()?;
        #[cfg(not(feature = "semantic"))]
        let meaning = None;
        Ok(Standing {
            documents: status.documents,
            indexed_at: status.indexed_at,
            stale: status.stale,
            meaning,
        })
    }
}

#[cfg(feature = "semantic")]
mod meaning {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, MutexGuard};
    use std::time::{Duration, Instant};

    use super::{Folder, Meaning, locked};
    use crate::embed::{Embedder, Model, OnnxEmbedder, Role};
    use crate::error::Result;
    use crate::hybrid::{self, Answer};
    use crate::library::{Finder, Hit, Library, Progress, indexing_threads};
    use crate::model::ModelSpec;
    use crate::neutral::LanguageAxes;
    use crate::vectors::SemanticIndex;
    use crate::why;

    /// How long a batch of the update waits for searches to finish before
    /// it goes on regardless: searches asked without a pause must not stop
    /// the vectors from ever being computed.
    const YIELD_LIMIT: Duration = Duration::from_secs(2);

    impl Folder {
        /// Answer by meaning too, with the model nooma uses, when it is in
        /// `models` - where `nooma model fetch` put it; see
        /// [`crate::model::default_models_dir`]. Without the model there the
        /// provider is returned as it was, and answers by words.
        pub fn with_model(self, models: &std::path::Path) -> Result<Self> {
            let spec = ModelSpec::default_model();
            if !spec.missing(models).is_empty() {
                return Ok(self);
            }
            let embedder = OnnxEmbedder::load(spec, models, indexing_threads())?;
            Ok(self.with_embedder(Box::new(embedder)))
        }

        /// Answer by meaning too, with this embedder.
        pub fn with_embedder(mut self, embedder: Box<dyn Embedder + Send>) -> Self {
            self.reading = Some(Reading {
                model: Described::of(&*embedder),
                embedder: Mutex::new(embedder),
                index: Mutex::new(None),
                searching: AtomicUsize::new(0),
            });
            self
        }
    }

    /// A model's identity, copied out: the index is opened and the vectors'
    /// store named while the model itself is busy reading.
    #[derive(Debug, Clone)]
    struct Described {
        id: String,
        recipe: String,
        dimensions: usize,
        language: Option<LanguageAxes>,
    }

    impl Described {
        fn of(model: &dyn Embedder) -> Self {
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

    /// The half by meaning of a [`Folder`]: the model, shared between the
    /// update and the searches, and the index kept open.
    pub(super) struct Reading {
        model: Described,
        embedder: Mutex<Box<dyn Embedder + Send>>,
        index: Mutex<Option<SemanticIndex>>,
        /// Searches that want the model now.
        searching: AtomicUsize,
    }

    /// One search counted, until it is dropped - returned or failed.
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

    /// The model as the update sees it: borrowed a batch at a time and given
    /// back between, when a waiting search takes it.
    struct Lent<'a>(&'a Reading);

    impl Model for Lent<'_> {
        fn model_id(&self) -> &str {
            self.0.model.model_id()
        }

        fn recipe(&self) -> String {
            self.0.model.recipe()
        }

        fn dimensions(&self) -> usize {
            self.0.model.dimensions()
        }

        fn language(&self) -> Option<&LanguageAxes> {
            self.0.model.language()
        }
    }

    impl Embedder for Lent<'_> {
        fn embed(&mut self, texts: &[&str], role: Role) -> Result<Vec<Vec<f32>>> {
            // A mutex need not be fair: without this the update would take
            // the model again the moment it gave it back, batch after batch,
            // and the search would wait for all of them.
            let asked = Instant::now();
            while self.0.searching.load(Ordering::Acquire) > 0 && asked.elapsed() < YIELD_LIMIT {
                std::thread::sleep(Duration::from_millis(1));
            }
            locked(&self.0.embedder).embed(texts, role)
        }
    }

    impl Reading {
        /// The index, read again when an update has replaced the one kept.
        fn index(&self, library: &Library) -> Result<MutexGuard<'_, Option<SemanticIndex>>> {
            let mut kept = locked(&self.index);
            if kept.as_ref().is_none_or(|index| !index.is_current()) {
                *kept = library.semantic(&self.model)?;
            }
            Ok(kept)
        }

        /// A query as the model reads it. The search is counted before the
        /// model is asked for, so the update sees it waiting.
        fn read(&self, query: &str) -> Result<Vec<f32>> {
            let _counted = Searching::count(&self.searching);
            Ok(locked(&self.embedder).embed(&[query], Role::Query)?.pop().unwrap_or_default())
        }

        pub(super) fn update(&self, library: &Library, progress: &(dyn Fn(Progress) + Sync)) -> Result<usize> {
            let report = library.update_vectors(&mut Lent(self), progress)?;
            Ok(report.embedded)
        }

        pub(super) fn search(&self, library: &Library, finder: &Finder, query: &str, limit: usize) -> Result<Answer> {
            let index = self.index(library)?;
            // No vectors yet: the words answer alone, and the standing says
            // why.
            let Some(index) = index.as_ref() else {
                return hybrid::words(finder, query, limit);
            };
            let vector = self.read(query)?;
            hybrid::search(finder, Some((index, &vector)), query, limit)
        }

        pub(super) fn explain(&self, library: &Library, finder: &Finder, query: &str, hit: &mut Hit) -> Result<bool> {
            let index = self.index(library)?;
            let Some(index) = index.as_ref() else {
                return Ok(false);
            };
            let vector = self.read(query)?;
            let _counted = Searching::count(&self.searching);
            why::explain(finder, index, &mut **locked(&self.embedder), &vector, hit)
        }

        pub(super) fn standing(&self, library: &Library) -> Result<Meaning> {
            let index = self.index(library)?;
            Ok(Meaning {
                model: self.model.id.clone(),
                ready: index.is_some(),
                behind: index.as_ref().map_or(0, SemanticIndex::behind),
            })
        }
    }
}
