//! The repository: which commit is checked out, and which files count.
//!
//! An index is pinned to a commit hash, so two callers asking about the same
//! commit get the same answer and a caller can tell whether the answer it
//! holds is still current. The files themselves are read from the work tree
//! rather than from the commit's tree: that is what the developer is looking
//! at, and the next version's incremental pass has to handle a dirty tree
//! anyway.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::lang::Language;

/// A git work tree nooma can index.
pub struct Repo {
    root: PathBuf,
    commit: String,
    /// Kept open so the commit's own bytes can be read back later, which is
    /// how a working tree is told from the commit it sits on.
    inner: gix::Repository,
    /// The ignore rules, built once.
    ///
    /// Shared by the working-tree walk and the commit walk on purpose: what
    /// counts as an indexable file is one rule, and two copies of it would
    /// eventually disagree about a `target/` that someone checked in, which
    /// reads as the tree being permanently dirty.
    ignore: ignore::gitignore::Gitignore,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `gix::Repository` prints its whole configuration, which would bury
        // the two fields anyone debugging this actually wants.
        f.debug_struct("Repo")
            .field("root", &self.root)
            .field("commit", &self.commit)
            .finish_non_exhaustive()
    }
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
        let ignore = build_ignore(&root);
        Ok(Self {
            root,
            commit,
            inner: repo,
            ignore,
        })
    }

    /// The absolute path of the work tree root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The commit the index is pinned to, as full hex.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// The hash of every indexable file as the checked-out commit holds it.
    ///
    /// Hashed with the same function as the working tree's files, over the
    /// same bytes, so the two are directly comparable. Git's own object ids
    /// are not: they are SHA-1 over a header plus the content, and keeping a
    /// second hash per file to compare against them would buy nothing.
    ///
    /// The whole list is returned rather than answers about the paths a caller
    /// names, because the thing a caller most needs to notice is a file the
    /// commit has and the tree has lost - and a question asked about what the
    /// tree holds can never mention one.
    ///
    /// The ignore rules are the same object the working-tree walk uses, so
    /// the two lists cannot disagree about what counts. A commit may hold a
    /// `target/` that someone checked in once; the walk skips it, and so does
    /// this.
    pub fn committed_source_hashes(&self) -> Result<BTreeMap<String, String>> {
        let commit = self.inner.head_commit().map_err(|_| Error::EmptyRepository(self.root.clone()))?;
        let tree = commit.tree().map_err(Error::git)?;

        let mut hashes = BTreeMap::new();
        let mut stack = vec![(String::new(), tree)];
        while let Some((prefix, tree)) = stack.pop() {
            for entry in tree.iter() {
                let entry = entry.map_err(Error::git)?;
                // A name that is not UTF-8 has no relative path on the
                // working-tree side either, so it cannot be compared with one.
                let Ok(name) = std::str::from_utf8(entry.filename()) else { continue };
                let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
                let mode = entry.mode();

                if mode.is_tree() {
                    if self.ignores(&path, true) {
                        continue;
                    }
                    let object = entry.object().map_err(Error::git)?;
                    stack.push((path, object.into_tree()));
                } else if mode.is_blob() && Language::from_path(&path).is_some() && !self.ignores(&path, false) {
                    let object = entry.object().map_err(Error::git)?;
                    let blob = object.into_blob();
                    hashes.insert(path, blake3::hash(&blob.data).to_hex().to_string());
                }
            }
        }
        Ok(hashes)
    }

    /// Whether the ignore rules keep this path out of the index.
    ///
    /// Asked of a path out of a commit, which need not exist on disk at all,
    /// so the question is about the rules and not about the file.
    fn ignores(&self, relative: &str, is_dir: bool) -> bool {
        self.ignore.matched_path_or_any_parents(self.root.join(relative), is_dir).is_ignore()
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

/// The ignore rules that decide what counts as an indexable file.
///
/// `.gitignore` is honored because git honors it: `target/` and
/// `node_modules/` are build output, and indexing them would bury a
/// repository's own symbols under its dependencies'. `.nooma-ignore` sits
/// beside it with the same syntax, for what belongs in git but not in the
/// index - vendored code, generated bindings, fixtures.
///
/// Only the rules at the repository root are collected. Nested `.gitignore`
/// files deeper in the tree are honored by the working-tree walk, which reads
/// them as it descends; the commit walk cannot see them without reading each
/// one out of the commit, and a rule that applies to one walk and not the
/// other is worse than a rule that applies to neither. Root rules are where
/// `target/` and `node_modules/` are declared in every repository of this
/// line, so the two walks agree about everything that matters in practice.
fn build_ignore(root: &Path) -> ignore::gitignore::Gitignore {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for name in [".gitignore", ".nooma-ignore"] {
        // A missing file is the usual case, and `add` reports it the same way
        // as a malformed one. Neither is worth failing discovery over.
        builder.add(root.join(name));
    }
    builder.build().unwrap_or_else(|_| ignore::gitignore::Gitignore::empty())
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
