//! The vector half: every chunk of the library as a vector, one store per
//! model, and an index over them that finds the closest without reading all.
//!
//! # One directory per model
//!
//! A vector means something only next to vectors from the same weights. So
//! the store is named by the model's id — `vectors/multilingual-e5-small/` —
//! and a second model gets a second store beside the first rather than
//! replacing it. Two models' answers to the same query can then be compared
//! side by side over the same library, which is how a model is chosen: by
//! measuring, not by a leaderboard.
//!
//! # Keyed by the text, not the file
//!
//! A vector is stored under the hash of the exact text the model read — the
//! passage — and nothing else. Computing one is the expensive step in the
//! whole product, and keying it this way means it is computed once per
//! distinct passage: a renamed file, a note copied into two folders, a
//! document re-cut by a new chunker whose chunks mostly come out the same —
//! all find their vectors already there. The full-text index is rebuilt for
//! free when its format moves; this store is not, for the same reason prose
//! summaries are kept apart from the repository index.
//!
//! Vectors are appended as each batch is computed, not saved at the end: an
//! update stopped half way keeps the half it paid for. A record cut short by
//! a crash is recognised by length and dropped on the next open.
//!
//! # What was computed, and what is searched
//!
//! `vectors.bin` is the record of what the model computed: every passage it
//! has read, including ones no file holds any more until there are enough of
//! those to rewrite it. `index.bin` is built from it: the passages the
//! library holds now, in an approximate nearest-neighbour graph (`usearch`),
//! each beside the chunks it stands for. The first is expensive and is never
//! thrown away lightly; the second is derived, and is built again from the
//! first in seconds whenever it cannot be trusted.
//!
//! The index is one file, written whole and renamed into place: a reader sees
//! one state or the next, never the graph of one with the chunks of another.
//! It is read into memory rather than mapped - on Windows a mapped file
//! cannot be replaced, and a window holding it would hold off every update
//! made from the command line.
//!
//! A long first update saves the index as it goes, so search by meaning
//! answers over what has been read before the rest is. Passages are read in
//! the order of their documents, so what is covered is whole documents, and
//! the index says how many are not covered yet.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use usearch::{IndexOptions, MetricKind, ScalarKind};

use crate::embed::{Embedder, Model, Role};
use crate::error::{Error, Result};
use crate::library::{Finder, Hit, IndexedFile, Library, Progress, StoredChunk};

/// The format of a vector store: its `meta.json` and the record layout of
/// `vectors.bin`.
pub const VECTORS_FORMAT_VERSION: u32 = 1;

/// The format of `index.bin`, the graph and the chunks beside it.
pub const INDEX_FORMAT_VERSION: u32 = 1;

/// Passages handed to the model and written to disk together. Small: the
/// runtime's memory grows with the batch, a long update shares the machine
/// with its owner, and a search waiting for the model waits at most one
/// batch.
const BATCH: usize = 16;

/// Passages sorted by length together. A batch is padded to its longest
/// passage, and the model reads the padding as it reads text: mixing a
/// one-line note with a full chunk made it read the one line as five hundred
/// tokens. Sorting a window rather than everything keeps documents whole as
/// they are read.
const WINDOW: usize = 256;

/// Dead records tolerated before a store is rewritten without them.
const COMPACT_AFTER: usize = 1024;

/// How long an update runs before it saves the index so far, and again after
/// every as long, once the index has grown by a tenth.
const SAVE_AFTER: Duration = Duration::from_secs(60);

/// What `index.bin` starts with.
const INDEX_MAGIC: &[u8; 8] = b"NOOMAVIX";

/// How many candidates a search of the graph keeps while it walks: the
/// trade between finding what exact search finds and time. The graph's own
/// default, 64, missed one in ten of the closest ten over random vectors;
/// 256 missed three in a thousand, for a few milliseconds a search over tens
/// of thousands of passages - beside the tens of milliseconds the model takes
/// to read the question. `nooma eval` shows what it costs on real questions.
const EXPANSION_SEARCH: usize = 256;

/// A key: the BLAKE3 hash of a passage.
type Key = [u8; 32];

/// The graph from `usearch`.
type Graph = usearch::Index;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Meta {
    format_version: u32,
    model: String,
    dimensions: usize,
    /// What filled the store: see [`Model::recipe`].
    recipe: String,
}

impl Meta {
    fn of(model: &dyn Model) -> Self {
        Self {
            format_version: VECTORS_FORMAT_VERSION,
            model: model.model_id().to_string(),
            dimensions: model.dimensions(),
            recipe: model.recipe(),
        }
    }

    /// Why vectors stored under this meta cannot stand beside ones from
    /// `wanted`, or `None` if they can.
    fn mismatch(&self, wanted: &Meta) -> Option<String> {
        if self.format_version != wanted.format_version {
            return Some(format!(
                "the vectors are format {}, this nooma writes {}",
                self.format_version, wanted.format_version
            ));
        }
        if self.model != wanted.model || self.dimensions != wanted.dimensions {
            return Some(format!(
                "the store held {} with {} dimensions, not {} with {}",
                self.model, self.dimensions, wanted.model, wanted.dimensions
            ));
        }
        if self.recipe != wanted.recipe {
            return Some(format!("the vectors were computed as {}, not as {}", self.recipe, wanted.recipe));
        }
        None
    }
}

/// What bringing a model's vectors up to date did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VectorReport {
    /// The model.
    pub model: String,
    /// Chunks in the library.
    pub chunks: usize,
    /// Distinct passages among them; identical chunks share a vector.
    pub passages: usize,
    /// Passages the model read in this update.
    pub embedded: usize,
    /// Vectors of passages no chunk has any more, dropped from the store.
    pub dropped: usize,
    /// Why the store was started again, if it was.
    pub rebuilt: Option<String>,
    /// How long the update took, in milliseconds.
    pub took_ms: u64,
    /// How long the model spent reading, in milliseconds.
    pub embed_ms: u64,
}

/// The text a model reads for a chunk: the document's title, the headings
/// above the chunk, and the chunk.
///
/// A chunk alone is often a paragraph that makes sense only under its
/// heading; "Returns" under "Kettle" is about the kettle. A top heading that
/// repeats the title is said once.
pub fn passage(title: &str, headings: &[String], body: &str) -> String {
    let headings = match headings.split_first() {
        Some((first, rest)) if first.trim() == title.trim() => rest,
        _ => headings,
    };
    let mut out = String::with_capacity(title.len() + body.len() + 64);
    if !title.trim().is_empty() {
        out.push_str(title.trim());
        out.push('\n');
    }
    if !headings.is_empty() {
        out.push_str(&headings.join(" › "));
        out.push('\n');
    }
    out.push_str(body.trim());
    out
}

fn key_of(passage: &str) -> Key {
    *blake3::hash(passage.as_bytes()).as_bytes()
}

/// A model's store, open for writing.
struct Store {
    dir: PathBuf,
    dimensions: usize,
    keys: Vec<Key>,
    index: HashMap<Key, usize>,
    data: Vec<f32>,
    // Held for as long as the store is open: a second writer would
    // interleave its records with these.
    _lock: File,
}

impl Store {
    fn record_bytes(dimensions: usize) -> usize {
        32 + dimensions * 4
    }

    fn open(dir: PathBuf, wanted: &Meta) -> Result<(Self, Option<String>)> {
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let lock_path = dir.join(".lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| Error::io(&lock_path, e))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(Error::Busy),
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::io(&lock_path, e)),
        }

        let meta_path = dir.join("meta.json");
        let vectors_path = dir.join("vectors.bin");
        let stored = std::fs::read(&meta_path).ok();
        let meta: Option<Meta> = stored.as_deref().and_then(|bytes| serde_json::from_slice(bytes).ok());
        let rebuilt = match (&stored, &meta) {
            (_, Some(meta)) => meta.mismatch(wanted),
            // Vectors nobody can say the origin of are not kept, and the
            // report says so rather than starting over in silence.
            (Some(_), None) if vectors_path.exists() => Some("the store's description could not be read".to_string()),
            _ => None,
        };
        let dimensions = wanted.dimensions;
        if meta.is_none() || rebuilt.is_some() {
            let _ = std::fs::remove_file(&vectors_path);
            crate::library::write_atomically(&meta_path, &serde_json::to_vec_pretty(wanted).expect("meta serializes"))?;
        }

        let (keys, data) = read_records(&vectors_path, dimensions)?;
        // A crash mid-append leaves part of a record. It is cut off now, so
        // the next append starts on a record boundary.
        let whole = (keys.len() * Self::record_bytes(dimensions)) as u64;
        if let Ok(file) = File::options().write(true).open(&vectors_path)
            && file.metadata().is_ok_and(|meta| meta.len() != whole)
        {
            file.set_len(whole).map_err(|e| Error::io(&vectors_path, e))?;
        }
        let index = keys.iter().enumerate().map(|(i, key)| (*key, i)).collect();
        Ok((
            Self {
                dir,
                dimensions,
                keys,
                index,
                data,
                _lock: lock,
            },
            rebuilt,
        ))
    }

    fn contains(&self, key: &Key) -> bool {
        self.index.contains_key(key)
    }

    fn vector(&self, key: &Key) -> Option<&[f32]> {
        let at = *self.index.get(key)?;
        Some(&self.data[at * self.dimensions..(at + 1) * self.dimensions])
    }

    fn append(&mut self, keys: &[Key], vectors: &[Vec<f32>]) -> Result<()> {
        let path = self.dir.join("vectors.bin");
        let file = File::options().create(true).append(true).open(&path).map_err(|e| Error::io(&path, e))?;
        let mut out = BufWriter::new(file);
        for (key, vector) in keys.iter().zip(vectors) {
            if self.index.contains_key(key) {
                continue;
            }
            out.write_all(key).map_err(|e| Error::io(&path, e))?;
            for x in vector {
                out.write_all(&x.to_le_bytes()).map_err(|e| Error::io(&path, e))?;
            }
            self.index.insert(*key, self.keys.len());
            self.keys.push(*key);
            self.data.extend_from_slice(vector);
        }
        out.flush().map_err(|e| Error::io(&path, e))
    }

    /// Rewrite the store without the vectors no chunk uses, once there are
    /// enough of them to be worth it.
    fn compact(&mut self, live: &HashSet<Key>) -> Result<usize> {
        let dead = self.keys.iter().filter(|key| !live.contains(*key)).count();
        if dead <= COMPACT_AFTER || dead <= live.len() {
            return Ok(0);
        }
        let dims = self.dimensions;
        let mut keys = Vec::with_capacity(live.len());
        let mut data = Vec::with_capacity(live.len() * dims);
        let mut bytes = Vec::with_capacity(live.len() * Self::record_bytes(dims));
        for (i, key) in self.keys.iter().enumerate() {
            if live.contains(key) {
                let vector = &self.data[i * dims..(i + 1) * dims];
                bytes.extend_from_slice(key);
                for x in vector {
                    bytes.extend_from_slice(&x.to_le_bytes());
                }
                keys.push(*key);
                data.extend_from_slice(vector);
            }
        }
        crate::library::write_atomically(&self.dir.join("vectors.bin"), &bytes)?;
        self.index = keys.iter().enumerate().map(|(i, key)| (*key, i)).collect();
        self.keys = keys;
        self.data = data;
        Ok(dead)
    }
}

/// Read every whole record of a store; a missing file is an empty store.
fn read_records(path: &Path, dimensions: usize) -> Result<(Vec<Key>, Vec<f32>)> {
    let mut bytes = Vec::new();
    match File::open(path) {
        Ok(mut file) => {
            file.read_to_end(&mut bytes).map_err(|e| Error::io(path, e))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(e) => return Err(Error::io(path, e)),
    }
    let record = Store::record_bytes(dimensions);
    let count = bytes.len() / record;
    let mut keys = Vec::with_capacity(count);
    let mut data = Vec::with_capacity(count * dimensions);
    for chunk in bytes.chunks_exact(record) {
        let (key, rest) = chunk.split_at(32);
        keys.push(key.try_into().expect("32 bytes"));
        data.extend(rest.as_chunks::<4>().0.iter().map(|x| f32::from_le_bytes(*x)));
    }
    Ok((keys, data))
}

/// A document as the index knows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IndexedDoc {
    /// The file, as the full-text index names it.
    path: String,
    /// The hash of the file's bytes when its chunks were read.
    hash: String,
    /// Whether every chunk of it is in the graph.
    complete: bool,
}

/// The part of `index.bin` a person could read: what it was built by, and
/// over which documents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Head {
    model: String,
    recipe: String,
    dimensions: usize,
    documents: Vec<IndexedDoc>,
}

/// A passage in the graph, and the chunks it stands for: a document, by its
/// place in [`Head::documents`], and the line its chunk starts on.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    key: Key,
    chunks: Vec<(u32, u32)>,
}

fn graph_error(error: impl std::fmt::Display) -> Error {
    Error::VectorIndex(error.to_string())
}

/// An empty graph for vectors of this length.
///
/// Inner product on unit vectors is their cosine, so the distance it reports
/// is one minus the cosine; vectors are kept as they came, at full precision.
/// The graph's own defaults for its shape are used, and a wider search than
/// its default - see [`EXPANSION_SEARCH`]; the answer is checked against exact
/// search in this crate's tests.
fn new_graph(dimensions: usize) -> Result<Graph> {
    let options = IndexOptions {
        dimensions,
        metric: MetricKind::IP,
        quantization: ScalarKind::F32,
        connectivity: 0,
        expansion_add: 0,
        expansion_search: EXPANSION_SEARCH,
        multi: false,
    };
    Graph::new(&options).map_err(graph_error)
}

/// The contents of `index.bin`.
struct Snapshot {
    head: Head,
    /// By the passage's id in the graph.
    entries: BTreeMap<u64, Entry>,
    graph: Graph,
}

impl Snapshot {
    /// The file's bytes:
    ///
    /// ```text
    /// "NOOMAVIX"  u32 format
    /// u32 length  the head, as JSON
    /// u32 count   per passage: u64 id, 32-byte key, u32 chunks, per chunk u32 document, u32 line
    /// u64 length  the graph, as usearch saves it
    /// ```
    fn encode(head: &Head, entries: &BTreeMap<u64, Entry>, graph: &Graph) -> Result<Vec<u8>> {
        let head = serde_json::to_vec(head).expect("the head serializes");
        let mut saved = vec![0u8; graph.serialized_length()];
        graph.save_to_buffer(&mut saved).map_err(graph_error)?;
        let graph = saved;
        let mut out = Vec::with_capacity(head.len() + graph.len() + entries.len() * 56 + 32);
        out.extend_from_slice(INDEX_MAGIC);
        out.extend_from_slice(&INDEX_FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&u32::try_from(head.len()).map_err(graph_error)?.to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&u32::try_from(entries.len()).map_err(graph_error)?.to_le_bytes());
        for (id, entry) in entries {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&entry.key);
            out.extend_from_slice(&u32::try_from(entry.chunks.len()).map_err(graph_error)?.to_le_bytes());
            for (doc, line) in &entry.chunks {
                out.extend_from_slice(&doc.to_le_bytes());
                out.extend_from_slice(&line.to_le_bytes());
            }
        }
        out.extend_from_slice(&(graph.len() as u64).to_le_bytes());
        out.extend_from_slice(&graph);
        Ok(out)
    }

    /// Read `index.bin`; the error says what was wrong, for a person.
    fn decode(bytes: &[u8]) -> std::result::Result<Self, String> {
        let mut at = Cursor { bytes, at: 0 };
        if at.take(INDEX_MAGIC.len())? != INDEX_MAGIC {
            return Err("it is not a nooma vector index".to_string());
        }
        let format = at.u32()?;
        if format != INDEX_FORMAT_VERSION {
            return Err(format!("the index is format {format}, this nooma writes {INDEX_FORMAT_VERSION}"));
        }
        let head_len = at.u32()? as usize;
        let head: Head = serde_json::from_slice(at.take(head_len)?).map_err(|e| format!("its head could not be read: {e}"))?;
        let count = at.u32()? as usize;
        let mut entries = BTreeMap::new();
        for _ in 0..count {
            let id = at.u64()?;
            let key: Key = at.take(32)?.try_into().expect("32 bytes");
            let chunk_count = at.u32()? as usize;
            let mut chunks = Vec::with_capacity(chunk_count.min(1024));
            for _ in 0..chunk_count {
                let doc = at.u32()?;
                let line = at.u32()?;
                if doc as usize >= head.documents.len() {
                    return Err("a chunk names a document the index does not have".to_string());
                }
                chunks.push((doc, line));
            }
            entries.insert(id, Entry { key, chunks });
        }
        let graph_len = usize::try_from(at.u64()?).map_err(|e| e.to_string())?;
        let graph_bytes = at.take(graph_len)?;
        if at.at != bytes.len() {
            return Err("it has bytes after its end".to_string());
        }
        let graph = new_graph(head.dimensions).map_err(|e| e.to_string())?;
        graph.load_from_buffer(graph_bytes).map_err(|e| format!("its graph could not be read: {e}"))?;
        // What a file says about searching it is not trusted to be this
        // build's choice.
        graph.change_expansion_search(EXPANSION_SEARCH);
        if graph.size() != entries.len() || graph.dimensions() != head.dimensions {
            return Err("its graph and its passages disagree".to_string());
        }
        Ok(Self { head, entries, graph })
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> std::result::Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|end| *end <= self.bytes.len()).ok_or("it is cut short")?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> std::result::Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> std::result::Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }
}

/// The library's chunks as passages: which passage each chunk is, and which
/// documents they belong to.
struct Passages {
    /// Every distinct passage with its text, in the order of the first
    /// chunk that has it.
    texts: Vec<(Key, String)>,
    /// Per passage, the chunks it stands for.
    chunks: HashMap<Key, Vec<(u32, u32)>>,
    /// Per document, its path and hash.
    documents: Vec<(String, String)>,
}

impl Passages {
    fn of(chunks: &[StoredChunk], files: &BTreeMap<String, IndexedFile>) -> Self {
        let mut texts = Vec::new();
        let mut by_key: HashMap<Key, Vec<(u32, u32)>> = HashMap::new();
        let mut documents: Vec<(String, String)> = Vec::new();
        let mut document_of: HashMap<&str, u32> = HashMap::new();
        for chunk in chunks {
            let doc = *document_of.entry(chunk.path.as_str()).or_insert_with(|| {
                let hash = files.get(&chunk.path).map(|file| file.hash.clone()).unwrap_or_default();
                documents.push((chunk.path.clone(), hash));
                u32::try_from(documents.len() - 1).unwrap_or(u32::MAX)
            });
            let text = passage(&chunk.title, &chunk.headings, &chunk.body);
            let key = key_of(&text);
            let refs = by_key.entry(key).or_default();
            if refs.is_empty() {
                texts.push((key, text));
            }
            refs.push((doc, chunk.line));
        }
        Self {
            texts,
            chunks: by_key,
            documents,
        }
    }
}

/// The index while an update changes it.
struct Building {
    graph: Graph,
    /// Per passage in the graph, its id there.
    ids: HashMap<Key, u64>,
    next_id: u64,
    /// The head and passages of the file as it was read, to tell whether
    /// writing it again would change anything.
    saved: Option<(Head, BTreeMap<u64, Entry>)>,
}

impl Building {
    fn fresh(dimensions: usize) -> Result<Self> {
        Ok(Self {
            graph: new_graph(dimensions)?,
            ids: HashMap::new(),
            next_id: 0,
            saved: None,
        })
    }

    /// The index on disk, if it was built by these vectors; why not, if it
    /// was there and was not.
    fn open(path: &Path, meta: &Meta) -> Result<(Self, Option<String>)> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Self::fresh(meta.dimensions)?, None)),
            Err(e) => return Err(Error::io(path, e)),
        };
        let snapshot = match Snapshot::decode(&bytes) {
            Ok(snapshot) => snapshot,
            Err(reason) => return Ok((Self::fresh(meta.dimensions)?, Some(reason))),
        };
        if snapshot.head.model != meta.model || snapshot.head.recipe != meta.recipe || snapshot.head.dimensions != meta.dimensions {
            return Ok((Self::fresh(meta.dimensions)?, Some("it was built from other vectors".to_string())));
        }
        let ids = snapshot.entries.iter().map(|(id, entry)| (entry.key, *id)).collect();
        let next_id = snapshot.entries.keys().next_back().map_or(0, |id| id + 1);
        Ok((
            Self {
                graph: snapshot.graph,
                ids,
                next_id,
                saved: Some((snapshot.head, snapshot.entries)),
            },
            None,
        ))
    }

    fn add(&mut self, key: Key, vector: &[f32]) -> Result<()> {
        if self.ids.contains_key(&key) {
            return Ok(());
        }
        if self.graph.size() >= self.graph.capacity() {
            // Grown by half again: one reservation per vector would copy the
            // graph's tables every time.
            let wanted = (self.graph.capacity() + self.graph.capacity() / 2).max(self.graph.size() + BATCH).max(64);
            self.graph.reserve(wanted).map_err(graph_error)?;
        }
        let id = self.next_id;
        self.graph.add(id, vector).map_err(graph_error)?;
        self.ids.insert(key, id);
        self.next_id += 1;
        Ok(())
    }

    fn remove(&mut self, key: &Key) -> Result<()> {
        if let Some(id) = self.ids.remove(key) {
            self.graph.remove(id).map_err(graph_error)?;
        }
        Ok(())
    }

    /// What `index.bin` would hold now.
    fn contents(&self, meta: &Meta, passages: &Passages) -> (Head, BTreeMap<u64, Entry>) {
        let mut complete = vec![true; passages.documents.len()];
        let mut entries = BTreeMap::new();
        for (key, chunks) in &passages.chunks {
            match self.ids.get(key) {
                Some(id) => {
                    entries.insert(
                        *id,
                        Entry {
                            key: *key,
                            chunks: chunks.clone(),
                        },
                    );
                }
                None => {
                    for (doc, _) in chunks {
                        complete[*doc as usize] = false;
                    }
                }
            }
        }
        let documents = passages
            .documents
            .iter()
            .zip(complete)
            .map(|((path, hash), complete)| IndexedDoc {
                path: path.clone(),
                hash: hash.clone(),
                complete,
            })
            .collect();
        let head = Head {
            model: meta.model.clone(),
            recipe: meta.recipe.clone(),
            dimensions: meta.dimensions,
            documents,
        };
        (head, entries)
    }

    /// Write `index.bin`, unless it would say what it says already.
    fn save(&mut self, path: &Path, meta: &Meta, passages: &Passages) -> Result<bool> {
        let (head, entries) = self.contents(meta, passages);
        if self.saved.as_ref().is_some_and(|(h, e)| *h == head && *e == entries) {
            return Ok(false);
        }
        crate::library::write_atomically(path, &Snapshot::encode(&head, &entries, &self.graph)?)?;
        self.saved = Some((head, entries));
        Ok(true)
    }
}

impl Library {
    fn vectors_root(&self) -> PathBuf {
        self.root().join("vectors")
    }

    fn vectors_dir(&self, model: &str) -> PathBuf {
        self.vectors_root().join(model)
    }

    fn index_file(&self, model: &str) -> PathBuf {
        self.vectors_dir(model).join("index.bin")
    }

    /// Bring one model's vectors up to date with the full-text index: embed
    /// every passage the store does not have yet, and the search index after.
    ///
    /// The chunks are read from the full-text index, so it is updated first;
    /// a library whose index has to be rebuilt says so rather than embedding
    /// what is about to change. `progress` counts passages to embed.
    ///
    /// Fails with [`Error::Busy`] when another process is updating the same
    /// model's vectors.
    pub fn update_vectors(&self, embedder: &mut dyn Embedder, progress: &(dyn Fn(Progress) + Sync)) -> Result<VectorReport> {
        let started = Instant::now();
        let meta = Meta::of(&*embedder);
        let model = meta.model.clone();
        let chunks = self.stored_chunks()?;
        let passages = Passages::of(&chunks, &self.indexed_files()?);
        let (mut store, rebuilt) = Store::open(self.vectors_dir(&model), &meta)?;
        let index_path = self.index_file(&model);

        let live: HashSet<Key> = passages.chunks.keys().copied().collect();
        let mut wanted: Vec<&(Key, String)> = passages.texts.iter().filter(|(key, _)| !store.contains(key)).collect();
        for window in wanted.chunks_mut(WINDOW) {
            window.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.0.cmp(&b.0)));
        }

        // The index follows the store: passages no chunk has leave it, and
        // passages the store has and it does not - computed by an update
        // that stopped before saving it - join it.
        let (mut building, _) = Building::open(&index_path, &meta)?;
        let gone: Vec<Key> = building.ids.keys().filter(|key| !live.contains(*key)).copied().collect();
        if gone.len() > building.ids.len() / 4 {
            // A graph with many holes searches worse than one built again,
            // and building one over the store takes seconds.
            building = Building::fresh(meta.dimensions)?;
        } else {
            for key in &gone {
                building.remove(key)?;
            }
        }
        for (key, _) in &passages.texts {
            if let Some(vector) = store.vector(key) {
                building.add(*key, vector)?;
            }
        }

        let total = wanted.len();
        progress(Progress { done: 0, total });
        let mut embed = || -> Result<u128> {
            let mut embed_ms = 0u128;
            let mut last_save = Instant::now();
            let mut saved_size = building.graph.size();
            for (done, batch) in wanted.chunks(BATCH).enumerate() {
                let texts: Vec<&str> = batch.iter().map(|(_, text)| text.as_str()).collect();
                let keys: Vec<Key> = batch.iter().map(|(key, _)| *key).collect();
                let at = Instant::now();
                let vectors = embedder.embed(&texts, Role::Passage)?;
                embed_ms += at.elapsed().as_millis();
                if vectors.len() != texts.len() || vectors.iter().any(|v| v.len() != store.dimensions) {
                    return Err(Error::Embedding(format!("{model} returned vectors of the wrong number or length")));
                }
                store.append(&keys, &vectors)?;
                for (key, vector) in keys.iter().zip(&vectors) {
                    building.add(*key, vector)?;
                }
                progress(Progress {
                    done: (done * BATCH + batch.len()).min(total),
                    total,
                });
                let size = building.graph.size();
                if last_save.elapsed() >= SAVE_AFTER && size >= saved_size + saved_size / 10 {
                    building.save(&index_path, &meta, &passages)?;
                    last_save = Instant::now();
                    saved_size = size;
                }
            }
            Ok(embed_ms)
        };
        let embedded = embed();
        // What was computed is searchable even when the update stopped half
        // way: the next one would add it to the index anyway, and until then
        // a search by meaning covers what it can and says what it cannot.
        let saved = building.save(&index_path, &meta, &passages);
        let embed_ms = embedded?;
        saved?;
        let dropped = store.compact(&live)?;

        Ok(VectorReport {
            model,
            chunks: chunks.len(),
            passages: live.len(),
            embedded: total,
            dropped,
            rebuilt,
            took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            embed_ms: u64::try_from(embed_ms).unwrap_or(u64::MAX),
        })
    }

    /// One model's search index, read into memory; `None` when it has none,
    /// or one built from other vectors than this model makes.
    ///
    /// The model is named, not loaded: a catalogue entry will do, and the
    /// weights are needed only to turn a query into a vector.
    ///
    /// The index may be behind the full-text index - an update of the
    /// vectors still running, or files changed since the last - and says by
    /// how many documents: see [`SemanticIndex::behind`]. Documents removed
    /// since it was built are never found by it.
    pub fn semantic(&self, model: &dyn Model) -> Result<Option<SemanticIndex>> {
        let meta = Meta::of(model);
        let path = self.index_file(&meta.model);
        let (bytes, stamp) = match read_stamped(&path) {
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(&path, e)),
        };
        // An index that cannot be read is built again by the next update,
        // which says why; until then there is nothing to search.
        let Ok(snapshot) = Snapshot::decode(&bytes) else {
            return Ok(None);
        };
        if snapshot.head.model != meta.model || snapshot.head.recipe != meta.recipe || snapshot.head.dimensions != meta.dimensions {
            return Ok(None);
        }

        let current = self.indexed_files()?;
        let by_path: HashMap<&str, &IndexedDoc> = snapshot.head.documents.iter().map(|doc| (doc.path.as_str(), doc)).collect();
        let behind = current
            .iter()
            .filter(|(path, file)| file.chunks > 0 && !by_path.get(path.as_str()).is_some_and(|doc| doc.complete && doc.hash == file.hash))
            .count();
        let present = snapshot.head.documents.iter().map(|doc| current.contains_key(&doc.path)).collect();
        let passages = snapshot.entries.len();
        Ok(Some(SemanticIndex {
            model: meta.model,
            dimensions: meta.dimensions,
            graph: snapshot.graph,
            chunks: snapshot.entries.into_iter().map(|(id, entry)| (id, entry.chunks)).collect(),
            documents: snapshot.head.documents.into_iter().map(|doc| PathBuf::from(doc.path)).collect(),
            present,
            behind,
            passages,
            file: path,
            stamp,
        }))
    }

    /// Delete one model's vectors and its index. Returns whether there were
    /// any.
    pub fn remove_vectors(&self, model: &str) -> Result<bool> {
        let dir = self.vectors_dir(model);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::io(&dir, e)),
        }
    }
}

/// A file's size and modification time: enough to tell that it was replaced.
type Stamp = (u64, SystemTime);

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Read a file whole, with the stamp it had when it was read.
fn read_stamped(path: &Path) -> std::io::Result<(Vec<u8>, Option<Stamp>)> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let stamp = meta.modified().ok().map(|modified| (meta.len(), modified));
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.read_to_end(&mut bytes)?;
    Ok((bytes, stamp))
}

/// One chunk found by meaning: where it is, and how close.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticHit {
    /// The file.
    pub path: PathBuf,
    /// The line the chunk starts on.
    pub line: u32,
    /// The cosine between the query and the chunk: 1 is the same direction,
    /// 0 unrelated.
    pub score: f32,
}

/// A model's search index over the library, in memory.
pub struct SemanticIndex {
    model: String,
    dimensions: usize,
    graph: Graph,
    /// Per passage id, its chunks: a document and a line.
    chunks: HashMap<u64, Vec<(u32, u32)>>,
    documents: Vec<PathBuf>,
    /// Per document, whether the library still holds it.
    present: Vec<bool>,
    behind: usize,
    passages: usize,
    file: PathBuf,
    stamp: Option<Stamp>,
}

impl std::fmt::Debug for SemanticIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticIndex")
            .field("model", &self.model)
            .field("passages", &self.passages)
            .field("behind", &self.behind)
            .finish_non_exhaustive()
    }
}

impl SemanticIndex {
    /// The model the vectors came from.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Passages in the index.
    pub fn len(&self) -> usize {
        self.passages
    }

    /// Whether it holds no passages.
    pub fn is_empty(&self) -> bool {
        self.passages == 0
    }

    /// Documents of the library the index does not cover yet: not in it at
    /// all, in it in part, or in it as an older version of the file.
    pub fn behind(&self) -> usize {
        self.behind
    }

    /// Whether the file it was read from is still the one on disk. An update
    /// replaces it; a caller keeping an index open reads it again when this
    /// turns false.
    pub fn is_current(&self) -> bool {
        self.stamp.is_some() && stamp_of(&self.file) == self.stamp
    }

    /// The chunks closest to a query vector, at most one per document,
    /// closest first.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<SemanticHit>> {
        self.closest(query, limit, false)
    }

    /// [`SemanticIndex::search`] by comparing the query with every vector
    /// rather than walking the graph: the answer the graph approximates, for
    /// measuring how closely it does.
    pub fn search_exact(&self, query: &[f32], limit: usize) -> Result<Vec<SemanticHit>> {
        self.closest(query, limit, true)
    }

    fn closest(&self, query: &[f32], limit: usize, exact: bool) -> Result<Vec<SemanticHit>> {
        let size = self.graph.size();
        if limit == 0 || size == 0 {
            return Ok(Vec::new());
        }
        if query.len() != self.dimensions {
            return Err(Error::Embedding(format!(
                "a query vector of {} components, where {} has {}",
                query.len(),
                self.model,
                self.dimensions
            )));
        }
        // Several chunks of one document can be closer than any chunk of the
        // next, so more chunks are asked for than documents wanted, and more
        // again when they fall on too few documents.
        let mut count = limit.saturating_mul(4).saturating_add(16).min(size);
        loop {
            let found = match exact {
                false => self.graph.search(query, count),
                true => self.graph.exact_search(query, count),
            }
            .map_err(graph_error)?;
            let mut best: HashMap<u32, (f32, u32)> = HashMap::new();
            for (id, distance) in found.keys.iter().zip(&found.distances) {
                let score = 1.0 - distance;
                for &(doc, line) in self.chunks.get(id).map_or(&[][..], Vec::as_slice) {
                    if !self.present[doc as usize] {
                        continue;
                    }
                    let held = best.entry(doc).or_insert((f32::NEG_INFINITY, line));
                    if score > held.0 || (score == held.0 && line < held.1) {
                        *held = (score, line);
                    }
                }
            }
            if best.len() >= limit || count >= size {
                let mut ranked: Vec<(u32, f32, u32)> = best.into_iter().map(|(doc, (score, line))| (doc, score, line)).collect();
                // Ties go to the path, so two runs over the same library agree.
                ranked.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then_with(|| self.documents[a.0 as usize].cmp(&self.documents[b.0 as usize]))
                });
                return Ok(ranked
                    .into_iter()
                    .take(limit)
                    .map(|(doc, score, line)| SemanticHit {
                        path: self.documents[doc as usize].clone(),
                        line,
                        score,
                    })
                    .collect());
            }
            count = count.saturating_mul(4).min(size);
        }
    }

    /// The documents closest to a vector, as hits read from the full-text
    /// index: the chunk's title, headings and opening, the score a cosine.
    ///
    /// `skip` leaves one file out - the one an example was taken from, which
    /// would otherwise be the closest to itself.
    pub fn hits(&self, finder: &Finder, vector: &[f32], limit: usize, skip: Option<&Path>) -> Result<Vec<Hit>> {
        let wanted = limit + usize::from(skip.is_some());
        let found: Vec<SemanticHit> = self
            .search(vector, wanted)?
            .into_iter()
            .filter(|hit| skip.is_none_or(|skip| hit.path != skip))
            .take(limit)
            .collect();
        let at: Vec<(PathBuf, u32)> = found.iter().map(|hit| (hit.path.clone(), hit.line)).collect();
        Ok(finder
            .chunks(&at)?
            .into_iter()
            .zip(&found)
            .filter_map(|(hit, semantic)| {
                hit.map(|mut hit| {
                    hit.score = semantic.score;
                    hit
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(model: &str, dimensions: usize) -> Meta {
        Meta {
            format_version: VECTORS_FORMAT_VERSION,
            model: model.to_string(),
            dimensions,
            recipe: model.to_string(),
        }
    }

    #[test]
    fn a_store_filled_by_another_recipe_is_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m");
        {
            let (mut store, _) = Store::open(path.clone(), &meta("m", 2)).unwrap();
            store.append(&[[1; 32]], &[vec![1.0, 0.0]]).unwrap();
        }
        let mut other = meta("m", 2);
        other.recipe = "m max_tokens=256".to_string();
        let (store, rebuilt) = Store::open(path, &other).unwrap();
        assert!(rebuilt.is_some_and(|reason| reason.contains("max_tokens=256")));
        assert!(store.keys.is_empty(), "a vector cut at another length is another vector");
    }

    #[test]
    fn a_top_heading_that_repeats_the_title_is_said_once() {
        let headings = vec!["Kettle".to_string(), "Returns".to_string()];
        assert_eq!(passage("Kettle", &headings, " Bring it back. "), "Kettle\nReturns\nBring it back.");
        let other = vec!["Care".to_string(), "Descaling".to_string()];
        assert_eq!(passage("Kettle", &other, "Vinegar."), "Kettle\nCare › Descaling\nVinegar.");
        assert_eq!(passage("", &[], "Just text."), "Just text.");
    }

    #[test]
    fn a_torn_last_record_is_dropped_and_cut_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m");
        {
            let (mut store, _) = Store::open(path.clone(), &meta("m", 2)).unwrap();
            store.append(&[[1; 32], [2; 32]], &[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap();
        }
        // Half of a third record, as a crash mid-write leaves it.
        let mut file = File::options().append(true).open(path.join("vectors.bin")).unwrap();
        file.write_all(&[3; 20]).unwrap();
        drop(file);
        let (store, _) = Store::open(path.clone(), &meta("m", 2)).unwrap();
        assert_eq!(store.keys, vec![[1; 32], [2; 32]]);
        assert_eq!(std::fs::metadata(path.join("vectors.bin")).unwrap().len(), 2 * 40);
    }

    #[test]
    fn a_store_of_another_model_or_length_is_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m");
        {
            let (mut store, _) = Store::open(path.clone(), &meta("m", 2)).unwrap();
            store.append(&[[1; 32]], &[vec![1.0, 0.0]]).unwrap();
        }
        let (store, rebuilt) = Store::open(path.clone(), &meta("m", 3)).unwrap();
        assert!(rebuilt.is_some());
        assert!(store.keys.is_empty(), "two-component vectors read as three would be garbage");
    }

    #[test]
    fn a_second_writer_is_told_the_store_is_busy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m");
        let (_held, _) = Store::open(path.clone(), &meta("m", 2)).unwrap();
        assert!(matches!(Store::open(path, &meta("m", 2)), Err(Error::Busy)));
    }

    #[test]
    fn compaction_waits_for_enough_dead_records_and_keeps_the_live_ones() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = Store::open(dir.path().join("m"), &meta("m", 1)).unwrap();
        let keys: Vec<Key> = (0..2100u32).map(|i| *blake3::hash(&i.to_le_bytes()).as_bytes()).collect();
        let vectors: Vec<Vec<f32>> = (0..2100).map(|i| vec![i as f32]).collect();
        store.append(&keys, &vectors).unwrap();

        let few_dead: HashSet<Key> = keys[..1500].iter().copied().collect();
        assert_eq!(store.compact(&few_dead).unwrap(), 0, "600 dead is not worth a rewrite");

        let live: HashSet<Key> = keys[..10].iter().copied().collect();
        assert_eq!(store.compact(&live).unwrap(), 2090);
        assert_eq!(store.keys.len(), 10);
        drop(store);
        let (reopened, _) = Store::open(dir.path().join("m"), &meta("m", 1)).unwrap();
        assert_eq!(reopened.keys, keys[..10].to_vec());
        assert_eq!(reopened.data, (0..10).map(|i| i as f32).collect::<Vec<_>>());
    }

    /// Unit vectors from a seed, without a random-number crate: the bytes of
    /// a hash, centred and normalized.
    fn unit_vector(seed: u32, dimensions: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(dimensions);
        let mut block = 0u32;
        while out.len() < dimensions {
            let hash = blake3::hash(&[seed.to_le_bytes(), block.to_le_bytes()].concat());
            out.extend(hash.as_bytes().iter().map(|b| f32::from(*b) - 127.5));
            block += 1;
        }
        out.truncate(dimensions);
        crate::embed::normalize(&mut out);
        out
    }

    /// The graph answers approximately, and the claim is that the
    /// approximation does not change the answer a person sees. Measured
    /// against exact search over the same vectors: of the ten closest, the
    /// graph finds nearly all.
    #[test]
    fn the_graph_finds_what_exact_search_finds() {
        let dimensions = 64;
        let graph = new_graph(dimensions).unwrap();
        graph.reserve(5000).unwrap();
        let vectors: Vec<Vec<f32>> = (0..5000).map(|i| unit_vector(i, dimensions)).collect();
        for (i, vector) in vectors.iter().enumerate() {
            graph.add(i as u64, vector).unwrap();
        }
        let mut found = 0usize;
        let queries = 200;
        for q in 0..queries {
            let query = unit_vector(1_000_000 + q, dimensions);
            let mut exact: Vec<(f32, u64)> = vectors
                .iter()
                .enumerate()
                .map(|(i, v)| (v.iter().zip(&query).map(|(a, b)| a * b).sum::<f32>(), i as u64))
                .collect();
            exact.sort_by(|a, b| b.0.total_cmp(&a.0));
            let truth: HashSet<u64> = exact.iter().take(10).map(|(_, id)| *id).collect();
            let approximate = graph.search(&query, 10).unwrap();
            found += approximate.keys.iter().filter(|id| truth.contains(id)).count();
            // The distance is one minus the cosine: the score a hit carries.
            let first = &vectors[approximate.keys[0] as usize];
            let cosine: f32 = first.iter().zip(&query).map(|(a, b)| a * b).sum();
            assert!(
                (1.0 - approximate.distances[0] - cosine).abs() < 1e-4,
                "{} against {cosine}",
                1.0 - approximate.distances[0]
            );
        }
        let recall = found as f64 / (queries * 10) as f64;
        assert!(recall >= 0.95, "recall at ten: {recall:.3}");
    }

    #[test]
    fn an_index_file_reads_back_as_it_was_written_and_a_damaged_one_does_not() {
        let graph = new_graph(4).unwrap();
        graph.reserve(8).unwrap();
        graph.add(3, &[1.0, 0.0, 0.0, 0.0]).unwrap();
        graph.add(7, &[0.0, 1.0, 0.0, 0.0]).unwrap();
        let head = Head {
            model: "m".to_string(),
            recipe: "m".to_string(),
            dimensions: 4,
            documents: vec![IndexedDoc {
                path: "/notes/a.md".to_string(),
                hash: "h".to_string(),
                complete: true,
            }],
        };
        let entries = BTreeMap::from([
            (
                3,
                Entry {
                    key: [1; 32],
                    chunks: vec![(0, 1)],
                },
            ),
            (
                7,
                Entry {
                    key: [2; 32],
                    chunks: vec![(0, 5), (0, 9)],
                },
            ),
        ]);
        let bytes = Snapshot::encode(&head, &entries, &graph).unwrap();

        let read = Snapshot::decode(&bytes).unwrap();
        assert_eq!(read.head, head);
        assert_eq!(read.entries, entries);
        assert_eq!(read.graph.search(&[0.0f32, 1.0, 0.0, 0.0][..], 1).unwrap().keys, vec![7]);

        assert!(Snapshot::decode(&bytes[..bytes.len() - 1]).is_err(), "a file cut short");
        assert!(Snapshot::decode(&[bytes.as_slice(), &[0]].concat()).is_err(), "a file with more after its end");
        let mut other = bytes.clone();
        other[8] = 99;
        assert!(Snapshot::decode(&other).err().unwrap().contains("format 99"));
    }
}
