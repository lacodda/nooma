//! A library: the folders a person searches, and the index kept over them.
//!
//! The repository index answers "what is in this repository"; a library
//! answers "where did I write about this". It has sources — folders — and one
//! full-text index over every text document under them, kept on disk between
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

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::{Query, QueryParser};
use tantivy::schema::{OwnedValue, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::{DocAddress, DocId, Index, IndexWriter, Score, SegmentReader, TantivyDocument, Term};

use crate::document::{self, DocKind, Document};
use crate::error::{Error, Result};
use crate::fulltext::{self, Fields};

/// The format of `sources.json`.
pub const SOURCES_FORMAT_VERSION: u32 = 1;

/// The format of the full-text index and its manifest.
///
/// Bumped when the schema, the analyzer or the manifest's shape changes. A
/// library of another format is not read: the next update throws the index
/// away and builds it again, and says so. Words stemmed by other rules would
/// otherwise be looked up by these ones and quietly not match.
pub const FULLTEXT_FORMAT_VERSION: u32 = 1;

/// How documents are cut into chunks.
///
/// Held apart from the format for the reason [`crate::index::CHUNKER_VERSION`]
/// is: a file's bytes and hash do not change when the rules for cutting it do,
/// so without this number an index built by the old rules would be kept.
pub const DOC_CHUNKER_VERSION: u32 = 1;

/// Files larger than this are not read. A text file this size is a log or a
/// dump, and reading it would hold up every other file behind it.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// How much memory the index writer may use while indexing.
const WRITER_MEMORY: usize = 64 * 1024 * 1024;

/// Characters in a result's fragment.
const FRAGMENT_CHARS: usize = 240;

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
    title: String,
    /// Path inside the source, lowercased, `/`-separated, without extension:
    /// what a `[[folder/note]]` link is matched against.
    rel: String,
    /// The notes this one links to, as written.
    links: Vec<String>,
    /// The files that link to this one, by path.
    backlinks: Vec<String>,
    chunks: usize,
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
    /// Sources whose folder could not be reached; their documents were kept.
    pub unavailable: Vec<PathBuf>,
    /// Why the whole index was thrown away and built again, if it was.
    pub rebuilt: Option<String>,
    /// How long the update took, in milliseconds.
    pub took_ms: u64,
}

/// A file an update found and could not read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// The file.
    pub path: PathBuf,
    /// Why, said for a person.
    pub reason: String,
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
    /// When the last update finished, as seconds since the epoch; `None`
    /// before the first.
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
    /// `markdown` or `text`.
    pub kind: String,
    /// The document's title.
    pub title: String,
    /// The headings above the matching chunk, outermost first.
    pub headings: Vec<String>,
    /// The line the matching chunk starts on.
    pub line: u32,
    /// A piece of the matching chunk around the words that matched.
    pub fragment: String,
    /// Where in `fragment` the matching words are, as byte ranges.
    #[serde(serialize_with = "ranges_as_pairs")]
    pub highlights: Vec<Range<usize>>,
    /// The document's tags.
    pub tags: Vec<String>,
    /// How well it matched. Comparable within one search, not across two.
    pub score: f32,
}

fn ranges_as_pairs<S: serde::Serializer>(ranges: &[Range<usize>], serializer: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(ranges.len()))?;
    for range in ranges {
        seq.serialize_element(&[range.start, range.end])?;
    }
    seq.end()
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
        let dirs = directories::ProjectDirs::from("com", "lacodda", "nooma")
            .ok_or_else(|| Error::io(PathBuf::from("."), std::io::Error::other("no home directory on this system")))?;
        Self::open_at(dirs.data_dir().join("library"))
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

    fn index_dir(&self) -> PathBuf {
        self.root.join("fulltext")
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
        let started = Instant::now();
        let mut report = UpdateReport::default();

        let stored = self.load_manifest()?;
        let (index, fields, mut manifest) = match (self.staleness(stored.as_ref()), stored) {
            (None, Some(manifest)) => match self.open_index() {
                Ok((index, fields)) => (index, fields, manifest),
                Err(error) => {
                    report.rebuilt = Some(format!("the stored index could not be opened: {error}"));
                    let (index, fields) = self.create_index()?;
                    (index, fields, Manifest::default())
                }
            },
            (reason, _) => {
                // No manifest at all is a first run, not a rebuild worth
                // announcing; a manifest of another format is.
                report.rebuilt = reason;
                let (index, fields) = self.create_index()?;
                (index, fields, Manifest::default())
            }
        };

        let mut writer: IndexWriter = match index.writer_with_num_threads(indexing_threads(), WRITER_MEMORY) {
            Ok(writer) => writer,
            Err(tantivy::TantivyError::LockFailure(..)) => return Err(Error::Busy),
            Err(error) => return Err(Error::fulltext(error)),
        };

        // Walk every source: what is there now, and what can be skipped
        // without opening it.
        let mut found: BTreeMap<String, Found> = BTreeMap::new();
        for source in &self.sources {
            if !source.path.is_dir() {
                report.unavailable.push(source.path.clone());
                for (key, entry) in &manifest.files {
                    if entry.source == source.path {
                        found.insert(key.clone(), Found::Kept(entry.clone()));
                    }
                }
                continue;
            }
            for file in walk(source, &mut report.skipped)? {
                let key = file.path.to_string_lossy().into_owned();
                let unchanged = manifest
                    .files
                    .get(&key)
                    .filter(|entry| entry.size == file.size && entry.modified == file.modified && entry.source == source.path);
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
        progress(Progress { done: 0, total });
        let read: Vec<(String, std::result::Result<Read, String>)> = candidates
            .par_iter()
            .map(|file| {
                let key = file.path.to_string_lossy().into_owned();
                let previous = manifest.files.get(&key);
                let result = read_file(file, previous);
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress(Progress { done, total });
                (key, result)
            })
            .collect();

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
                    entries.insert(key.clone(), entry);
                    parsed.insert(key, doc);
                }
                Err(reason) => report.skipped.push(Skipped {
                    path: file.path.clone(),
                    reason,
                }),
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

        for key in &removed {
            writer.delete_term(Term::from_field_text(fields.path, key));
        }
        report.removed = removed.len();

        let mut dropped = Vec::new();
        for key in &reindex {
            let Some(entry) = entries.get(key) else { continue };
            let doc = match parsed.remove(key) {
                Some(doc) => doc,
                None => match reread(Path::new(key)) {
                    Ok(doc) => doc,
                    Err(reason) => {
                        report.skipped.push(Skipped {
                            path: PathBuf::from(key),
                            reason,
                        });
                        dropped.push(key.clone());
                        writer.delete_term(Term::from_field_text(fields.path, key));
                        continue;
                    }
                },
            };
            writer.delete_term(Term::from_field_text(fields.path, key));
            let backlink_titles: Vec<&str> = entry.backlinks.iter().filter_map(|k| entries.get(k)).map(|e| e.title.as_str()).collect();
            report.chunks += add_document(&writer, &fields, key, entry, &doc, &backlink_titles)?;
            report.indexed += 1;
        }
        for key in dropped {
            entries.remove(&key);
        }

        writer.commit().map_err(Error::fulltext)?;

        manifest.format_version = FULLTEXT_FORMAT_VERSION;
        manifest.chunker_version = DOC_CHUNKER_VERSION;
        manifest.indexed_at = Some(unix_now());
        manifest.files = entries;
        let bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
        write_atomically(&self.manifest_path(), &bytes)?;
        // The writer holds tantivy's lock; dropping it only after the manifest
        // is on disk keeps a second updater from reading a manifest that is
        // behind the index it is about to extend.
        writer.wait_merging_threads().map_err(Error::fulltext)?;

        report.documents = manifest.files.len();
        report.took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }

    /// Find the documents that contain the words of a query.
    ///
    /// Every word has to occur, in the text or the title, tags, headings or
    /// links; when no document holds them all, documents holding any of them
    /// are returned instead, ranked by how many and how rare. An empty result
    /// for a four-word query where three words match is the answer a person
    /// takes for "search is broken".
    ///
    /// One hit per document, at its best chunk.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Hit>> {
        let manifest = self.load_manifest()?;
        if let Some(reason) = self.staleness(manifest.as_ref()) {
            return Err(Error::LibraryStale(reason));
        }
        if manifest.is_none() || query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let (index, fields) = self.open_index()?;
        let reader = index.reader().map_err(Error::fulltext)?;
        let searcher = reader.searcher();

        let mut parser = QueryParser::for_index(&index, fields.searched().iter().map(|(field, _)| *field).collect());
        for (field, boost) in fields.searched() {
            parser.set_field_boost(field, boost);
        }
        let any = parser.parse_query_lenient(query).0;
        parser.set_conjunction_by_default();
        let all = parser.parse_query_lenient(query).0;

        let mut hits = self.collect(&searcher, &fields, &*all, limit)?;
        if hits.is_empty() {
            hits = self.collect(&searcher, &fields, &*any, limit)?;
        }
        Ok(hits)
    }

    fn collect(&self, searcher: &tantivy::Searcher, fields: &Fields, query: &dyn Query, limit: usize) -> Result<Vec<Hit>> {
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
            hits.push(Hit {
                path: PathBuf::from(path),
                kind: first_text(&doc, fields.kind),
                title: first_text(&doc, fields.title),
                headings: all_text(&doc, fields.headings),
                line: doc
                    .get_first(fields.line)
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(1),
                fragment,
                highlights,
                tags: all_text(&doc, fields.tags),
                score,
            });
            if hits.len() == limit {
                break;
            }
        }
        Ok(hits)
    }
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
    let kind = DocKind::of(Path::new(key)).map_or("text", DocKind::name);
    for chunk in &doc.chunks {
        let mut out = TantivyDocument::default();
        out.add_text(fields.path, key);
        out.add_text(fields.kind, kind);
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
        writer.add_document(out).map_err(Error::fulltext)?;
    }
    Ok(doc.chunks.len())
}

/// Indexing shares the machine with whatever the person is doing. Half the
/// cores, at least one and at most four.
fn indexing_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    (cores / 2).clamp(1, 4)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
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

fn walk(source: &Source, skipped: &mut Vec<Skipped>) -> Result<Vec<WalkedFile>> {
    let mut overrides = ignore::overrides::OverrideBuilder::new(&source.path);
    for pattern in &source.exclude {
        overrides
            .add(&format!("!{pattern}"))
            .map_err(|e| Error::BadPattern(pattern.clone(), e.to_string()))?;
    }
    let overrides = overrides.build().map_err(|e| Error::BadPattern(source.exclude.join(", "), e.to_string()))?;

    let walker = ignore::WalkBuilder::new(&source.path)
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .add_custom_ignore_filename(".nooma-ignore")
        .overrides(overrides)
        .follow_links(false)
        .build();

    let mut files = Vec::new();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                skipped.push(Skipped {
                    path: source.path.clone(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) || DocKind::of(entry.path()).is_none() {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                skipped.push(Skipped {
                    path: entry.path().to_path_buf(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES {
            skipped.push(Skipped {
                path: entry.path().to_path_buf(),
                reason: format!("larger than {} MB", MAX_FILE_BYTES / 1024 / 1024),
            });
            continue;
        }
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

fn read_file(file: &WalkedFile, previous: Option<&FileEntry>) -> std::result::Result<Read, String> {
    let bytes = std::fs::read(&file.path).map_err(|e| e.to_string())?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    if let Some(previous) = previous.filter(|p| p.hash == hash && p.source == file.source) {
        return Ok(Read::Same(previous.clone()));
    }
    let doc = parse(&file.path, &bytes)?;
    let rel = file
        .path
        .strip_prefix(&file.source)
        .unwrap_or(&file.path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let entry = FileEntry {
        source: file.source.clone(),
        size: file.size,
        modified: file.modified,
        hash,
        title: doc.title.clone(),
        rel,
        links: doc.links.clone(),
        backlinks: Vec::new(),
        chunks: doc.chunks.len(),
    };
    Ok(Read::Changed(entry, doc))
}

fn reread(path: &Path) -> std::result::Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    parse(path, &bytes)
}

fn parse(path: &Path, bytes: &[u8]) -> std::result::Result<Document, String> {
    let kind = DocKind::of(path).ok_or_else(|| "not a text document".to_string())?;
    let text = decode(bytes).ok_or_else(|| "not UTF-8 or UTF-16 text".to_string())?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(document::read(kind, &stem, &text))
}

/// A file's text: UTF-8, or UTF-16 when it says so with a byte order mark —
/// what Windows Notepad saved for years. Anything else is not guessed at; a
/// wrong guess indexes gibberish that no query will ever match, and a skip at
/// least says so.
fn decode(bytes: &[u8]) -> Option<String> {
    let utf16 = |bytes: &[u8], read: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| read(*pair)).collect();
        String::from_utf16(&units).ok()
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
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

    #[test]
    fn utf16_with_a_mark_is_text_and_unmarked_garbage_is_not() {
        let mut le = vec![0xFF, 0xFE];
        for unit in "привет".encode_utf16() {
            le.extend(unit.to_le_bytes());
        }
        assert_eq!(decode(&le).as_deref(), Some("привет"));
        // Windows-1251 "привет": not UTF-8, and not guessed at.
        assert_eq!(decode(&[0xEF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]), None);
    }

    fn entry(source: &str, rel: &str, links: &[&str]) -> FileEntry {
        FileEntry {
            source: PathBuf::from(source),
            size: 0,
            modified: 0,
            hash: String::new(),
            title: rel.to_string(),
            rel: rel.to_string(),
            links: links.iter().map(|l| l.to_string()).collect(),
            backlinks: Vec::new(),
            chunks: 1,
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
