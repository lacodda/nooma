//! What an index holds.
//!
//! The shape is deliberately flat: a repository index is a list of files, and
//! a file holds its symbols and its imports. Nothing here is a tree, because
//! every consumer so far wants either "all symbols named X" or "everything in
//! this file", and a tree makes both slower to answer than a scan.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::lang::Language;

/// The on-disk format this build writes.
///
/// Bumped whenever the stored shape changes in a way an older reader would
/// misread. A reader that meets a different number refuses the file and says
/// to reindex — it never tries to read it anyway. A silently misread index
/// looks like "search stopped finding things", which is the worst bug this
/// product can have.
pub const FORMAT_VERSION: u32 = 2;

/// How this build cuts a file into the units it records.
///
/// A symbol here, a text chunk later: either way, changing how a file is
/// divided changes what every stored entry means, while the file's bytes and
/// therefore its hash stay exactly the same. Without a number to compare, an
/// index built by the old rules would be kept and extended by the new ones,
/// and the result reads as search quietly getting worse - the failure this
/// product can least afford.
///
/// Bumped whenever a query, a symbol kind or the chunking rule changes what a
/// file yields. A mismatch forces a full reparse; it does not invalidate the
/// file format, which [`FORMAT_VERSION`] covers.
///
/// 2: files carry a module summary — the header, and each declaration's
/// signature and doc. The bytes of a file are unchanged by that, and so is its
/// hash, so without this number an index built before summaries existed would
/// be reused and extended, and every file in it would keep answering "no
/// summary" forever.
pub const CHUNKER_VERSION: u32 = 2;

/// What an index describes: a commit, and whether the tree matched it.
///
/// A commit alone cannot say "the tree at this commit plus three unsaved
/// edits", and an index that answered `current` for that would be telling a
/// caller its own newest work is already indexed. The pair makes the wrong
/// answer unrepresentable rather than something a caller has to remember to
/// check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// The commit that was checked out, as a full hex object id.
    pub commit: String,
    /// Whether an indexed file differed from what that commit holds.
    ///
    /// Derived from content hashes, not from `git status`: git and the
    /// filesystem can disagree - a file written after git last looked is
    /// changed by every measure that matters here and by none that git
    /// reports yet.
    pub dirty: bool,
}

impl Revision {
    /// Whether this revision can stand in for `other`.
    ///
    /// Only a clean tree at the same commit can: a dirty tree is not a state
    /// that repeats, since the next edit makes a different one bearing the
    /// same name.
    pub fn covers(&self, other: &Self) -> bool {
        self.commit == other.commit && !self.dirty && !other.dirty
    }

    /// The commit as people quote it, with a marker when the tree had edits.
    pub fn short(&self) -> String {
        let commit = &self.commit[..self.commit.len().min(8)];
        if self.dirty { format!("{commit}+dirty") } else { commit.to_string() }
    }
}

/// One symbol declared in a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// What it is called.
    pub name: String,
    /// What kind of thing it is.
    pub kind: SymbolKind,
    /// The 1-based line it is declared on, as an editor counts lines.
    pub line: u32,
    /// The enclosing symbol, if the declaration sits inside one — a method's
    /// type, a nested function's function. `None` at file scope.
    pub parent: Option<String>,
}

/// The kinds of symbol worth naming across four languages.
///
/// Deliberately coarse. A union is a type, a trait is a type, a Go interface
/// is a type: consumers ask "where is `RepoIndex` declared", never "was it a
/// struct or an enum". Splitting the kind finer would make every caller match
/// on four spellings of the same idea.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SymbolKind {
    /// A function, method, or closure bound to a name.
    Function,
    /// A struct, enum, union, trait, interface, class, or type alias.
    Type,
    /// A module, namespace, or Go package clause.
    Module,
    /// A constant or a static.
    Constant,
}

impl SymbolKind {
    /// The name this kind goes by in `--json` and on disk.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Type => "type",
            Self::Module => "module",
            Self::Constant => "constant",
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One thing a file pulls in from elsewhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// The module path as written in the source: `std::collections::BTreeMap`,
    /// `./widgets/Button`, `os.path`, `net/http`.
    ///
    /// Stored as written, not resolved to a file. Resolution needs the
    /// language's whole module system — tsconfig paths, Python's sys.path, Go
    /// modules — and guessing it wrong is worse than not doing it. Module
    /// dependencies below resolve only what can be resolved honestly.
    pub module: String,
    /// The 1-based line the import sits on.
    pub line: u32,
}

/// Everything known about one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndex {
    /// The path relative to the repository root, with `/` separators on every
    /// platform — a Windows index and a Linux index of the same commit agree.
    pub path: String,
    /// The language it was parsed as.
    pub language: Language,
    /// The BLAKE3 hash of the file's bytes.
    ///
    /// This is what makes the next version's incremental pass possible: the
    /// commit says which files could have changed, the hash says which ones
    /// actually did.
    pub content_hash: String,
    /// What it declares, in the order the parser met them.
    pub symbols: Vec<Symbol>,
    /// What it pulls in.
    pub imports: Vec<Import>,
    /// The module's surface in prose: its header, and each declaration's
    /// signature and doc.
    ///
    /// Stored beside the symbols rather than worked out on demand because it
    /// is keyed by the same content hash: a file that did not change does not
    /// get summarized again, which is the whole of the cache. `None` only for
    /// a file the parser could not read at all.
    #[serde(default)]
    pub summary: Option<crate::summary::ModuleSummary>,
}

/// The index of one repository at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoIndex {
    /// The on-disk format, checked before anything else is believed.
    pub format_version: u32,
    /// The rules the files were cut up by, checked before they are reused.
    pub chunker_version: u32,
    /// What this index describes.
    pub revision: Revision,
    /// The absolute path of the work tree this was built from.
    pub root: PathBuf,
    /// The indexed files, sorted by path so two runs over one revision produce
    /// byte-identical output.
    pub files: Vec<FileIndex>,
}

impl RepoIndex {
    /// An empty index for a repository, at the revision given.
    pub fn empty(root: PathBuf, revision: Revision) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            chunker_version: CHUNKER_VERSION,
            revision,
            root,
            files: Vec::new(),
        }
    }

    /// How many symbols the whole index holds.
    pub fn symbol_count(&self) -> usize {
        self.files.iter().map(|f| f.symbols.len()).sum()
    }

    /// Whether this index's entries can be reused by the running build.
    ///
    /// False when it was cut up by different rules. The format check lives in
    /// the store, because a format mismatch means the file cannot be read at
    /// all; this one means it was read fine and says something else.
    pub fn is_reusable(&self) -> bool {
        self.chunker_version == CHUNKER_VERSION
    }

    /// Every file, keyed by path.
    pub fn by_path(&self) -> BTreeMap<&str, &FileIndex> {
        self.files.iter().map(|f| (f.path.as_str(), f)).collect()
    }

    /// Which files import which, for the files that are in the index.
    ///
    /// Only edges that land on an indexed file are reported. An import of
    /// `std::fmt` or `react` has no file in this repository and is left out of
    /// the graph — it stays visible on [`FileIndex::imports`], where it is a
    /// fact rather than an edge.
    pub fn module_dependencies(&self) -> BTreeMap<&str, Vec<&str>> {
        let known: BTreeMap<&str, &FileIndex> = self.by_path();
        let mut graph = BTreeMap::new();
        for file in &self.files {
            let mut edges: Vec<&str> = file
                .imports
                .iter()
                .filter_map(|import| resolve(&known, &file.path, file.language, &import.module))
                .collect();
            edges.sort_unstable();
            edges.dedup();
            if !edges.is_empty() {
                graph.insert(file.path.as_str(), edges);
            }
        }
        graph
    }
}

/// Turn an import as written into an indexed path, when that can be done
/// honestly.
///
/// Relative imports resolve, because the source says exactly where to look.
/// Everything else does not: `crate::store` could be this repository or a
/// dependency of the same name, and a wrong edge in a dependency graph is
/// worse than a missing one.
fn resolve<'a>(known: &BTreeMap<&'a str, &'a FileIndex>, from: &str, language: Language, module: &str) -> Option<&'a str> {
    if !module.starts_with('.') {
        return None;
    }
    let dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for part in module.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let base = parts.join("/");
    let candidates: &[&str] = match language {
        Language::TypeScript => &[".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.tsx"],
        Language::Python => &[".py", "/__init__.py"],
        _ => &[],
    };
    // An import may already carry its extension; try it bare first.
    known.get(base.as_str()).map(|f| f.path.as_str()).or_else(|| {
        candidates
            .iter()
            .find_map(|suffix| known.get(format!("{base}{suffix}").as_str()).map(|f| f.path.as_str()))
    })
}
