//! The vector half: every chunk of the library as a vector, one store per
//! model.
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
//! # Search
//!
//! Exact: the query against every chunk, best chunk per document. An
//! approximate index is the next step, and changes how fast the answer
//! comes, not what it is.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::embed::{Embedder, Role};
use crate::error::{Error, Result};
use crate::library::{Library, Progress};

/// The format of a vector store: its `meta.json` and the record layout of
/// `vectors.bin`.
pub const VECTORS_FORMAT_VERSION: u32 = 1;

/// Passages handed to the model and written to disk together.
const BATCH: usize = 64;

/// Dead records tolerated before a store is rewritten without them.
const COMPACT_AFTER: usize = 1024;

/// A key: the BLAKE3 hash of a passage.
type Key = [u8; 32];

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    format_version: u32,
    model: String,
    dimensions: usize,
    /// What filled the store: see [`Embedder::recipe`].
    recipe: String,
}

impl Meta {
    fn of(embedder: &dyn Embedder) -> Self {
        Self {
            format_version: VECTORS_FORMAT_VERSION,
            model: embedder.model_id().to_string(),
            dimensions: embedder.dimensions(),
            recipe: embedder.recipe(),
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

impl Library {
    fn vectors_root(&self) -> PathBuf {
        self.root().join("vectors")
    }

    fn vectors_dir(&self, model: &str) -> PathBuf {
        self.vectors_root().join(model)
    }

    /// Bring one model's vectors up to date with the full-text index: embed
    /// every passage the store does not have yet.
    ///
    /// The chunks are read from the full-text index, so it is updated first;
    /// a library whose index has to be rebuilt says so rather than embedding
    /// what is about to change.
    pub fn update_vectors(&self, embedder: &mut dyn Embedder, progress: &(dyn Fn(Progress) + Sync)) -> Result<VectorReport> {
        let started = Instant::now();
        let model = embedder.model_id().to_string();
        let chunks = self.stored_chunks()?;
        let (mut store, rebuilt) = Store::open(self.vectors_dir(&model), &Meta::of(embedder))?;

        let mut live = HashSet::new();
        let mut wanted: Vec<(Key, String)> = Vec::new();
        for chunk in &chunks {
            let text = passage(&chunk.title, &chunk.headings, &chunk.body);
            let key = key_of(&text);
            if live.insert(key) && !store.contains(&key) {
                wanted.push((key, text));
            }
        }

        // Passages of a like length go together. A batch is padded to its
        // longest passage, and the model reads the padding as it reads text:
        // mixing a one-line note with a full chunk made it read the one line
        // as five hundred tokens.
        wanted.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| a.0.cmp(&b.0)));

        let total = wanted.len();
        let done = AtomicUsize::new(0);
        progress(Progress { done: 0, total });
        let mut embed_ms = 0u128;
        for batch in wanted.chunks(BATCH) {
            let texts: Vec<&str> = batch.iter().map(|(_, text)| text.as_str()).collect();
            let keys: Vec<Key> = batch.iter().map(|(key, _)| *key).collect();
            let at = Instant::now();
            let vectors = embedder.embed(&texts, Role::Passage)?;
            embed_ms += at.elapsed().as_millis();
            if vectors.len() != texts.len() || vectors.iter().any(|v| v.len() != store.dimensions) {
                return Err(Error::Embedding(format!("{model} returned vectors of the wrong number or length")));
            }
            store.append(&keys, &vectors)?;
            let done = done.fetch_add(batch.len(), Ordering::Relaxed) + batch.len();
            progress(Progress { done, total });
        }
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

    /// Every chunk of the library with its vector from one model, held in
    /// memory for searching.
    ///
    /// Fails when a chunk has no vector from this model yet: a search over
    /// part of the library would answer as if it were the whole.
    pub fn semantic(&self, embedder: &dyn Embedder) -> Result<SemanticIndex> {
        let model = embedder.model_id();
        let dimensions = embedder.dimensions();
        let chunks = self.stored_chunks()?;
        let dir = self.vectors_dir(model);
        let meta: Option<Meta> = std::fs::read(dir.join("meta.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let usable = meta.is_some_and(|meta| meta.mismatch(&Meta::of(embedder)).is_none());
        let (keys, data) = if usable {
            read_records(&dir.join("vectors.bin"), dimensions)?
        } else {
            (Vec::new(), Vec::new())
        };
        let index: HashMap<Key, usize> = keys.iter().enumerate().map(|(i, key)| (*key, i)).collect();

        let mut documents: Vec<SemanticDoc> = Vec::new();
        let mut document_of: BTreeMap<String, u32> = BTreeMap::new();
        let mut entries = Vec::with_capacity(chunks.len());
        let mut matrix = Vec::with_capacity(chunks.len() * dimensions);
        let mut missing = 0usize;
        for chunk in chunks {
            let key = key_of(&passage(&chunk.title, &chunk.headings, &chunk.body));
            let Some(&at) = index.get(&key) else {
                missing += 1;
                continue;
            };
            let next = u32::try_from(documents.len()).unwrap_or(u32::MAX);
            let doc = *document_of.entry(chunk.path.clone()).or_insert_with(|| {
                documents.push(SemanticDoc {
                    path: PathBuf::from(&chunk.path),
                    title: chunk.title.clone(),
                });
                next
            });
            entries.push((doc, chunk.line, chunk.headings));
            matrix.extend_from_slice(&data[at * dimensions..(at + 1) * dimensions]);
        }
        if missing > 0 {
            return Err(Error::VectorsBehind {
                model: model.to_string(),
                missing,
            });
        }
        Ok(SemanticIndex {
            model: model.to_string(),
            dimensions,
            documents,
            chunks: entries,
            matrix,
        })
    }

    /// Delete one model's vectors. Returns whether there were any.
    pub fn remove_vectors(&self, model: &str) -> Result<bool> {
        let dir = self.vectors_dir(model);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(Error::io(&dir, e)),
        }
    }
}

#[derive(Debug, Clone)]
struct SemanticDoc {
    path: PathBuf,
    title: String,
}

/// One document found by meaning.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticHit {
    /// The file.
    pub path: PathBuf,
    /// The document's title.
    pub title: String,
    /// The headings above the closest chunk, outermost first.
    pub headings: Vec<String>,
    /// The line the closest chunk starts on.
    pub line: u32,
    /// The cosine between the query and the closest chunk: 1 is the same
    /// direction, 0 unrelated.
    pub score: f32,
}

/// A library's chunks as vectors from one model, in memory.
#[derive(Debug, Clone)]
pub struct SemanticIndex {
    model: String,
    dimensions: usize,
    documents: Vec<SemanticDoc>,
    /// Per chunk: its document, its line and its headings.
    chunks: Vec<(u32, u32, Vec<String>)>,
    matrix: Vec<f32>,
}

impl SemanticIndex {
    /// The model the vectors came from.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Chunks held.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether it holds no chunks.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The documents closest to a query vector, one hit per document at its
    /// closest chunk.
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<SemanticHit> {
        if limit == 0 || query.len() != self.dimensions {
            return Vec::new();
        }
        let mut best: HashMap<u32, (f32, usize)> = HashMap::new();
        for (i, row) in self.matrix.chunks_exact(self.dimensions).enumerate() {
            let score: f32 = row.iter().zip(query).map(|(a, b)| a * b).sum();
            let doc = self.chunks[i].0;
            let held = best.entry(doc).or_insert((f32::NEG_INFINITY, i));
            if score > held.0 {
                *held = (score, i);
            }
        }
        let mut ranked: Vec<(u32, f32, usize)> = best.into_iter().map(|(doc, (score, chunk))| (doc, score, chunk)).collect();
        // Ties go to the path, so two runs over the same library agree.
        ranked.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| self.documents[a.0 as usize].path.cmp(&self.documents[b.0 as usize].path))
        });
        ranked
            .into_iter()
            .take(limit)
            .map(|(doc, score, chunk)| {
                let document = &self.documents[doc as usize];
                let (_, line, headings) = &self.chunks[chunk];
                SemanticHit {
                    path: document.path.clone(),
                    title: document.title.clone(),
                    headings: headings.clone(),
                    line: *line,
                    score,
                }
            })
            .collect()
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
}
