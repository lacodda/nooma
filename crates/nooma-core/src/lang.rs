//! Which languages are understood, and how each one is read.
//!
//! A language is one row of a table: its name, the extensions it answers to,
//! its grammar, and the queries that pull symbols and imports out of a parse
//! tree. Adding the fifth language means adding a row and a query file, not
//! editing five `match` arms scattered through the crate.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A language `nooma-core` can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Language {
    /// Rust (`.rs`).
    Rust,
    /// TypeScript and TSX (`.ts`, `.tsx`, and JavaScript, which the grammar
    /// accepts as a subset).
    TypeScript,
    /// Python (`.py`, `.pyi`).
    Python,
    /// Go (`.go`).
    Go,
}

/// Every language this build understands, in a stable order.
pub const ALL: &[Language] = &[Language::Rust, Language::TypeScript, Language::Python, Language::Go];

impl Language {
    /// The language a file extension names, if any.
    ///
    /// Extension only: a repository's files are named by their authors, and
    /// sniffing content to second-guess `.rs` would cost a read of every file
    /// in the tree to change the answer approximately never. Documents get
    /// content sniffing in the document block, where the payoff is real.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Option<Self> {
        let ext = path.as_ref().extension()?.to_str()?;
        Some(match ext {
            "rs" => Self::Rust,
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Self::TypeScript,
            "py" | "pyi" => Self::Python,
            "go" => Self::Go,
            _ => return None,
        })
    }

    /// The name this language goes by in `--json` output and on disk.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
        }
    }

    /// The tree-sitter grammar for this language.
    pub(crate) fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// The tree-sitter query that names this language's symbols.
    pub(crate) const fn symbol_query(self) -> &'static str {
        match self {
            Self::Rust => include_str!("queries/rust/symbols.scm"),
            Self::TypeScript => include_str!("queries/typescript/symbols.scm"),
            Self::Python => include_str!("queries/python/symbols.scm"),
            Self::Go => include_str!("queries/go/symbols.scm"),
        }
    }

    /// The tree-sitter query that names this language's imports.
    pub(crate) const fn import_query(self) -> &'static str {
        match self {
            Self::Rust => include_str!("queries/rust/imports.scm"),
            Self::TypeScript => include_str!("queries/typescript/imports.scm"),
            Self::Python => include_str!("queries/python/imports.scm"),
            Self::Go => include_str!("queries/go/imports.scm"),
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_name_their_language() {
        assert_eq!(Language::from_path("src/main.rs"), Some(Language::Rust));
        assert_eq!(Language::from_path("app/page.tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_path("tool.py"), Some(Language::Python));
        assert_eq!(Language::from_path("cmd/serve.go"), Some(Language::Go));
    }

    #[test]
    fn unknown_extensions_are_not_guessed() {
        assert_eq!(Language::from_path("README.md"), None);
        assert_eq!(Language::from_path("Cargo.toml"), None);
        assert_eq!(Language::from_path("LICENSE"), None);
    }

    /// A grammar that does not match the core crate's ABI fails at load, not
    /// at parse — and the failure looks like "no symbols found", which reads
    /// as a bad query. Load every grammar so the mismatch is loud.
    #[test]
    fn every_grammar_loads_and_every_query_compiles() {
        for &lang in ALL {
            let grammar = lang.grammar();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar).unwrap_or_else(|e| panic!("{lang}: grammar rejected: {e}"));
            tree_sitter::Query::new(&grammar, lang.symbol_query()).unwrap_or_else(|e| panic!("{lang}: symbol query: {e}"));
            tree_sitter::Query::new(&grammar, lang.import_query()).unwrap_or_else(|e| panic!("{lang}: import query: {e}"));
        }
    }
}
