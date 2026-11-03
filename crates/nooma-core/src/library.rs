//! A library: the folders a person searches, and the index kept over them.
//!
//! The repository index answers "what is in this repository"; a library
//! answers "where did I write about this". It has sources — folders — and one
//! full-text index over every document under them, kept on disk between
//! runs and brought up to date by reading only what changed.
//!
//! # What is kept, and where
//!
//! Three things under one directory: `sources.json`, the folders; the
//! `fulltext/` index itself; and `manifest.json`, what each indexed file was
//! when it was read — size, modification time, content hash, title and links.
//! The manifest is what makes an update cheap: a file whose size and time are
//! unchanged is not opened at all, and one whose bytes hash the same as before
//! is not parsed again.
//!
//! The manifest is written after the index commits, never before. A crash in
//! between leaves a manifest that is behind the index, and the next update
//! re-reads those files and replaces their chunks — which is harmless. The
//! other order would leave a manifest claiming files the index does not hold,
//! and they would never be read again: search quietly missing documents, the
//! failure this product can least afford.
//!
//! # A source that is not there
//!
//! A folder on a drive that is not plugged in is not a folder whose files were
//! deleted. Its entries are kept as they are and the update says it could not
//! reach it; removing them would make every reconnect a full reindex, and a
//! search in between would not find what it found yesterday for no reason the
//! reader could see.
//!
//! # Only what changed
//!
//! An update looks at every file under every source, or - when a watcher
//! ([`crate::watch`]) saw which paths changed - at those paths alone: the
//! folders above them are walked down to them, so every rule that leaves a
//! file out applies as it does on a whole walk, and the rest of the index is
//! kept as it is. It is one update either way, the same code deciding what
//! a file is; what the watcher says only narrows where it looks. An update
//! that found nothing changed writes nothing: a watcher wakes it for every
//! file saved anywhere in a source, most of them no documents.
//!
//! # What could not be read
//!
//! A file found and not read - too large, under a password, damaged - is
//! kept in the manifest with why, as a read one is with what it holds: it is
//! not tried again until it changes, and the index can say at any time what
//! it lacks ([`Library::health`]), not only in the report of the update that
//! met it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tantivy::collector::{DocSetCollector, TopDocs};
use tantivy::query::{Query, QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, OwnedValue, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::{DocAddress, DocId, Index, IndexWriter, Score, SegmentReader, TantivyDocument, Term};

use crate::document::{self, DocKind, Document};
use crate::error::{Error, Result};
use crate::fulltext::{self, Fields};
use crate::reading::{self, Unread, Why};

/// The format of `sources.json`.
pub const SOURCES_FORMAT_VERSION: u32 = 1;

/// The format of the full-text index and its manifest.
///
/// Bumped when the schema, the analyzer or the manifest's shape changes. A
/// library of another format is not read: the next update throws the index
/// away and builds it again, and says so. Words stemmed by other rules would
/// otherwise be looked up by these ones and quietly not match.
///
/// 2: every chunk carries its file's modification time, and keeps every field
/// it is searched by - both of which ranking reads.
///
/// 3: a chunk of a PDF carries its page, and the manifest says what read
/// each file.
///
/// 4: words are stemmed by the current Snowball rules (frostem), which cut
/// some English words otherwise - `added` is `add`, no longer `ad`; and the
/// manifest says what each file is, as its bytes told it, and which files
/// with no extension were looked at and are no documents.
///
/// 5: a chunk says whether its text was read by OCR, and pictures are
/// documents where their text can be read.
///
/// 6: the manifest keeps what each file could not give - documents with no
/// text, pictures of text not read, encodings guessed - and the files not
/// read at all, with why.
///
/// 7: the line a chunk starts on is a fast field, so the chunks of one file
/// are put in order without reading each of them - a preview shows a chunk
/// among its neighbours.
pub const FULLTEXT_FORMAT_VERSION: u32 = 7;

/// The nooma that tried a file it could not read: a later one may read it,
/// or tell it otherwise, so it tries it again.
const TRIED_BY: &str = env!("CARGO_PKG_VERSION");

/// The files that say what a folder leaves out: a change to one is a change
/// to what the whole folder holds.
const RULE_FILES: [&str; 3] = [".gitignore", ".ignore", ".nooma-ignore"];

/// How documents are cut into chunks.
///
/// Held apart from the format for the reason [`crate::index::CHUNKER_VERSION`]
/// is: a file's bytes and hash do not change when the rules for cutting it do,
/// so without this number an index built by the old rules would be kept.
pub const DOC_CHUNKER_VERSION: u32 = 1;

/// How much memory the index writer may use while indexing.
const WRITER_MEMORY: usize = 64 * 1024 * 1024;

/// Characters in a result's fragment.
pub(crate) const FRAGMENT_CHARS: usize = 240;

/// A folder the library searches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// The folder, as an absolute path.
    pub path: PathBuf,
    /// Patterns to leave out, in `.gitignore` syntax, relative to the folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SourcesFile {
    format_version: u32,
    sources: Vec<Source>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    format_version: u32,
    chunker_version: u32,
    /// When the last update finished, as seconds since the epoch.
    indexed_at: Option<i64>,
    /// Every indexed file, by its path as a string.
    files: BTreeMap<String, FileEntry>,
    /// Files that were looked at and are no documents, by their path: those
    /// with no extension, and pictures with no text in them. Not opened
    /// again until they change, or what looked at them does. A folder of
    /// them - programs, a camera's photos - would otherwise be read on every
    /// update.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    passed: BTreeMap<String, Passed>,
    /// Files found and not read, by their path, with why.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    skipped: BTreeMap<String, Unreadable>,
}

/// A file found and not read, as it was then, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Unreadable {
    /// `None` for a folder that could not be walked into.
    size: Option<u64>,
    modified: u64,
    why: Why,
    reason: String,
    /// The nooma that tried it: see [`TRIED_BY`].
    by: String,
}

impl Unreadable {
    /// Whether the file would be refused as it was, were it tried now:
    /// unchanged, tried by this nooma, and for a reason that lasts.
    fn still(&self, file: &WalkedFile) -> bool {
        self.size == Some(file.size) && self.modified == file.modified && self.by == TRIED_BY && self.why.lasts()
    }
}

/// A file looked at and found to be no document, as it was then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Passed {
    size: u64,
    modified: u64,
    /// What looked for text in it, for a picture: see [`reading::reader`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reader: String,
}

impl Passed {
    /// Whether the file would be passed over as it was, were it looked at
    /// now: unchanged, and a picture looked at by what reads pictures now.
    fn still(&self, file: &WalkedFile) -> bool {
        self.size == file.size && self.modified == file.modified && (self.reader.is_empty() || reading::reads_the_same(DocKind::Image, &self.reader))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileEntry {
    /// The source the file was found under.
    source: PathBuf,
    size: u64,
    /// Modification time in nanoseconds since the epoch.
    modified: u64,
    /// The blake3 hash of the file's bytes.
    hash: String,
    /// What the file is, as its bytes told it.
    kind: DocKind,
    title: String,
    /// Path inside the source, lowercased, `/`-separated, without the
    /// extension of a note and with any other's: what a `[[folder/note]]` or
    /// `![[folder/manual.pdf]]` link is matched against.
    rel: String,
    /// The notes this one links to, as written.
    links: Vec<String>,
    /// The files that link to this one, by path.
    backlinks: Vec<String>,
    chunks: usize,
    /// What read the file's text, when it was not read as UTF-8 text: see
    /// [`reading::reader`]. A file read otherwise now is read again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reader: String,
    /// Whether a PDF, Word document or book holds no text: a scan, most
    /// likely, where there is no OCR to read it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    textless: bool,
    /// Why its pages that are pictures of text could not be read, when they
    /// could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unrecognized: Option<String>,
    /// The encoding a text file was read in, when it was guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guessed: Option<String>,
}

impl FileEntry {
    /// Whether the file would be read as it was, were it read now.
    fn reads_the_same(&self) -> bool {
        reading::reads_the_same(self.kind, &self.reader)
    }
}

/// What an update did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UpdateReport {
    /// Documents the library holds after the update.
    pub documents: usize,
    /// Documents read and indexed again in this update.
    pub indexed: usize,
    /// Documents that were in the index and are gone from their folder.
    pub removed: usize,
    /// Chunks written in this update.
    pub chunks: usize,
    /// Files that were found and could not be read, with the reason.
    pub skipped: Vec<Skipped>,
    /// PDFs, Word documents and books read in this update that hold no
    /// text: scans, most likely, where there is no OCR to read them. They
    /// are in the index, found by their names alone.
    pub textless: Vec<PathBuf>,
    /// Pictures and PDFs read in this update whose text was read, in whole
    /// or in part, by OCR.
    pub recognized: Vec<PathBuf>,
    /// Files read in this update with pages that are pictures of text OCR
    /// could not read, with why. They are in the index by what else they
    /// hold.
    pub unrecognized: Vec<Unrecognized>,
    /// Text files read in this update whose encoding was not UTF-8 and was
    /// guessed, with the guess: a wrong one reads as gibberish, and is seen
    /// here rather than wondered at.
    pub guessed: Vec<Guessed>,
    /// Sources whose folder could not be reached; their documents were kept.
    pub unavailable: Vec<PathBuf>,
    /// Why the whole index was thrown away and built again, if it was.
    pub rebuilt: Option<String>,
    /// How long the update took, in milliseconds.
    pub took_ms: u64,
}

/// A file an update found and could not read - or a folder it could not
/// walk into.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// The file.
    pub path: PathBuf,
    /// Why, as the index's health sorts it.
    pub why: Why,
    /// Why, said for a person.
    pub reason: String,
    /// Its size in bytes, when it is a file that was found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// A document read without its pages that are pictures of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unrecognized {
    /// The file.
    pub path: PathBuf,
    /// Why they could not be read, said for a person.
    pub reason: String,
}

/// What the index lacks of the files under its sources, and what it holds of
/// them only in part: what the updates that read them left in the manifest.
///
/// A person asks it when a document they know is there is not found. Every
/// list is sorted by path, the skipped files by why first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Health {
    /// Files found and not read: kept so until the file changes, or is read.
    pub skipped: Vec<Skipped>,
    /// PDFs, Word documents and books with no text in them - scans, where
    /// there is no OCR to read them - in the index by their names alone.
    pub textless: Vec<PathBuf>,
    /// Why text in scans and pictures is not read on this machine, when it
    /// is not and there are scans in the library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_ocr: Option<String>,
    /// Documents read without their pages that are pictures of text.
    pub unrecognized: Vec<Unrecognized>,
    /// Text files read in an encoding that was guessed: a wrong guess reads
    /// as gibberish.
    pub guessed: Vec<Guessed>,
    /// Sources whose folder cannot be reached now; their documents are kept
    /// as they were.
    pub unavailable: Vec<PathBuf>,
}

impl Health {
    /// How many files and folders there are to look at.
    pub fn count(&self) -> usize {
        self.skipped.len() + self.textless.len() + self.unrecognized.len() + self.guessed.len() + self.unavailable.len()
    }
}

/// What an update looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every file under every source.
    Everything,
    /// These paths and everything under them: files and folders a watcher
    /// saw change, some of them gone by now. The rest of the index is kept
    /// as it is.
    Paths(BTreeSet<PathBuf>),
}

impl Scope {
    /// One scope that covers this one and another.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Paths(mut these), Self::Paths(those)) => {
                these.extend(those);
                Self::Paths(these)
            }
            _ => Self::Everything,
        }
    }

    /// The part of the scope under a source.
    fn within(&self, source: &Path) -> Within {
        let Self::Paths(paths) = self else { return Within::All };
        let mut under = Vec::new();
        for path in paths.iter().filter(|path| path.starts_with(source)) {
            // A rule file decides what its whole folder holds.
            let path = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) if RULE_FILES.contains(&name) => path.parent().unwrap_or(path),
                _ => path.as_path(),
            };
            if path == source {
                return Within::All;
            }
            under.push(path.to_path_buf());
        }
        if under.is_empty() { Within::Nothing } else { Within::Paths(Arc::new(under)) }
    }
}

/// The part of a scope under one source.
#[derive(Debug, Clone)]
enum Within {
    All,
    Nothing,
    Paths(Arc<Vec<PathBuf>>),
}

impl Within {
    /// Whether a file, or a folder and all it holds, is looked at.
    fn covers(&self, path: &Path) -> bool {
        match self {
            Self::All => true,
            Self::Nothing => false,
            Self::Paths(paths) => paths.iter().any(|changed| path.starts_with(changed)),
        }
    }

    /// Whether a walk goes into a folder: one a changed path is in, or one
    /// inside a changed folder.
    fn reaches(&self, dir: &Path) -> bool {
        match self {
            Self::All => true,
            Self::Nothing => false,
            Self::Paths(paths) => paths.iter().any(|changed| changed.starts_with(dir) || dir.starts_with(changed)),
        }
    }
}

/// How an update keeps pace with the machine it runs on.
#[derive(Clone, Copy)]
pub struct Pace<'a> {
    /// Told how far the update has got.
    pub progress: &'a (dyn Fn(Progress) + Sync),
    /// Set for an update nobody is waiting for: its threads run below the
    /// person's programs, and it calls this before each file it reads and
    /// each batch of passages it embeds, going on when it returns -
    /// [`crate::background::hold`] holds it while the machine runs on its
    /// battery.
    pub background: Option<&'a (dyn Fn() + Sync)>,
}

impl<'a> Pace<'a> {
    /// An update someone asked for and waits on.
    pub fn foreground(progress: &'a (dyn Fn(Progress) + Sync)) -> Self {
        Self { progress, background: None }
    }

    /// An update nobody is waiting for: see [`Pace::background`].
    pub fn background(progress: &'a (dyn Fn(Progress) + Sync), hold: &'a (dyn Fn() + Sync)) -> Self {
        Self {
            progress,
            background: Some(hold),
        }
    }

    pub(crate) fn hold(&self) {
        if let Some(hold) = self.background {
            hold();
        }
    }
}

/// A text file whose encoding an update guessed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Guessed {
    /// The file.
    pub path: PathBuf,
    /// The encoding it was read in, by its WHATWG name: `windows-1251`.
    pub encoding: String,
}

/// How far an update has got, for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Files read so far.
    pub done: usize,
    /// Files that need reading in this update.
    pub total: usize,
}

/// What the library holds, without changing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The folders.
    pub sources: Vec<Source>,
    /// Documents in the index.
    pub documents: usize,
    /// Chunks in the index.
    pub chunks: usize,
    /// When an update last changed the index, as seconds since the epoch;
    /// `None` before the first. One that found nothing changed leaves it.
    pub indexed_at: Option<i64>,
    /// Why the stored index cannot be searched until the next update, if it
    /// cannot.
    pub stale: Option<String>,
}

/// One document found by a search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    /// The file.
    pub path: PathBuf,
    /// `markdown`, `text` or `pdf`.
    pub kind: String,
    /// The document's title.
    pub title: String,
    /// The headings above the matching chunk, outermost first.
    pub headings: Vec<String>,
    /// The line the matching chunk starts on.
    pub line: u32,
    /// The page the matching chunk is on, in a PDF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Whether the matching chunk's text was read by OCR out of a picture,
    /// and so may hold misread words.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recognized: bool,
    /// A piece of the matching chunk around the words that matched.
    pub fragment: String,
    /// Where in `fragment` the matching words are, as byte ranges.
    #[serde(serialize_with = "ranges_as_pairs")]
    pub highlights: Vec<Range<usize>>,
    /// Where in `fragment` the sentence closest in meaning to the query is,
    /// as a byte range: set for a hit the meaning found and no word of the
    /// query is marked in, once asked why - see [`crate::why`].
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "range_as_pair")]
    pub closest: Option<Range<usize>>,
    /// The document's tags.
    pub tags: Vec<String>,
    /// When the file was last modified, in seconds since the epoch.
    pub modified: i64,
    /// How well it matched. Comparable within one search, not across two.
    pub score: f32,
}

/// What the words of a query found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Matched {
    /// The documents, best first, one per document.
    pub hits: Vec<WordHit>,
    /// Whether every hit holds every word of the query. `false` when no
    /// document held them all and these hold some: a weaker answer, and one
    /// that has to be said as such.
    pub all_words: bool,
}

/// A document found by the words of a query.
#[derive(Debug, Clone, PartialEq)]
pub struct WordHit {
    /// Its best chunk; the score is the full-text one.
    pub hit: Hit,
    /// The share of the query's distinctive words the chunk holds, from 0
    /// to 1: one when it holds every one, as it does whenever
    /// [`Matched::all_words`] is true. One word of six is a chunk that shares
    /// a word with the question, not one that answers it.
    pub share: f32,
}

fn ranges_as_pairs<S: serde::Serializer>(ranges: &[Range<usize>], serializer: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(ranges.len()))?;
    for range in ranges {
        seq.serialize_element(&[range.start, range.end])?;
    }
    seq.end()
}

fn range_as_pair<S: serde::Serializer>(range: &Option<Range<usize>>, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    match range {
        Some(range) => serializer.serialize_some(&[range.start, range.end]),
        None => serializer.serialize_none(),
    }
}

/// The folders a person searches, and the index over them.
#[derive(Debug, Clone)]
pub struct Library {
    root: PathBuf,
    sources: Vec<Source>,
}

impl Library {
    /// The library in this machine's usual place for application data.
    pub fn open() -> Result<Self> {
        Self::open_at(crate::data_dir()?.join("library"))
    }

    /// The library in a given directory, created if it is not there.
    pub fn open_at(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;
        let path = root.join("sources.json");
        let sources = match std::fs::read(&path) {
            Ok(bytes) => {
                let file: SourcesFile = serde_json::from_slice(&bytes).map_err(|source| Error::IndexUnreadable { path: path.clone(), source })?;
                if file.format_version != SOURCES_FORMAT_VERSION {
                    return Err(Error::IndexFormat {
                        found: file.format_version,
                        expected: SOURCES_FORMAT_VERSION,
                    });
                }
                file.sources
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        Ok(Self { root, sources })
    }

    /// Where this library keeps its files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folders it searches.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Add a folder to search. Nothing is read until the next update.
    ///
    /// Two sources may not overlap: a folder inside another would put its
    /// files in the index twice, and one containing another would make the
    /// inner one's exclusions mean nothing.
    pub fn add_source(&mut self, path: &Path, exclude: Vec<String>) -> Result<&Source> {
        let path = std::path::absolute(path).map_err(|e| Error::io(path, e))?;
        if !path.is_dir() {
            return Err(Error::NotADirectory(path));
        }
        for existing in &self.sources {
            if path.starts_with(&existing.path) || existing.path.starts_with(&path) {
                return Err(Error::SourceOverlap {
                    path,
                    existing: existing.path.clone(),
                });
            }
        }
        self.sources.push(Source { path, exclude });
        self.save_sources()?;
        Ok(self.sources.last().expect("just pushed"))
    }

    /// Stop searching a folder. Its documents leave the index at the next
    /// update.
    pub fn remove_source(&mut self, path: &Path) -> Result<Source> {
        let absolute = std::path::absolute(path).map_err(|e| Error::io(path, e))?;
        let position = self
            .sources
            .iter()
            .position(|source| source.path == absolute)
            .ok_or(Error::UnknownSource(absolute))?;
        let removed = self.sources.remove(position);
        self.save_sources()?;
        Ok(removed)
    }

    fn save_sources(&self) -> Result<()> {
        let file = SourcesFile {
            format_version: SOURCES_FORMAT_VERSION,
            sources: self.sources.clone(),
        };
        write_atomically(&self.root.join("sources.json"), &serde_json::to_vec_pretty(&file).expect("sources serialize"))
    }

    fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// The index directory is named by its format. An index of another
    /// format is never opened in place: the new one is built beside it and
    /// the old one removed after, which also works when another process -
    /// the window, say - still has the old one open, as Windows would refuse
    /// to delete files in use.
    fn index_dir(&self) -> PathBuf {
        self.root.join(format!("fulltext-{FULLTEXT_FORMAT_VERSION}"))
    }

    /// Remove index directories of other formats, as far as the system
    /// allows; one still held open goes on the next update.
    fn remove_other_formats(&self) {
        let current = self.index_dir();
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path != current && path.is_dir() && name.to_string_lossy().starts_with("fulltext") {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    fn load_manifest(&self) -> Result<Option<Manifest>> {
        let path = self.manifest_path();
        match std::fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(&path, e)),
        }
    }

    /// Why the stored index cannot be used as it is, or `None` if it can.
    fn staleness(&self, manifest: Option<&Manifest>) -> Option<String> {
        let manifest = manifest?;
        if manifest.format_version != FULLTEXT_FORMAT_VERSION {
            return Some(format!(
                "the index is format {}, this nooma writes {FULLTEXT_FORMAT_VERSION}",
                manifest.format_version
            ));
        }
        if manifest.chunker_version != DOC_CHUNKER_VERSION {
            return Some(format!(
                "documents were cut by chunker {}, this nooma cuts by {DOC_CHUNKER_VERSION}",
                manifest.chunker_version
            ));
        }
        None
    }

    fn open_index(&self) -> Result<(Index, Fields)> {
        let dir = self.index_dir();
        let index = Index::open_in_dir(&dir).map_err(Error::fulltext)?;
        let fields = Fields::from_schema(&index.schema()).map_err(Error::fulltext)?;
        index.tokenizers().register(fulltext::ANALYZER, fulltext::analyzer());
        Ok((index, fields))
    }

    fn create_index(&self) -> Result<(Index, Fields)> {
        let dir = self.index_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        }
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let (schema, fields) = Fields::schema();
        let index = Index::create_in_dir(&dir, schema).map_err(Error::fulltext)?;
        index.tokenizers().register(fulltext::ANALYZER, fulltext::analyzer());
        Ok((index, fields))
    }

    /// What the library holds, read without changing anything.
    pub fn status(&self) -> Result<Status> {
        let manifest = self.load_manifest()?;
        let stale = self.staleness(manifest.as_ref());
        let manifest = manifest.unwrap_or_default();
        Ok(Status {
            sources: self.sources.clone(),
            documents: manifest.files.len(),
            chunks: manifest.files.values().map(|entry| entry.chunks).sum(),
            indexed_at: manifest.indexed_at,
            stale,
        })
    }

    /// Bring the index up to date with the folders.
    pub fn update(&self) -> Result<UpdateReport> {
        self.update_with(&|_| {})
    }

    /// [`Library::update`], reporting progress as files are read.
    ///
    /// Fails with [`Error::Busy`] when another process is updating the same
    /// library; the index it would have written is the one that process is
    /// already writing.
    pub fn update_with(&self, progress: &(dyn Fn(Progress) + Sync)) -> Result<UpdateReport> {
        self.update_scoped(&Scope::Everything, Pace::foreground(progress))
    }

    /// Bring the index up to date with what is in a scope of the folders, at
    /// a pace: see [`Scope`] and [`Pace`].
    ///
    /// An index that has to be built again is built from everything, whatever
    /// the scope. Fails with [`Error::Busy`] as [`Library::update_with`] does.
    pub fn update_scoped(&self, scope: &Scope, pace: Pace<'_>) -> Result<UpdateReport> {
        let started = Instant::now();
        let mut report = UpdateReport::default();

        let stored = self.load_manifest()?;
        // No manifest at all is a first run, not a rebuild worth announcing; a
        // manifest of another format or chunker is.
        report.rebuilt = self.staleness(stored.as_ref());
        let mut start_over = stored.is_none() || report.rebuilt.is_some();
        let mut manifest = stored.filter(|_| !start_over).unwrap_or_default();

        let (index, fields) = match self.index_dir().join("meta.json").exists() {
            true => match self.open_index() {
                Ok(opened) => opened,
                Err(error) => {
                    report.rebuilt = Some(format!("the stored index could not be opened: {error}"));
                    start_over = true;
                    manifest = Manifest::default();
                    self.create_index()?
                }
            },
            false => {
                if !start_over {
                    report.rebuilt = Some("the stored index is missing".to_string());
                    start_over = true;
                    manifest = Manifest::default();
                }
                self.create_index()?
            }
        };

        let mut writer: IndexWriter = match index.writer_with_num_threads(indexing_threads(), WRITER_MEMORY) {
            Ok(writer) => writer,
            Err(tantivy::TantivyError::LockFailure(..)) => return Err(Error::Busy),
            Err(error) => return Err(Error::fulltext(error)),
        };
        // Starting over empties the index through the writer, so a reader
        // elsewhere keeps working and sees the new contents at the commit,
        // rather than having its files deleted from under it.
        if start_over {
            writer.delete_all_documents().map_err(Error::fulltext)?;
        }
        let scope = if start_over { &Scope::Everything } else { scope };

        // Walk every source, as far as the scope reaches: what is there now,
        // and what can be skipped without opening it. What the walk does not
        // look at is kept as it was - all of a source it cannot reach or was
        // not asked about, and the rest of one it was asked about in part.
        let mut found: BTreeMap<String, Found> = BTreeMap::new();
        let mut passed: BTreeMap<String, Passed> = BTreeMap::new();
        let mut skipped: BTreeMap<String, Unreadable> = BTreeMap::new();
        for source in &self.sources {
            let reachable = source.path.is_dir();
            if !reachable {
                report.unavailable.push(source.path.clone());
            }
            let within = if reachable { scope.within(&source.path) } else { Within::Nothing };
            let kept = |key: &String| Path::new(key).starts_with(&source.path) && !within.covers(Path::new(key));
            for (key, entry) in &manifest.files {
                if entry.source == source.path && !within.covers(Path::new(key)) {
                    found.insert(key.clone(), Found::Kept(entry.clone()));
                }
            }
            passed.extend(manifest.passed.iter().filter(|(key, _)| kept(key)).map(|(key, p)| (key.clone(), p.clone())));
            skipped.extend(manifest.skipped.iter().filter(|(key, _)| kept(key)).map(|(key, s)| (key.clone(), s.clone())));
            if matches!(within, Within::Nothing) {
                continue;
            }

            let mut unwalked = Vec::new();
            let files = walk(source, &within, &mut unwalked)?;
            for (path, reason) in unwalked {
                skipped.insert(
                    path.to_string_lossy().into_owned(),
                    Unreadable {
                        size: None,
                        modified: 0,
                        why: Why::Unopened,
                        reason: reason.clone(),
                        by: TRIED_BY.to_string(),
                    },
                );
                report.skipped.push(Skipped {
                    path,
                    why: Why::Unopened,
                    reason,
                    size: None,
                });
            }
            for file in files {
                let key = file.path.to_string_lossy().into_owned();
                if let Some(still) = manifest.passed.get(&key).filter(|passed| passed.still(&file)) {
                    passed.insert(key, still.clone());
                    continue;
                }
                if let Some(still) = manifest.skipped.get(&key).filter(|refused| refused.still(&file)) {
                    skipped.insert(key, still.clone());
                    continue;
                }
                let unchanged = manifest
                    .files
                    .get(&key)
                    .filter(|entry| entry.size == file.size && entry.modified == file.modified && entry.source == source.path && entry.reads_the_same());
                let found_as = match unchanged {
                    Some(entry) => Found::Kept(entry.clone()),
                    None => Found::Candidate(file),
                };
                found.insert(key, found_as);
            }
        }

        // Read what may have changed. A file whose bytes hash as before is
        // kept as it was; only a real change is parsed.
        let candidates: Vec<&WalkedFile> = found
            .values()
            .filter_map(|found| match found {
                Found::Candidate(file) => Some(file),
                Found::Kept(_) => None,
            })
            .collect();
        let total = candidates.len();
        let done = AtomicUsize::new(0);
        (pace.progress)(Progress { done: 0, total });
        // On as many threads as the writer gets, not every core: a PDF is
        // parsed, not just read, and a folder of books would otherwise take
        // the whole machine - and hold a file's bytes per core.
        let background = pace.background.is_some();
        let readers = rayon::ThreadPoolBuilder::new()
            .num_threads(indexing_threads())
            .start_handler(move |_| {
                if background {
                    crate::background::lower_this_thread();
                }
            })
            .build()
            .map_err(|e| Error::io(&self.root, std::io::Error::other(e)))?;
        let read: Vec<(String, std::result::Result<Read, Unread>)> = readers.install(|| {
            candidates
                .par_iter()
                .map(|file| {
                    pace.hold();
                    let key = file.path.to_string_lossy().into_owned();
                    let previous = manifest.files.get(&key);
                    let result = read_file(file, previous);
                    let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                    (pace.progress)(Progress { done, total });
                    (key, result)
                })
                .collect()
        });

        let mut parsed: BTreeMap<String, Document> = BTreeMap::new();
        let mut entries: BTreeMap<String, FileEntry> = BTreeMap::new();
        for (key, found) in &found {
            if let Found::Kept(entry) = found {
                entries.insert(key.clone(), entry.clone());
            }
        }
        for (key, result) in read {
            let Found::Candidate(file) = &found[&key] else {
                unreachable!("only candidates are read")
            };
            match result {
                Ok(Read::Same(mut entry)) => {
                    entry.size = file.size;
                    entry.modified = file.modified;
                    entries.insert(key, entry);
                }
                Ok(Read::Changed(entry, doc)) => {
                    if entry.textless {
                        report.textless.push(file.path.clone());
                    }
                    if doc.chunks.iter().any(|chunk| chunk.recognized) {
                        report.recognized.push(file.path.clone());
                    }
                    if let Some(reason) = &entry.unrecognized {
                        report.unrecognized.push(Unrecognized {
                            path: file.path.clone(),
                            reason: reason.clone(),
                        });
                    }
                    if let Some(encoding) = &entry.guessed {
                        report.guessed.push(Guessed {
                            path: file.path.clone(),
                            encoding: encoding.clone(),
                        });
                    }
                    entries.insert(key.clone(), entry);
                    parsed.insert(key, doc);
                }
                // A picture with no text in it is not reported at all, and a
                // file whose name never said it was a document is not
                // reported for not being one: both are passed over until
                // they change.
                Err(unread) => match unread.why().filter(|_| unread.is_said(&file.path)) {
                    Some((why, reason)) => {
                        skipped.insert(key, Unreadable::of(file, why, reason));
                        report.skipped.push(Skipped {
                            path: file.path.clone(),
                            why,
                            reason: reason.to_string(),
                            size: Some(file.size),
                        });
                    }
                    None => {
                        let reader = match unread {
                            Unread::NoText(reader) => reader,
                            _ => String::new(),
                        };
                        passed.insert(
                            key,
                            Passed {
                                size: file.size,
                                modified: file.modified,
                                reader,
                            },
                        );
                    }
                },
            }
        }

        let removed: Vec<String> = manifest.files.keys().filter(|key| !entries.contains_key(*key)).cloned().collect();

        // Backlinks are a fact about the whole library, not a file: resolve
        // every link again, then reindex whatever the answer changed for.
        resolve_backlinks(&mut entries);
        let mut reindex: BTreeSet<String> = parsed.keys().cloned().collect();
        for (key, entry) in &entries {
            match manifest.files.get(key) {
                Some(old) if old.backlinks != entry.backlinks => {
                    reindex.insert(key.clone());
                }
                _ => {}
            }
        }
        // A linking note's title is part of the target's backlinks field, so
        // a changed or removed note sends its targets, old and new, round
        // again. The map is built once: asking every entry about every
        // touched note is quadratic, and the first update touches them all.
        let mut targets_of: BTreeMap<&String, BTreeSet<&String>> = BTreeMap::new();
        for (key, entry) in entries.iter().chain(manifest.files.iter()) {
            for linker in &entry.backlinks {
                targets_of.entry(linker).or_default().insert(key);
            }
        }
        for touched in parsed.keys().chain(removed.iter()) {
            if let Some(targets) = targets_of.get(touched) {
                reindex.extend(targets.iter().filter(|key| entries.contains_key(**key)).map(|key| (*key).clone()));
            }
        }

        // An update that found nothing changed leaves the index and the
        // manifest as they are: a watcher wakes one for every file saved
        // anywhere under a source, and most of them are no documents.
        let touches_index = start_over || !removed.is_empty() || !reindex.is_empty();
        if !touches_index && entries == manifest.files && passed == manifest.passed && skipped == manifest.skipped {
            drop(writer);
            report.documents = entries.len();
            report.took_ms = millis_since(started);
            return Ok(report);
        }

        for key in &removed {
            writer.delete_term(Term::from_field_text(fields.path, key));
        }
        report.removed = removed.len();

        let mut dropped = Vec::new();
        for key in &reindex {
            let Some(entry) = entries.get(key) else { continue };
            let doc = match parsed.remove(key) {
                Some(doc) => doc,
                None => {
                    pace.hold();
                    match reading::read_path(Path::new(key)) {
                        Ok(opened) => opened.document,
                        Err(unread) => {
                            // A picture whose text is gone by now is passed
                            // over on the next update, without a word.
                            if let Some((why, reason)) = unread.why().filter(|_| unread.is_said(Path::new(key))) {
                                report.skipped.push(Skipped {
                                    path: PathBuf::from(key),
                                    why,
                                    reason: reason.to_string(),
                                    size: Some(entry.size),
                                });
                            }
                            dropped.push(key.clone());
                            writer.delete_term(Term::from_field_text(fields.path, key));
                            continue;
                        }
                    }
                }
            };
            writer.delete_term(Term::from_field_text(fields.path, key));
            let backlink_titles: Vec<&str> = entry.backlinks.iter().filter_map(|k| entries.get(k)).map(|e| e.title.as_str()).collect();
            report.chunks += add_document(&writer, &fields, key, entry, &doc, &backlink_titles)?;
            report.indexed += 1;
        }
        for key in dropped {
            entries.remove(&key);
        }

        if touches_index {
            writer.commit().map_err(Error::fulltext)?;
        }

        manifest.format_version = FULLTEXT_FORMAT_VERSION;
        manifest.chunker_version = DOC_CHUNKER_VERSION;
        manifest.indexed_at = Some(unix_now());
        manifest.files = entries;
        manifest.passed = passed;
        manifest.skipped = skipped;
        let bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
        write_atomically(&self.manifest_path(), &bytes)?;
        // The writer holds tantivy's lock; dropping it only after the manifest
        // is on disk keeps a second updater from reading a manifest that is
        // behind the index it is about to extend.
        writer.wait_merging_threads().map_err(Error::fulltext)?;
        self.remove_other_formats();

        report.documents = manifest.files.len();
        report.took_ms = millis_since(started);
        Ok(report)
    }

    /// What the index lacks of the files under its sources, and what it
    /// holds of them only in part, read without changing anything: see
    /// [`Health`].
    pub fn health(&self) -> Result<Health> {
        let manifest = self.load_manifest()?.unwrap_or_default();
        let under_a_source = |key: &str| self.sources.iter().any(|source| Path::new(key).starts_with(&source.path));
        let mut health = Health::default();
        for (key, refused) in manifest.skipped.iter().filter(|(key, _)| under_a_source(key)) {
            health.skipped.push(Skipped {
                path: PathBuf::from(key),
                why: refused.why,
                reason: refused.reason.clone(),
                size: refused.size,
            });
        }
        health.skipped.sort_by(|a, b| a.why.cmp(&b.why).then_with(|| a.path.cmp(&b.path)));
        for (key, entry) in manifest.files.iter().filter(|(key, _)| under_a_source(key)) {
            let path = PathBuf::from(key);
            if entry.textless {
                health.textless.push(path.clone());
            }
            if let Some(reason) = &entry.unrecognized {
                health.unrecognized.push(Unrecognized {
                    path: path.clone(),
                    reason: reason.clone(),
                });
            }
            if let Some(encoding) = &entry.guessed {
                health.guessed.push(Guessed {
                    path,
                    encoding: encoding.clone(),
                });
            }
        }
        if !health.textless.is_empty() {
            health.no_ocr = crate::ocr::ocr().err().map(str::to_string);
        }
        health.unavailable = self
            .sources
            .iter()
            .filter(|source| !source.path.is_dir())
            .map(|source| source.path.clone())
            .collect();
        Ok(health)
    }

    /// Find the documents that contain the words of a query.
    ///
    /// Every word has to occur, in the text or the title, tags, headings or
    /// links; when no document holds them all, documents holding any of them
    /// are returned instead, ranked by how many and how rare, and the answer
    /// says it is that weaker one. An empty result for a four-word query where
    /// three words match is the answer a person takes for "search is broken".
    ///
    /// One hit per document, at its best chunk.
    pub fn search(&self, query: &str, limit: usize) -> Result<Matched> {
        match self.finder()? {
            Some(finder) => finder.search(query, limit),
            None => Ok(Matched::default()),
        }
    }

    /// The index opened for searching, to keep and search many times; `None`
    /// before the first update.
    ///
    /// Opening the index and checking its format costs more than a search
    /// does, so a caller answering every keystroke opens it once. The finder
    /// sees each update as it is committed, by this process or another.
    pub fn finder(&self) -> Result<Option<Finder>> {
        let manifest = self.load_manifest()?;
        if let Some(reason) = self.staleness(manifest.as_ref()) {
            return Err(Error::LibraryStale(reason));
        }
        if manifest.is_none() {
            return Ok(None);
        }
        let (index, fields) = self.open_index()?;
        let reader = index.reader().map_err(Error::fulltext)?;
        Ok(Some(Finder { index, fields, reader }))
    }
}

/// A file as the last update read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndexedFile {
    /// The blake3 hash of its bytes.
    pub(crate) hash: String,
    /// The chunks it was cut into.
    pub(crate) chunks: usize,
}

/// A chunk as the full-text index holds it.
#[cfg(feature = "semantic")]
#[derive(Debug, Clone)]
pub(crate) struct StoredChunk {
    pub(crate) path: String,
    pub(crate) title: String,
    pub(crate) headings: Vec<String>,
    pub(crate) body: String,
    pub(crate) line: u32,
}

impl Library {
    /// Every chunk in the full-text index, by path and line.
    ///
    /// The index is where the chunks are kept, so it is what everything built
    /// over chunks reads them from: a second copy would be a second answer to
    /// "what did the chunker make of this file". Empty before the first
    /// update; an index that must be rebuilt is an error, since its chunks are
    /// about to change.
    #[cfg(feature = "semantic")]
    pub(crate) fn stored_chunks(&self) -> Result<Vec<StoredChunk>> {
        let manifest = self.load_manifest()?;
        if let Some(reason) = self.staleness(manifest.as_ref()) {
            return Err(Error::LibraryStale(reason));
        }
        if manifest.is_none() {
            return Ok(Vec::new());
        }
        let (index, fields) = self.open_index()?;
        let reader = index.reader().map_err(Error::fulltext)?;
        let searcher = reader.searcher();
        let addresses = searcher
            .search(&tantivy::query::AllQuery, &tantivy::collector::DocSetCollector)
            .map_err(Error::fulltext)?;
        let mut chunks = Vec::with_capacity(addresses.len());
        for address in addresses {
            let doc: TantivyDocument = searcher.doc(address).map_err(Error::fulltext)?;
            chunks.push(StoredChunk {
                path: first_text(&doc, fields.path),
                title: first_text(&doc, fields.title),
                headings: all_text(&doc, fields.headings),
                body: first_text(&doc, fields.body),
                line: line_of(&doc, &fields),
            });
        }
        chunks.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        Ok(chunks)
    }

    /// Every file the full-text index holds, by path; empty before the first
    /// update, an error when the index must be rebuilt.
    pub(crate) fn indexed_files(&self) -> Result<BTreeMap<String, IndexedFile>> {
        let manifest = self.load_manifest()?;
        if let Some(reason) = self.staleness(manifest.as_ref()) {
            return Err(Error::LibraryStale(reason));
        }
        Ok(manifest
            .map(|manifest| {
                manifest
                    .files
                    .into_iter()
                    .map(|(path, entry)| {
                        (
                            path,
                            IndexedFile {
                                hash: entry.hash,
                                chunks: entry.chunks,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// An index open for searching.
pub struct Finder {
    index: Index,
    fields: Fields,
    reader: tantivy::IndexReader,
}

impl std::fmt::Debug for Finder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finder").finish_non_exhaustive()
    }
}

impl Finder {
    /// See [`Library::search`].
    pub fn search(&self, query: &str, limit: usize) -> Result<Matched> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Matched::default());
        }
        // A commit by another process is picked up now rather than when the
        // reader's own watcher gets round to it: a search right after an
        // update has to see it.
        self.reader.reload().map_err(Error::fulltext)?;
        let searcher = self.reader.searcher();
        let fields = &self.fields;

        let mut parser = QueryParser::for_index(&self.index, fields.searched().iter().map(|(field, _)| *field).collect());
        for (field, boost) in fields.searched() {
            parser.set_field_boost(field, boost);
        }
        let distinctive = Distinctive::of(&searcher, fields.body, query);
        let any = parser.parse_query_lenient(&distinctive.query).0;
        parser.set_conjunction_by_default();
        let all = parser.parse_query_lenient(query).0;

        let hits = self.collect(&searcher, fields, &*all, limit, None)?;
        if !hits.is_empty() {
            return Ok(Matched { hits, all_words: true });
        }
        Ok(Matched {
            hits: self.collect(&searcher, fields, &*any, limit, Some(&distinctive.terms))?,
            all_words: false,
        })
    }

    /// The best chunk of each of the first `limit` documents. `terms`, when
    /// given, are the words a chunk is measured against for its share; without
    /// them every hit holds all the words, as the query demanded.
    fn collect(
        &self,
        searcher: &tantivy::Searcher,
        fields: &Fields,
        query: &dyn Query,
        limit: usize,
        terms: Option<&BTreeSet<String>>,
    ) -> Result<Vec<WordHit>> {
        // Several chunks of one document can outrank every chunk of the next,
        // so more are fetched than will be shown.
        let fetch = limit.saturating_mul(8).saturating_add(32);
        let collector = TopDocs::with_limit(fetch).tweak_score(move |segment: &SegmentReader| {
            let counts = segment.fast_fields().u64("backlink_count").map(|column| column.first_or_default_col(0)).ok();
            move |doc: DocId, score: Score| {
                // Notes others point to are a little more likely to be the
                // one meant; a little, so a note nobody links to still wins on
                // its words.
                let links = counts.as_ref().map_or(0, |c| c.get_val(doc));
                score * (1.0 + 0.1 * (links as f32).ln_1p())
            }
        });
        let top: Vec<(Score, DocAddress)> = searcher.search(query, &collector).map_err(Error::fulltext)?;

        let mut snippets = SnippetGenerator::create(searcher, query, fields.body).map_err(Error::fulltext)?;
        snippets.set_max_num_chars(FRAGMENT_CHARS);

        let mut seen = BTreeSet::new();
        let mut hits = Vec::new();
        for (score, address) in top {
            let doc: TantivyDocument = searcher.doc(address).map_err(Error::fulltext)?;
            let path = first_text(&doc, fields.path);
            if !seen.insert(path.clone()) {
                continue;
            }
            let body = first_text(&doc, fields.body);
            let snippet = snippets.snippet(&body);
            let (fragment, highlights) = if snippet.is_empty() {
                (opening(&body), Vec::new())
            } else {
                (snippet.fragment().to_string(), snippet.highlighted().to_vec())
            };
            let share = terms.map_or(1.0, |terms| share_held(&doc, fields, terms));
            hits.push(WordHit {
                hit: Hit {
                    path: PathBuf::from(path),
                    kind: first_text(&doc, fields.kind),
                    title: first_text(&doc, fields.title),
                    headings: all_text(&doc, fields.headings),
                    line: line_of(&doc, fields),
                    page: page_of(&doc, fields),
                    recognized: recognized_of(&doc, fields),
                    fragment,
                    highlights,
                    closest: None,
                    tags: all_text(&doc, fields.tags),
                    modified: modified_of(&doc, fields),
                    score,
                },
                share,
            });
            if hits.len() == limit {
                break;
            }
        }
        Ok(hits)
    }
}

impl Finder {
    /// The chunks starting at these lines of these files, as hits without a
    /// score: the chunk's opening for a fragment, nothing highlighted.
    ///
    /// What found them - the vector index - knows where a chunk is and not
    /// what it says; the text is kept here, once. A file changed since the
    /// place was recorded gives its chunk at or before the line, and a file
    /// the index no longer holds gives `None`.
    pub fn chunks(&self, at: &[(PathBuf, u32)]) -> Result<Vec<Option<Hit>>> {
        self.reader.reload().map_err(Error::fulltext)?;
        let searcher = self.reader.searcher();
        let fields = &self.fields;
        let mut out = Vec::with_capacity(at.len());
        for (path, line) in at {
            out.push(self.chunk_at(&searcher, path, *line)?.map(|(starts, doc)| Hit {
                path: path.clone(),
                kind: first_text(&doc, fields.kind),
                title: first_text(&doc, fields.title),
                headings: all_text(&doc, fields.headings),
                line: starts,
                page: page_of(&doc, fields),
                recognized: recognized_of(&doc, fields),
                fragment: opening(&first_text(&doc, fields.body)),
                highlights: Vec::new(),
                closest: None,
                tags: all_text(&doc, fields.tags),
                modified: modified_of(&doc, fields),
                score: 0.0,
            }));
        }
        Ok(out)
    }

    /// The whole text of the chunk starting at a line of a file, found as
    /// [`Finder::chunks`] finds it; `None` when the index no longer holds the
    /// file.
    pub fn chunk_text(&self, path: &Path, line: u32) -> Result<Option<String>> {
        self.reader.reload().map_err(Error::fulltext)?;
        let searcher = self.reader.searcher();
        Ok(self.chunk_at(&searcher, path, line)?.map(|(_, doc)| first_text(&doc, self.fields.body)))
    }

    /// The chunk that starts at a line of a file, or - the file changed since
    /// the line was recorded - the one at or before it, with the line it
    /// starts on.
    fn chunk_at(&self, searcher: &tantivy::Searcher, path: &Path, line: u32) -> Result<Option<(u32, TantivyDocument)>> {
        let chunks = self.in_order(searcher, path)?;
        let Some(at) = place_of(&chunks, line) else { return Ok(None) };
        let (starts, address) = chunks[at];
        let doc: TantivyDocument = searcher.doc(address).map_err(Error::fulltext)?;
        Ok(Some((starts, doc)))
    }

    /// The chunk starting at a line of a file - found as [`Finder::chunks`]
    /// finds it - whole, with up to `before` chunks of the file before it and
    /// `after` after it, and the words of `query` marked in each: a document
    /// read around the place a search found in it. `None` when the index no
    /// longer holds the file. Without a query nothing is marked: a pasted
    /// passage is not words to look for.
    ///
    /// The chunks are put in order by the lines the index keeps beside them,
    /// so a book of a thousand chunks costs the reading of the ones shown.
    pub fn preview(&self, path: &Path, line: u32, query: Option<&str>, before: usize, after: usize) -> Result<Option<Preview>> {
        self.reader.reload().map_err(Error::fulltext)?;
        let searcher = self.reader.searcher();
        let fields = &self.fields;
        let chunks = self.in_order(&searcher, path)?;
        let Some(at) = place_of(&chunks, line) else { return Ok(None) };
        let first = at.saturating_sub(before);
        let last = at.saturating_add(after).min(chunks.len() - 1);
        let terms = query.map(|query| Distinctive::of(&searcher, fields.body, query).terms).unwrap_or_default();
        let mut passages = Vec::with_capacity(last - first + 1);
        let mut named = None;
        for (_, address) in &chunks[first..=last] {
            let doc: TantivyDocument = searcher.doc(*address).map_err(Error::fulltext)?;
            named.get_or_insert_with(|| (first_text(&doc, fields.kind), first_text(&doc, fields.title)));
            let text = first_text(&doc, fields.body);
            passages.push(Passage {
                headings: all_text(&doc, fields.headings),
                line: line_of(&doc, fields),
                page: page_of(&doc, fields),
                recognized: recognized_of(&doc, fields),
                highlights: fulltext::marks(&text, &terms),
                text,
            });
        }
        let (kind, title) = named.unwrap_or_default();
        Ok(Some(Preview {
            path: path.to_path_buf(),
            kind,
            title,
            passages,
            found: at - first,
            before: first,
            after: chunks.len() - 1 - last,
        }))
    }

    /// Every chunk of a file with the line it starts on, in order. The lines
    /// are read from the column the index keeps them in, not from the chunks:
    /// a long journal has hundreds of chunks, and reading them all to pick
    /// one cost more than the search that found it.
    fn in_order(&self, searcher: &tantivy::Searcher, path: &Path) -> Result<Vec<(u32, DocAddress)>> {
        let key = path.to_string_lossy();
        let in_file = TermQuery::new(Term::from_field_text(self.fields.path, &key), IndexRecordOption::Basic);
        let addresses = searcher.search(&in_file, &DocSetCollector).map_err(Error::fulltext)?;
        let mut columns: BTreeMap<u32, tantivy::columnar::Column<u64>> = BTreeMap::new();
        let mut out = Vec::with_capacity(addresses.len());
        for address in addresses {
            let column = match columns.entry(address.segment_ord) {
                std::collections::btree_map::Entry::Occupied(column) => column.into_mut(),
                std::collections::btree_map::Entry::Vacant(slot) => {
                    let reader = searcher.segment_reader(address.segment_ord);
                    slot.insert(reader.fast_fields().u64("line").map_err(Error::fulltext)?)
                }
            };
            let starts = column.first(address.doc_id).and_then(|n| u32::try_from(n).ok()).unwrap_or(1);
            out.push((starts, address));
        }
        out.sort_unstable();
        Ok(out)
    }
}

/// Which of a file's chunks, in order, holds a line: the one starting at it,
/// or - the file changed since the line was recorded - the last one before
/// it, or the first.
fn place_of(chunks: &[(u32, DocAddress)], line: u32) -> Option<usize> {
    chunks
        .iter()
        .rposition(|(starts, _)| *starts <= line)
        .or_else(|| (!chunks.is_empty()).then_some(0))
}

/// A document around one of its chunks: what a preview of a search result
/// shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Preview {
    /// The file.
    pub path: PathBuf,
    /// What it is, as a hit names it.
    pub kind: String,
    /// The document's title.
    pub title: String,
    /// The chunks shown, in the document's order.
    pub passages: Vec<Passage>,
    /// Which of `passages` is the chunk asked about.
    pub found: usize,
    /// How many chunks of the document come before the first one shown.
    pub before: usize,
    /// How many come after the last one shown.
    pub after: usize,
}

/// A chunk of a document, whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Passage {
    /// The headings above it, outermost first.
    pub headings: Vec<String>,
    /// The line it starts on, as a hit's.
    pub line: u32,
    /// The page it is on, in a PDF.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Whether its text was read by OCR out of a picture.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub recognized: bool,
    /// Its text: for a note, the markdown it is written in.
    pub text: String,
    /// Where the words of the query are in `text`, as byte ranges.
    #[serde(serialize_with = "ranges_as_pairs")]
    pub highlights: Vec<Range<usize>>,
}

/// A chunk's lines and times are kept as numbers.
fn first_u64(doc: &TantivyDocument, field: tantivy::schema::Field) -> Option<u64> {
    doc.get_first(field).and_then(|v| v.as_u64())
}

fn line_of(doc: &TantivyDocument, fields: &Fields) -> u32 {
    first_u64(doc, fields.line).and_then(|n| u32::try_from(n).ok()).unwrap_or(1)
}

fn page_of(doc: &TantivyDocument, fields: &Fields) -> Option<u32> {
    first_u64(doc, fields.page).and_then(|n| u32::try_from(n).ok())
}

fn recognized_of(doc: &TantivyDocument, fields: &Fields) -> bool {
    first_u64(doc, fields.recognized) == Some(1)
}

fn modified_of(doc: &TantivyDocument, fields: &Fields) -> i64 {
    first_u64(doc, fields.modified).and_then(|n| i64::try_from(n).ok()).unwrap_or(0)
}

/// Every term the analyzer makes of a text.
fn terms_of(analyzer: &mut tantivy::tokenizer::TextAnalyzer, text: &str, into: &mut BTreeSet<String>) {
    let mut stream = analyzer.token_stream(text);
    while stream.advance() {
        into.insert(stream.token().text.clone());
    }
}

/// The words of a query that say something about it.
///
/// When no document holds every word, the search takes any word - and a word
/// nearly every chunk holds, *the*, *for*, *и*, then matches nearly every
/// document: a question of eight words where one is missing everywhere came
/// back as the notes that share its *the*. Such words are left out of that
/// search, and out of the count of how much of the question a chunk holds.
/// It is a judgement the library makes of itself, by counting, rather than a
/// list of stop words per language. A query made of nothing else is kept
/// whole; a word with syntax in it - a code, a quote - is never left out.
struct Distinctive {
    /// The query without its common words.
    query: String,
    /// The terms of what is left, as the index holds them.
    terms: BTreeSet<String>,
}

impl Distinctive {
    fn of(searcher: &tantivy::Searcher, body: tantivy::schema::Field, query: &str) -> Self {
        let chunks = searcher.num_docs();
        let mut analyzer = fulltext::analyzer();
        let mut common = |word: &str| {
            let word = word.trim_end_matches([',', '.', ';', '!', '?', '…']);
            if word.is_empty() || !word.chars().all(char::is_alphanumeric) {
                return false;
            }
            let mut terms = BTreeSet::new();
            terms_of(&mut analyzer, word, &mut terms);
            !terms.is_empty()
                && terms.iter().all(|term| {
                    let held = searcher.doc_freq(&Term::from_field_text(body, term)).unwrap_or(0);
                    held * 2 > chunks
                })
        };
        let kept: Vec<&str> = query.split_whitespace().filter(|word| !common(word)).collect();
        let query = if kept.is_empty() { query.to_string() } else { kept.join(" ") };
        let mut terms = BTreeSet::new();
        terms_of(&mut fulltext::analyzer(), &query, &mut terms);
        Self { query, terms }
    }
}

/// The share of these terms a chunk holds, in any field a query searches.
fn share_held(doc: &TantivyDocument, fields: &Fields, terms: &BTreeSet<String>) -> f32 {
    if terms.is_empty() {
        return 1.0;
    }
    let mut analyzer = fulltext::analyzer();
    let mut held = BTreeSet::new();
    for (field, _) in fields.searched() {
        for value in doc.get_all(field).filter_map(|v| v.as_str()) {
            terms_of(&mut analyzer, value, &mut held);
        }
    }
    terms.iter().filter(|term| held.contains(*term)).count() as f32 / terms.len() as f32
}

fn first_text(doc: &TantivyDocument, field: tantivy::schema::Field) -> String {
    doc.get_first(field).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

fn all_text(doc: &TantivyDocument, field: tantivy::schema::Field) -> Vec<String> {
    doc.get_all(field).filter_map(|v| v.as_str().map(str::to_string)).collect()
}

/// The start of a chunk, for a hit that matched by title or tags and has no
/// words in its text to centre a fragment on.
fn opening(body: &str) -> String {
    match body.char_indices().nth(FRAGMENT_CHARS) {
        Some((end, _)) => format!("{}…", body[..end].trim_end()),
        None => body.to_string(),
    }
}

fn add_document(writer: &IndexWriter, fields: &Fields, key: &str, entry: &FileEntry, doc: &Document, backlink_titles: &[&str]) -> Result<usize> {
    for chunk in &doc.chunks {
        let mut out = TantivyDocument::default();
        out.add_text(fields.path, key);
        out.add_text(fields.kind, entry.kind.name());
        out.add_text(fields.title, &doc.title);
        for alias in &doc.aliases {
            out.add_text(fields.aliases, alias);
        }
        for heading in &chunk.headings {
            out.add_text(fields.headings, heading);
        }
        out.add_text(fields.body, &chunk.text);
        for tag in &doc.tags {
            out.add_text(fields.tags, tag);
        }
        for link in &doc.links {
            out.add_text(fields.links, link);
        }
        for title in backlink_titles {
            out.add_text(fields.backlinks, *title);
        }
        out.add_field_value(fields.backlink_count, &OwnedValue::U64(entry.backlinks.len() as u64));
        out.add_u64(fields.line, u64::from(chunk.line));
        if let Some(page) = chunk.page {
            out.add_u64(fields.page, u64::from(page));
        }
        if chunk.recognized {
            out.add_u64(fields.recognized, 1);
        }
        out.add_u64(fields.modified, entry.modified / 1_000_000_000);
        writer.add_document(out).map_err(Error::fulltext)?;
    }
    Ok(doc.chunks.len())
}

/// Indexing shares the machine with whatever the person is doing. Half the
/// cores, at least one and at most four - for the full-text writer and for a
/// model computing vectors alike.
pub fn indexing_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    (cores / 2).clamp(1, 4)
}

/// Milliseconds since an instant, for a report.
fn millis_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The time now, in seconds since the epoch.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|e| Error::io(&temporary, e))?;
    std::fs::rename(&temporary, path).map_err(|e| Error::io(path, e))
}

/// A file found by the walk.
#[derive(Debug, Clone)]
struct WalkedFile {
    path: PathBuf,
    source: PathBuf,
    size: u64,
    modified: u64,
}

enum Found {
    /// Unchanged by size and time: its entry is carried over as it is.
    Kept(FileEntry),
    /// Needs opening to know.
    Candidate(WalkedFile),
}

enum Read {
    /// The bytes hash as before.
    Same(FileEntry),
    /// New or changed: the fresh entry and the parsed document.
    Changed(FileEntry, Document),
}

impl Unreadable {
    fn of(file: &WalkedFile, why: Why, reason: &str) -> Self {
        Self {
            size: Some(file.size),
            modified: file.modified,
            why,
            reason: reason.to_string(),
            by: TRIED_BY.to_string(),
        }
    }
}

/// The files under a source, as far as the scope reaches; what could not be
/// walked into goes to `unwalked`, with why.
fn walk(source: &Source, within: &Within, unwalked: &mut Vec<(PathBuf, String)>) -> Result<Vec<WalkedFile>> {
    let mut overrides = ignore::overrides::OverrideBuilder::new(&source.path);
    for pattern in &source.exclude {
        overrides
            .add(&format!("!{pattern}"))
            .map_err(|e| Error::BadPattern(pattern.clone(), e.to_string()))?;
    }
    let overrides = overrides.build().map_err(|e| Error::BadPattern(source.exclude.join(", "), e.to_string()))?;

    let mut walker = ignore::WalkBuilder::new(&source.path);
    walker
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .add_custom_ignore_filename(".nooma-ignore")
        .overrides(overrides)
        .follow_links(false);
    // Down to the changed paths and into them, every rule applying on the
    // way as it does on a whole walk.
    if let Within::Paths(_) = within {
        let within = within.clone();
        walker.filter_entry(move |entry| match entry.file_type().is_some_and(|kind| kind.is_dir()) {
            true => within.reaches(entry.path()),
            false => within.covers(entry.path()),
        });
    }

    let mut files = Vec::new();
    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                unwalked.push((path_of(&error).unwrap_or(&source.path).to_path_buf(), error.to_string()));
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if !reading::offered(entry.path()) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                unwalked.push((entry.path().to_path_buf(), error.to_string()));
                continue;
            }
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        files.push(WalkedFile {
            path: entry.path().to_path_buf(),
            source: source.path.clone(),
            size: metadata.len(),
            modified,
        });
    }
    Ok(files)
}

/// The path a walk failed at, when the error names one.
fn path_of(error: &ignore::Error) -> Option<&Path> {
    match error {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::Loop { child, .. } => Some(child),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => path_of(err),
        ignore::Error::Partial(errors) => errors.iter().find_map(path_of),
        _ => None,
    }
}

fn read_file(file: &WalkedFile, previous: Option<&FileEntry>) -> std::result::Result<Read, Unread> {
    let bytes = reading::load(&file.path)?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    if let Some(previous) = previous.filter(|p| p.hash == hash && p.source == file.source && p.reads_the_same()) {
        return Ok(Read::Same(previous.clone()));
    }
    let opened = reading::read(&file.path, bytes)?;
    let inside = file.path.strip_prefix(&file.source).unwrap_or(&file.path);
    // A vault links a note without its `.md` and any other file with its
    // extension: `[[plan]]` is plan.md, `![[manual.pdf]]` is the PDF, and
    // `[[manual]]` is not.
    let inside = match opened.kind {
        DocKind::Markdown => inside.with_extension(""),
        DocKind::Text | DocKind::Pdf | DocKind::Docx | DocKind::Epub | DocKind::Image => inside.to_path_buf(),
    };
    let rel = inside.to_string_lossy().replace('\\', "/").to_lowercase();
    let doc = opened.document;
    let textless = !opened.kind.has_lines() && doc.chunks.iter().all(|chunk| chunk.text.is_empty());
    let entry = FileEntry {
        source: file.source.clone(),
        size: file.size,
        modified: file.modified,
        hash,
        kind: opened.kind,
        title: doc.title.clone(),
        rel,
        links: doc.links.clone(),
        backlinks: Vec::new(),
        chunks: doc.chunks.len(),
        reader: opened.reader,
        textless,
        unrecognized: opened.unrecognized,
        guessed: opened.guessed.map(str::to_string),
    };
    Ok(Read::Changed(entry, doc))
}

/// Fill in each entry's backlinks from every other entry's links.
///
/// A link resolves within its own source, the way a vault resolves it: a bare
/// name by file name, a path by the end of the path. A name two files share
/// resolves to neither — pointing a backlink at the wrong note is worse than
/// leaving it out, since it ranks a note for words it was never linked by.
fn resolve_backlinks(entries: &mut BTreeMap<String, FileEntry>) {
    let mut by_name: BTreeMap<(&Path, String), Vec<&String>> = BTreeMap::new();
    for (key, entry) in entries.iter() {
        let name = entry.rel.rsplit('/').next().unwrap_or(&entry.rel).to_string();
        by_name.entry((entry.source.as_path(), name)).or_default().push(key);
    }

    let mut backlinks: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (key, entry) in entries.iter() {
        for link in &entry.links {
            let target = link.replace('\\', "/").trim_start_matches("./").to_lowercase();
            let target = target.strip_suffix(".md").unwrap_or(&target).to_string();
            let found: Vec<&String> = if target.contains('/') {
                entries
                    .iter()
                    .filter(|(_, other)| other.source == entry.source && (other.rel == target || other.rel.ends_with(&format!("/{target}"))))
                    .map(|(other_key, _)| other_key)
                    .collect()
            } else {
                let name = document::link_key(&target);
                by_name.get(&(entry.source.as_path(), name)).cloned().unwrap_or_default()
            };
            if let [target_key] = found.as_slice()
                && *target_key != key
            {
                backlinks.entry((*target_key).clone()).or_default().insert(key.clone());
            }
        }
    }

    for (key, entry) in entries.iter_mut() {
        entry.backlinks = backlinks.remove(key).map(|set| set.into_iter().collect()).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: &str, rel: &str, links: &[&str]) -> FileEntry {
        FileEntry {
            source: PathBuf::from(source),
            size: 0,
            modified: 0,
            hash: String::new(),
            kind: DocKind::Markdown,
            title: rel.to_string(),
            rel: rel.to_string(),
            links: links.iter().map(|l| l.to_string()).collect(),
            backlinks: Vec::new(),
            chunks: 1,
            reader: String::new(),
            textless: false,
            unrecognized: None,
            guessed: None,
        }
    }

    #[test]
    fn a_link_resolves_by_name_or_path_within_its_source() {
        let mut entries = BTreeMap::from([
            ("/v/plan".to_string(), entry("/v", "plan", &[])),
            ("/v/a".to_string(), entry("/v", "a", &["Plan"])),
            ("/v/b".to_string(), entry("/v", "b", &["dir/deep"])),
            ("/v/dir/deep".to_string(), entry("/v", "dir/deep", &[])),
            ("/w/c".to_string(), entry("/w", "c", &["plan"])),
        ]);
        resolve_backlinks(&mut entries);
        assert_eq!(entries["/v/plan"].backlinks, vec!["/v/a"], "the link from another source must not count");
        assert_eq!(entries["/v/dir/deep"].backlinks, vec!["/v/b"]);
    }

    #[test]
    fn a_name_two_notes_share_resolves_to_neither() {
        let mut entries = BTreeMap::from([
            ("/v/x/plan".to_string(), entry("/v", "x/plan", &[])),
            ("/v/y/plan".to_string(), entry("/v", "y/plan", &[])),
            ("/v/a".to_string(), entry("/v", "a", &["plan", "x/plan"])),
        ]);
        resolve_backlinks(&mut entries);
        assert!(entries["/v/y/plan"].backlinks.is_empty());
        assert_eq!(entries["/v/x/plan"].backlinks, vec!["/v/a"], "the path form is unambiguous");
    }

    #[test]
    fn a_note_linking_itself_is_not_its_own_backlink() {
        let mut entries = BTreeMap::from([("/v/a".to_string(), entry("/v", "a", &["a"]))]);
        resolve_backlinks(&mut entries);
        assert!(entries["/v/a"].backlinks.is_empty());
    }
}
