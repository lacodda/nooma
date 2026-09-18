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
pub const FORMAT_VERSION: u32 = 1;

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
}

/// The index of one repository at one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoIndex {
    /// The on-disk format, checked before anything else is believed.
    pub format_version: u32,
    /// The commit this index describes, as a full hex object id.
    pub commit: String,
    /// The absolute path of the work tree this was built from.
    pub root: PathBuf,
    /// The indexed files, sorted by path so two runs over one commit produce
    /// byte-identical output.
    pub files: Vec<FileIndex>,
}

impl RepoIndex {
    /// How many symbols the whole index holds.
    pub fn symbol_count(&self) -> usize {
        self.files.iter().map(|f| f.symbols.len()).sum()
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
