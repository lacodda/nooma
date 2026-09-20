//! Where prose is kept, so that it is paid for once.
//!
//! Deliberately not part of the index. Everything the index holds is free to
//! recompute — parsing a file again costs milliseconds — which is why a
//! `CHUNKER_VERSION` bump throws the whole index away and rebuilds it. That is
//! the right thing to do with structure and exactly the wrong thing to do with
//! prose: prose costs the user money, and a version bump that silently burned
//! it would teach them not to use the feature twice.
//!
//! So prose lives in its own file, with its own format number, and a chunker
//! bump does not touch it. The key is the content hash alone — no path, no
//! repository — because the same bytes are the same module wherever they sit,
//! and a vendored copy of a crate should not be described a second time.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// The on-disk format of the prose cache.
///
/// Its own number, independent of the index's. A cache that cannot be read is
/// started empty rather than refused: the worst case is paying for a module
/// again, and refusing to run because an old cache file exists is worse.
const CACHE_FORMAT: u32 = 1;

/// The prose already paid for, on this machine.
#[derive(Debug)]
pub struct Cache {
    path: PathBuf,
    entries: RefCell<Entries>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entries {
    format: u32,
    /// Prose by content hash. A `BTreeMap` so the file comes out the same way
    /// twice and a diff of it is readable.
    prose: BTreeMap<String, String>,
}

impl Cache {
    /// Open the cache beside the store's index files.
    ///
    /// The same directory the index lives in: it describes the same machine's
    /// work, and someone clearing nooma's data should not have to find two
    /// places to clear.
    pub fn open(store_root: &Path) -> Result<Self> {
        let path = store_root.join("prose.json");
        let entries = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Entries>(&bytes) {
                // A cache written by a format this build does not know is
                // started over rather than read under today's meaning.
                Ok(entries) if entries.format == CACHE_FORMAT => entries,
                _ => Entries::new(),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Entries::new(),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self {
            path,
            entries: RefCell::new(entries),
        })
    }

    /// Where the cache file sits.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many modules have prose already.
    pub fn len(&self) -> usize {
        self.entries.borrow().prose.len()
    }

    /// Whether nothing has been described yet.
    ///
    /// Only the tests ask, but clippy requires it wherever `len` exists, and
    /// a `len` without it is the lint rather than an oversight.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The prose held for a content hash, if any.
    pub fn get(&self, content_hash: &str) -> Option<String> {
        self.entries.borrow().prose.get(content_hash).cloned()
    }

    /// Record prose for a content hash and write the cache out.
    ///
    /// Written after every entry rather than once at the end: a run over a
    /// large repository is a long series of paid calls, and a crash halfway
    /// through must not throw away the half already bought.
    pub fn put(&self, content_hash: &str, text: &str) -> Result<()> {
        self.entries.borrow_mut().prose.insert(content_hash.to_owned(), text.to_owned());
        self.flush()
    }

    fn flush(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&*self.entries.borrow()).expect("the cache is always serializable");
        // Written aside and renamed over, as the index is: a crash mid-write
        // must leave the prose already paid for intact rather than truncated.
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, &bytes).with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path).with_context(|| format!("writing {}", self.path.display()))?;
        Ok(())
    }
}

impl Entries {
    fn new() -> Self {
        Self {
            format: CACHE_FORMAT,
            prose: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        cache.put("abc", "What it is for.").unwrap();

        let reopened = Cache::open(dir.path()).unwrap();
        assert_eq!(reopened.get("abc").as_deref(), Some("What it is for."));
    }

    #[test]
    fn an_unknown_hash_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        assert_eq!(cache.get("never-seen"), None);
    }

    /// A cache from a format this build does not know is started over, not
    /// refused: prose is a cache, nothing else depends on it, and refusing to
    /// run because one exists would cost more than paying for a module again.
    ///
    /// Mutating the equality check in `open` has to turn this red.
    #[test]
    fn a_foreign_format_starts_over_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = serde_json::json!({ "format": CACHE_FORMAT + 1, "prose": { "abc": "written under other rules" } });
        std::fs::write(dir.path().join("prose.json"), serde_json::to_vec(&foreign).unwrap()).unwrap();

        let cache = Cache::open(dir.path()).unwrap();
        assert_eq!(cache.get("abc"), None, "a foreign format must not be read under today's meaning");
        assert!(cache.is_empty());
    }

    /// A truncated file is the normal result of a crash mid-write. It must
    /// cost one repeated call, not a command that refuses to start.
    #[test]
    fn a_truncated_cache_starts_over_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prose.json"), br#"{"format":1,"prose":{"abc""#).unwrap();

        let cache = Cache::open(dir.path()).unwrap();
        assert!(cache.is_empty());
        // And it is usable afterwards, rather than poisoned by what it met.
        cache.put("abc", "fresh").unwrap();
        assert_eq!(cache.get("abc").as_deref(), Some("fresh"));
    }

    /// Every entry reaches disk as it arrives. A run over a large repository
    /// is a series of paid calls, and a crash partway through must not discard
    /// the ones already bought.
    #[test]
    fn each_entry_is_on_disk_before_the_next_call() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        cache.put("first", "One.").unwrap();

        // Read through a second handle, which never saw the in-memory write.
        let witness = Cache::open(dir.path()).unwrap();
        assert_eq!(witness.get("first").as_deref(), Some("One."));
    }

    /// The whole point of keeping prose out of the index: an index rebuild is
    /// free and throws everything away, and prose is neither. The cache file
    /// must not be the index file.
    #[test]
    fn the_cache_is_a_file_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path()).unwrap();
        let store = nooma_core::Store::open_at(dir.path()).unwrap();
        assert_ne!(cache.path(), store.path_for(Path::new("/some/repo")));
    }
}
