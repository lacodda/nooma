//! The repository: which commit is checked out, and which files count.
//!
//! An index is pinned to a commit hash, so two callers asking about the same
//! commit get the same answer and a caller can tell whether the answer it
//! holds is still current. The files themselves are read from the work tree
//! rather than from the commit's tree: that is what the developer is looking
//! at, and the next version's incremental pass has to handle a dirty tree
//! anyway.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::lang::Language;

/// A git work tree nooma can index.
#[derive(Debug)]
pub struct Repo {
    root: PathBuf,
    commit: String,
}

/// One file worth parsing, found by the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Absolute path on this machine.
    pub absolute: PathBuf,
    /// Path relative to the repository root, `/`-separated on every platform.
    pub relative: String,
    /// The language its extension names.
    pub language: Language,
}

impl Repo {
    /// Open the repository that contains `path`.
    ///
    /// Any path inside the work tree will do — a subdirectory, or a file. This
    /// is how every git command behaves, and a caller that has a file path
    /// should not have to walk up to find the root itself.
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        // Discovery walks up from a directory. A caller holding a file path —
        // which is the usual way an editor or another tool asks — would get a
        // bare refusal, so start from the file's directory instead.
        let start = if path.is_file() { path.parent().unwrap_or(path) } else { path };
        let repo = gix::discover(start).map_err(|_| Error::NotARepository(path.to_path_buf()))?;
        let root = repo.workdir().ok_or_else(|| Error::NotARepository(path.to_path_buf()))?.to_path_buf();
        let root = normalize(&root).map_err(|e| Error::io(&root, e))?;
        // A repository with no commits is a normal state — `git init` and
        // nothing else — and it is not an error the caller caused. Name it, so
        // "no symbols found" never stands in for "there is nothing here yet".
        let commit = repo.head_commit().map_err(|_| Error::EmptyRepository(root.clone()))?.id().to_hex().to_string();
        Ok(Self { root, commit })
    }

    /// The absolute path of the work tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The commit the index is pinned to, as full hex.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Every file in the work tree that nooma knows how to parse.
    ///
    /// `.gitignore` is honored because git honors it: `target/` and
    /// `node_modules/` are build output, and indexing them would bury the
    /// repository's own symbols under its dependencies'. `.nooma-ignore` sits
    /// beside it with the same syntax, for what belongs in git but not in the
    /// index — vendored code, generated bindings, fixtures.
    pub fn source_files(&self) -> Result<Vec<SourceFile>> {
        let mut walker = ignore::WalkBuilder::new(&self.root);
        walker
            .standard_filters(true)
            .hidden(true)
            .parents(false)
            .add_custom_ignore_filename(".nooma-ignore");
        let mut files = Vec::new();
        for entry in walker.build() {
            let entry = match entry {
                Ok(entry) => entry,
                // One unreadable directory must not lose the other thousand
                // files. Permission errors on a work tree are ordinary on
                // Windows, where an editor can hold a lock.
                Err(_) => continue,
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Some(language) = Language::from_path(entry.path()) else {
                continue;
            };
            let Some(relative) = relative_to(&self.root, entry.path()) else {
                continue;
            };
            files.push(SourceFile {
                absolute: entry.path().to_path_buf(),
                relative,
                language,
            });
        }
        // Sorted so that an index of one commit is the same bytes every run —
        // which is what lets a caller compare two indexes at all.
        files.sort_by(|a, b| a.relative.cmp(&b.relative));
        Ok(files)
    }
}

/// The canonical form of a path, without Windows' extended-length prefix.
///
/// `canonicalize` on Windows returns `\\?\C:\...`. It is a real path the OS
/// accepts, and nothing else on the machine produces it: it reaches `--json`,
/// the key the index is stored under and every message, and it never equals
/// the `C:\...` a caller passed. The same repository then looks like two, and
/// a person reading the output gets a path they cannot paste anywhere.
fn normalize(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = path.canonicalize()?;
    let text = canonical.to_string_lossy();
    // A UNC path is left alone: stripping the prefix off `\\?\UNC\server\share`
    // leaves `UNC\server\share`, which is not a path at all.
    match text.strip_prefix(r"\\?\") {
        Some(stripped) if !stripped.starts_with("UNC\\") => Ok(PathBuf::from(stripped)),
        _ => Ok(canonical),
    }
}

/// `path` relative to `root`, with forward slashes.
///
/// The separator is normalized rather than left to the platform: an index
/// built on Windows and one built in CI on Linux describe the same commit, and
/// they have to agree on what a path is called or every comparison between
/// them is noise.
fn relative_to(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut out = String::with_capacity(relative.as_os_str().len());
    for component in relative.components() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(component.as_os_str().to_str()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_are_normalized() {
        let root = Path::new("/tmp/repo");
        let nested = root.join("src").join("lang.rs");
        assert_eq!(relative_to(root, &nested).as_deref(), Some("src/lang.rs"));
    }

    #[test]
    fn a_path_outside_the_root_has_no_relative_form() {
        assert_eq!(relative_to(Path::new("/tmp/repo"), Path::new("/etc/passwd")), None);
    }
}
