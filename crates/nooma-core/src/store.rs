//! Where the index lives between runs.
//!
//! The index is on disk from the first version, not because v0.1 is slow
//! enough to need a cache, but because persistence changes the shape of
//! everything above it: the incremental pass, the model version that forces a
//! reindex, the contract `rigger` reads. Bolting it on later would mean
//! rewriting all three.
//!
//! One repository, one file, named by the work tree's path. The commit hash is
//! stored inside rather than in the name: a caller asks "is your index current
//! for this commit", and a directory of a hundred stale files per repository
//! would answer that question by accumulating garbage.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::index::{FORMAT_VERSION, RepoIndex};

/// The directory indexes are kept in.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store in this machine's usual place for application data.
    pub fn open() -> Result<Self> {
        Self::open_at(crate::data_dir()?.join("index"))
    }

    /// The store in a given directory, created if it is not there.
    ///
    /// Tests use this; so does anyone who wants the index beside the thing it
    /// describes rather than in their profile.
    pub fn open_at(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;
        Ok(Self { root })
    }

    /// Where this store keeps its files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file holding the index of the repository at `repo_root`.
    ///
    /// Named by a hash of the path, not by the path itself: work trees live at
    /// `C:\Projects\nooma` and `/home/k/nooma`, and turning either into a
    /// filename means inventing an escaping scheme that two platforms would
    /// have to agree on. The readable name goes inside the file, where
    /// `RepoIndex::root` already holds it.
    pub fn path_for(&self, repo_root: &Path) -> PathBuf {
        let key = blake3::hash(repo_root.to_string_lossy().as_bytes()).to_hex();
        self.root.join(format!("{}.json", &key[..32]))
    }

    /// Read the stored index for a repository, if there is one.
    ///
    /// A missing file is `Ok(None)` — not having indexed yet is a normal
    /// state. A file written by a different format is an error, and one that
    /// [`Error::is_stale_index`] tells the caller to fix by reindexing.
    pub fn load(&self, repo_root: &Path) -> Result<Option<RepoIndex>> {
        let path = self.path_for(repo_root);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(&path, e)),
        };
        // The format is read before the rest of the document, so that a field
        // that changed meaning between versions is never deserialized under
        // the new meaning and quietly believed.
        let probe: FormatProbe = serde_json::from_slice(&bytes).map_err(|source| Error::IndexUnreadable { path: path.clone(), source })?;
        if probe.format_version != FORMAT_VERSION {
            return Err(Error::IndexFormat {
                found: probe.format_version,
                expected: FORMAT_VERSION,
            });
        }
        let index: RepoIndex = serde_json::from_slice(&bytes).map_err(|source| Error::IndexUnreadable { path, source })?;
        Ok(Some(index))
    }

    /// Write an index, replacing whatever was there.
    ///
    /// Written to a neighbouring file and renamed over the target: a crash
    /// halfway through a write leaves the previous index intact instead of a
    /// truncated one, and a truncated index is exactly the failure that reads
    /// as "search stopped finding things".
    pub fn save(&self, index: &RepoIndex) -> Result<PathBuf> {
        let path = self.path_for(&index.root);
        let bytes = serde_json::to_vec(index).expect("an index is always serializable");
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, &bytes).map_err(|e| Error::io(&temporary, e))?;
        std::fs::rename(&temporary, &path).map_err(|e| Error::io(&path, e))?;
        Ok(path)
    }
}

/// Just enough of the document to check the format before trusting the rest.
#[derive(serde::Deserialize)]
struct FormatProbe {
    format_version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(root: &Path) -> RepoIndex {
        let revision = crate::index::Revision {
            commit: "0".repeat(40),
            dirty: false,
        };
        RepoIndex::empty(root.to_path_buf(), revision)
    }

    #[test]
    fn an_index_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        let index = sample(Path::new("/some/repo"));
        store.save(&index).unwrap();
        assert_eq!(store.load(Path::new("/some/repo")).unwrap(), Some(index));
    }

    #[test]
    fn never_indexed_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        assert_eq!(store.load(Path::new("/never/seen")).unwrap(), None);
    }

    #[test]
    fn two_repositories_do_not_share_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        assert_ne!(store.path_for(Path::new("/a")), store.path_for(Path::new("/b")));
    }

    /// The point of the format field: an older index must be refused, not
    /// read under today's meaning. Mutating the number has to turn this red.
    #[test]
    fn a_foreign_format_is_refused_and_says_to_reindex() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        let repo = Path::new("/some/repo");
        let mut stale = sample(repo);
        stale.format_version = FORMAT_VERSION + 1;
        std::fs::write(store.path_for(repo), serde_json::to_vec(&stale).unwrap()).unwrap();

        let error = store.load(repo).unwrap_err();
        assert!(
            matches!(error, Error::IndexFormat { found, expected }
                if found == FORMAT_VERSION + 1 && expected == FORMAT_VERSION),
            "expected a format complaint, got: {error}"
        );
        assert!(error.is_stale_index());
    }

    #[test]
    fn a_truncated_index_is_refused_and_says_to_reindex() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_at(dir.path()).unwrap();
        let repo = Path::new("/some/repo");
        std::fs::write(store.path_for(repo), br#"{"format_version":1,"commit":"#).unwrap();

        let error = store.load(repo).unwrap_err();
        assert!(error.is_stale_index(), "expected a stale-index error, got: {error}");
    }
}
