//! Pulling symbols and imports out of source.
//!
//! Each language contributes two tree-sitter queries and nothing else: one
//! that captures declarations, one that captures imports. The code here is the
//! same for all four, so a fifth language is a row in [`crate::lang`] plus two
//! `.scm` files.
//!
//! Captures follow a convention the queries all obey:
//!
//! - `@name` — the identifier to record
//! - `@kind.function`, `@kind.type`, `@kind.module`, `@kind.constant` — what
//!   kind of symbol the match is; the capture wraps the whole declaration, so
//!   it also gives the span a nested declaration is tested against
//! - `@module` — an import's module path, as written

use rayon::prelude::*;
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator as _};

use crate::error::{Error, Result};
use crate::index::{FileIndex, Import, Symbol, SymbolKind};
use crate::lang::Language;
use crate::repo::SourceFile;

/// Parse one file's bytes into its symbols and imports.
pub fn parse(language: Language, source: &[u8]) -> Option<(Vec<Symbol>, Vec<Import>)> {
    let grammar = language.grammar();
    let mut parser = Parser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    // A query that does not compile is a bug in this crate, not in the file
    // being parsed — the test in `lang` compiles all eight at build time, so
    // reaching here with a bad query means that test was deleted.
    let symbol_query = Query::new(&grammar, language.symbol_query()).ok()?;
    let import_query = Query::new(&grammar, language.import_query()).ok()?;

    let symbols = collect_symbols(&symbol_query, root, source);
    let imports = collect_imports(&import_query, root, source);
    Some((symbols, imports))
}

/// Index one file, reading it from disk.
pub fn index_file(file: &SourceFile) -> Result<FileIndex> {
    let source = std::fs::read(&file.absolute).map_err(|e| Error::io(&file.absolute, e))?;
    let (symbols, imports) = parse(file.language, &source).ok_or_else(|| Error::Parse {
        path: file.absolute.clone(),
        language: file.language,
    })?;
    Ok(describe(
        file.relative.clone(),
        file.language,
        blake3::hash(&source).to_hex().to_string(),
        symbols,
        imports,
        &source,
    ))
}

/// Assemble one file's entry, summary and all.
///
/// The summary is built here rather than by each caller, so a file cannot be
/// indexed without one: a file whose entry says "no summary" is
/// indistinguishable from a module that genuinely has nothing to say, and the
/// difference would only surface as a gap in search results.
pub fn describe(path: String, language: Language, content_hash: String, symbols: Vec<Symbol>, imports: Vec<Import>, source: &[u8]) -> FileIndex {
    let mut file = FileIndex {
        path,
        language,
        content_hash,
        symbols,
        imports,
        summary: None,
    };
    file.summary = crate::summary::summarize(&file, source);
    file
}

/// Index many files, in parallel, keeping the input order.
///
/// Parsing is CPU-bound and every file is independent, so this is the one
/// place in the crate that earns a thread pool. Files that cannot be read or
/// parsed are dropped rather than failing the run: an index missing one
/// generated file is useful, an index that refused to build is not.
pub fn index_files(files: &[SourceFile]) -> Vec<FileIndex> {
    files.par_iter().filter_map(|file| index_file(file).ok()).collect()
}

/// One match of the symbol query, before parents are worked out.
struct Found {
    symbol: Symbol,
    start: usize,
    end: usize,
    /// Whether this belongs in the index, or only lends its span.
    ///
    /// A Rust `impl` block is the case this exists for: it is not a
    /// declaration — `Ledger` is declared once, by its struct — but a method
    /// inside it belongs to `Ledger`, and only the impl block's span says so.
    reported: bool,
}

/// Run the symbol query and turn each match into a [`Symbol`].
fn collect_symbols(query: &Query, root: Node<'_>, source: &[u8]) -> Vec<Symbol> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    // Collected with their spans first, because a symbol's parent is whichever
    // other symbol encloses it — which cannot be known until all of them are
    // in hand. Asking the syntax tree instead would mean naming, per language,
    // every node type that counts as a scope.
    let mut found: Vec<Found> = Vec::new();
    while let Some(m) = matches.next() {
        let mut name = None;
        let mut kind = None;
        let mut span = None;
        let mut reported = true;
        for capture in m.captures() {
            let capture_name = names[capture.index as usize];
            if capture_name == "name" {
                name = capture.node.utf8_text(source).ok();
            } else if capture_name == "scope" {
                // A scope has no kind of its own; one is recorded only so the
                // entry is well formed, and it is dropped before output.
                kind = Some(SymbolKind::Module);
                reported = false;
                span = Some((capture.node.start_byte(), capture.node.end_byte()));
            } else if let Some(k) = kind_of(capture_name) {
                kind = Some(k);
                span = Some((capture.node.start_byte(), capture.node.end_byte()));
            }
        }
        let (Some(name), Some(kind), Some((start, end))) = (name, kind, span) else {
            continue;
        };
        let line = line_of(source, start);
        let symbol = Symbol {
            name: name.to_string(),
            kind,
            line,
            parent: None,
        };
        found.push(Found { symbol, start, end, reported });
    }
    found.sort_by_key(|f| (f.start, std::cmp::Reverse(f.end)));
    attach_parents(fold_duplicates(found))
}

/// Keep one symbol per declaration, preferring the most specific kind.
///
/// A query cannot say "a binding, unless its value is a function", so
/// `const Panel = () => {}` matches both the arrow-function pattern and the
/// plain-binding one. Both are right about the same declaration; the narrower
/// answer is the useful one. Anything else at the same span with the same name
/// is the same fact said twice.
fn fold_duplicates(found: Vec<Found>) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::with_capacity(found.len());
    for entry in found {
        let same = out
            .iter_mut()
            .find(|held| held.start == entry.start && held.end == entry.end && held.symbol.name == entry.symbol.name);
        match same {
            Some(held) => {
                // A real declaration always wins over a scope at the same
                // span: the scope was only ever there to lend its span.
                if !held.reported && entry.reported {
                    held.symbol.kind = entry.symbol.kind;
                    held.reported = true;
                } else if held.reported == entry.reported && specificity(entry.symbol.kind) > specificity(held.symbol.kind) {
                    held.symbol.kind = entry.symbol.kind;
                }
            }
            None => out.push(entry),
        }
    }
    out
}

/// How much a kind claims about a declaration.
///
/// Only the ordering matters, and only between kinds two patterns can both
/// claim: a binding is the fallback, so anything else outranks it.
const fn specificity(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Constant => 0,
        SymbolKind::Function | SymbolKind::Type | SymbolKind::Module => 1,
    }
}

/// Give each symbol the innermost other symbol that encloses it.
fn attach_parents(found: Vec<Found>) -> Vec<Symbol> {
    let mut out: Vec<Symbol> = Vec::with_capacity(found.len());
    // The stack holds the declarations still open at this point in the file.
    // Because matches are sorted by start, and by widest-first on a tie, the
    // top of the stack after popping is always the innermost enclosing one.
    let mut open: Vec<(String, usize)> = Vec::new();
    for Found {
        mut symbol,
        start,
        end,
        reported,
    } in found
    {
        while open.last().is_some_and(|&(_, open_end)| open_end <= start) {
            open.pop();
        }
        symbol.parent = open.last().map(|(name, _)| name.clone());
        open.push((symbol.name.clone(), end));
        // A scope has done its work by being on the stack.
        if reported {
            out.push(symbol);
        }
    }
    out
}

/// Run the import query and turn each match into an [`Import`].
fn collect_imports(query: &Query, root: Node<'_>, source: &[u8]) -> Vec<Import> {
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    let mut imports = Vec::new();
    while let Some(m) = matches.next() {
        for capture in m.captures() {
            if names[capture.index as usize] != "module" {
                continue;
            }
            let Ok(text) = capture.node.utf8_text(source) else {
                continue;
            };
            let module = text.trim_matches(['"', '\'', '`']).to_string();
            if module.is_empty() {
                continue;
            }
            imports.push(Import {
                module,
                line: line_of(source, capture.node.start_byte()),
            });
        }
    }
    imports.sort_by_key(|i| i.line);
    imports.dedup_by(|a, b| a.module == b.module && a.line == b.line);
    imports
}

/// The capture name for a kind, or `None` if the capture means something else.
fn kind_of(capture: &str) -> Option<SymbolKind> {
    match capture {
        "kind.function" => Some(SymbolKind::Function),
        "kind.type" => Some(SymbolKind::Type),
        "kind.module" => Some(SymbolKind::Module),
        "kind.constant" => Some(SymbolKind::Constant),
        _ => None,
    }
}

/// The 1-based line a byte offset falls on.
///
/// tree-sitter counts rows from zero; editors, compilers and everyone reading
/// the output count from one. Converted here, once, rather than at each of the
/// places that shows a line to a person.
fn line_of(source: &[u8], offset: usize) -> u32 {
    let newlines = source[..offset.min(source.len())].iter().filter(|&&b| b == b'\n').count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_one_based() {
        let source = b"one\ntwo\nthree";
        assert_eq!(line_of(source, 0), 1);
        assert_eq!(line_of(source, 4), 2);
        assert_eq!(line_of(source, 8), 3);
    }

    #[test]
    fn an_offset_past_the_end_does_not_panic() {
        assert_eq!(line_of(b"one\ntwo", 999), 2);
    }
}
