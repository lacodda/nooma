//! Turning a parsed file into prose a reader — or an embedder — can use.
//!
//! An index entry answers "where is `RepoIndex` declared". A summary answers
//! the question the product is actually for: *what is this module about?* The
//! difference matters for what comes next — a vector index needs text with
//! meaning in it, and a list of bare identifiers has almost none. A signature
//! and the sentence above it have a great deal.
//!
//! Nothing here is generated or inferred. Every line of a summary is text the
//! author wrote, lifted out of the file and put in order: the module's own
//! header comment, then each declaration worth naming with its signature and
//! its doc. A summary that invented a description would be one nobody could
//! trust, and this product's whole promise is that the machine did not add
//! anything the author did not write.
//!
//! # What is left out
//!
//! Bodies. A summary is the surface of a module, and the body of a function is
//! the one part of it every caller is entitled to ignore. Leaving bodies out
//! is also what keeps a summary small enough to embed whole.

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::index::{FileIndex, Symbol, SymbolKind};
use crate::lang::Language;

/// The summary of one module.
///
/// "Module" means one file: the four languages disagree about what a module is
/// — a Rust file is part of one, a Go file is part of a package, a Python file
/// is one — and a file is the unit all four agree exists and the only one a
/// content hash can key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSummary {
    /// The path relative to the repository root.
    pub path: String,
    /// The language it was read as.
    pub language: Language,
    /// The file's own header comment, if it has one.
    ///
    /// `//!` in Rust, a leading `"""` in Python, the comment above `package`
    /// in Go, a leading block comment in TypeScript. This is the one sentence
    /// most likely to say what the module is for, which is why it leads.
    pub header: Option<String>,
    /// The declarations worth naming, in the order they appear.
    pub entries: Vec<SummaryEntry>,
}

/// One declaration, as a summary records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryEntry {
    /// What it is called.
    pub name: String,
    /// What kind of thing it is.
    pub kind: SymbolKind,
    /// The 1-based line it is declared on.
    pub line: u32,
    /// The enclosing declaration, if any.
    pub parent: Option<String>,
    /// The declaration as written, up to but not including its body.
    ///
    /// `pub fn add(a: i32, b: i32) -> i32`, `def add(a: int) -> int:`,
    /// `export function add(a: number): number`. Whitespace is collapsed so a
    /// signature broken across six lines reads as one.
    pub signature: String,
    /// The documentation attached to it, with comment markers stripped.
    pub doc: Option<String>,
    /// Whether the language marks this declaration as visible outside.
    ///
    /// Each language says so differently and all four say it somehow: `pub`,
    /// `export`, an initial capital, a name that does not start with `_`.
    pub public: bool,
}

impl ModuleSummary {
    /// The summary as one block of text, for a reader or an embedder.
    ///
    /// The shape is deliberately plain: this is what a vector index will be
    /// handed, and markup would be tokens spent on punctuation. Private
    /// declarations are left out of the text though they are kept in
    /// `entries` — someone asking what a module offers is asking about its
    /// surface, and its internals would dilute the meaning of the whole.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.path);
        if let Some(header) = &self.header {
            out.push_str("\n\n");
            out.push_str(header);
        }
        for entry in self.entries.iter().filter(|e| e.public) {
            out.push_str("\n\n");
            out.push_str(&entry.signature);
            if let Some(doc) = &entry.doc {
                out.push('\n');
                out.push_str(doc);
            }
        }
        out
    }

    /// How many declarations the summary reports as visible outside.
    pub fn public_count(&self) -> usize {
        self.entries.iter().filter(|e| e.public).count()
    }
}

/// Build the summary of a file that has already been parsed into symbols.
///
/// The symbols are passed in rather than found again: they carry the parents
/// the symbol pass worked out, and asking the tree a second time would give
/// two places where "what encloses what" is decided.
pub fn summarize(file: &FileIndex, source: &[u8]) -> Option<ModuleSummary> {
    let grammar = file.language.grammar();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();

    let header = header_doc(file.language, root, source);
    let entries = file.symbols.iter().map(|symbol| entry_for(file.language, root, source, symbol)).collect();

    Some(ModuleSummary {
        path: file.path.clone(),
        language: file.language,
        header,
        entries,
    })
}

/// Build one entry, finding the declaration node the symbol names.
fn entry_for(language: Language, root: Node<'_>, source: &[u8], symbol: &Symbol) -> SummaryEntry {
    let node = declaration_at(root, symbol.line, &symbol.name, source);
    let signature = node.map(|node| signature_of(node, source)).unwrap_or_else(|| symbol.name.clone());
    let doc = node.and_then(|node| doc_for(language, node, source));
    let public = match node {
        Some(node) => is_public(language, node, source),
        // Without a node there is nothing to read a modifier off, so the
        // languages that encode visibility in the name can still answer and
        // the ones that do not fall back to private.
        None => implied_public(language, &symbol.name),
    };
    SummaryEntry {
        name: symbol.name.clone(),
        kind: symbol.kind,
        line: symbol.line,
        parent: symbol.parent.clone(),
        signature,
        doc,
        public,
    }
}

/// The declaration node a symbol was recorded from.
///
/// Found by position and name rather than by running the symbol query again:
/// the query already ran, and asking it twice invites the two answers to
/// differ about what was declared where.
fn declaration_at<'a>(root: Node<'a>, line: u32, name: &str, source: &[u8]) -> Option<Node<'a>> {
    let row = line.checked_sub(1)? as usize;
    let mut best: Option<Node<'a>> = None;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        // Nothing inside a node that ends before this row, or begins after
        // it, can be the declaration.
        if node.start_position().row > row || node.end_position().row < row {
            continue;
        }
        if node.start_position().row == row && has_body(node) && name_text(node, source) == name {
            // The innermost match wins: `export function f()` and the
            // `function f()` inside it both start here, and the signature
            // belongs to the declaration rather than to the wrapper.
            let narrower = best.is_none_or(|held| node.start_byte() >= held.start_byte() && node.end_byte() <= held.end_byte());
            if narrower {
                best = Some(node);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    best
}

/// The fields a declaration keeps the part after its signature in.
///
/// One of these is what a declaration in these four languages has and a bare
/// expression does not, so their presence is the test — rather than a list of
/// node kinds needing a row per language. `type` earns its place from Go,
/// where `type Ledger struct {...}` hangs the struct off a `type` field and
/// has no `body` at all; without it every Go type loses its signature and its
/// doc while the functions beside it keep theirs.
const BODY_FIELDS: [&str; 3] = ["body", "value", "type"];

/// Whether a node is the kind of thing that has a signature and a body.
fn has_body(node: Node<'_>) -> bool {
    body_of(node).is_some()
}

/// The node holding whatever follows the signature.
fn body_of(node: Node<'_>) -> Option<Node<'_>> {
    BODY_FIELDS.iter().find_map(|field| node.child_by_field_name(field))
}

/// The declaration as written, up to its body.
///
/// It starts at the outermost wrapper, not at the named node: Go puts the
/// name on a `type_spec` that begins *after* the `type` keyword, so reading
/// from the named node gives `Ledger` where the declaration says
/// `type Ledger struct`. The same climb keeps TypeScript's `export`, which is
/// half of what an exported signature means.
///
/// Whitespace is collapsed because a signature broken across six lines is one
/// thing to read and one thing to embed.
fn signature_of(node: Node<'_>, source: &[u8]) -> String {
    let node = outermost_wrapper(node);
    let end = signature_end(node).unwrap_or_else(|| node.end_byte());
    let text = std::str::from_utf8(&source[node.start_byte()..end.max(node.start_byte())]).unwrap_or_default();
    collapse(text.trim().trim_end_matches(['{', '=']).trim())
}

/// The byte the signature stops at: where the declaration's contents begin.
///
/// Followed one level down when the field leads to another declaration rather
/// than to its contents. Go's `type Ledger struct {...}` hangs a `struct_type`
/// off its `type` field, and that node begins at the word `struct` — which is
/// part of the signature, not of the contents. Its own field list is where the
/// contents actually start.
fn signature_end(node: Node<'_>) -> Option<usize> {
    let body = body_of(node).or_else(|| node.named_child(0).and_then(body_of))?;
    // A body that declares nothing of its own ends the signature where it
    // starts; one that wraps a further definition yields to it.
    Some(match body.named_child(0) {
        Some(inner) if is_container(body) => inner.start_byte(),
        _ => body.start_byte(),
    })
}

/// Whether a node is a definition whose own opening word belongs to the
/// signature — a Go `struct`, `interface` or `map` type.
fn is_container(node: Node<'_>) -> bool {
    matches!(node.kind(), "struct_type" | "interface_type")
}

/// Collapse every run of whitespace to one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The documentation attached to a declaration.
///
/// Python keeps it inside the body as the first statement; the other three
/// keep it in comments above. Both are looked for, because asking the language
/// which convention it follows would be a table a fifth language has to be
/// added to, while looking for both costs nothing when only one is there.
fn doc_for(language: Language, node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(docstring) = leading_docstring(node, source) {
        return Some(docstring);
    }
    preceding_comments(language, node, source)
}

/// A docstring held as the first statement of a body — Python's convention.
fn leading_docstring(node: Node<'_>, source: &[u8]) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let first = body.named_child(0)?;
    let string = if first.kind() == "string" { first } else { first.named_child(0)? };
    if string.kind() != "string" {
        return None;
    }
    let cleaned = dedent(&unquote(string.utf8_text(source).ok()?));
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Strip a Python string literal's prefix and quotes.
///
/// The triple quote is removed before the single one, or `"""Adds."""` loses
/// one quote at each end and keeps two.
fn unquote(text: &str) -> String {
    let text = text.trim();
    let body = text.trim_start_matches(['r', 'b', 'u', 'f', 'R', 'B', 'U', 'F']);
    for quote in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(inner) = body.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)) {
            return inner.to_string();
        }
    }
    body.to_string()
}

/// The comments written immediately above a declaration.
///
/// The walk starts from the outermost wrapper holding this declaration alone:
/// `export function f() {}` parses as an export statement around the
/// declaration, and the doc comment sits above the export. Walking the
/// declaration's own siblings finds nothing there — which would leave exactly
/// the public API of every TypeScript module undocumented, silently.
fn preceding_comments(language: Language, node: Node<'_>, source: &[u8]) -> Option<String> {
    let anchor = outermost_wrapper(node);
    let mut lines: Vec<String> = Vec::new();
    let mut below = anchor;
    let mut previous = anchor.prev_sibling();
    while let Some(sibling) = previous {
        if !is_comment(sibling) || !is_doc_comment(language, sibling) {
            break;
        }
        // A blank line between a comment and what follows it means the
        // comment is about something else. Rows are compared rather than
        // bytes, since the gap between them is only ever whitespace.
        if below.start_position().row > sibling.end_position().row + 1 {
            break;
        }
        lines.push(strip_marker(sibling.utf8_text(source).unwrap_or_default()));
        below = sibling;
        previous = sibling.prev_sibling();
    }
    lines.reverse();
    let joined = dedent(&lines.join("\n"));
    (!joined.is_empty()).then_some(joined)
}

/// The outermost node holding this declaration as its only named child.
///
/// Climbing by "the parent starts at the same byte" does not work: an
/// `export_statement` starts at `export`, before the declaration it wraps.
fn outermost_wrapper(node: Node<'_>) -> Node<'_> {
    let mut at = node;
    while let Some(parent) = at.parent() {
        if parent.named_child_count() == 1 && parent.named_child(0) == Some(at) {
            at = parent;
        } else {
            break;
        }
    }
    at
}

fn is_comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "comment" | "line_comment" | "block_comment")
}

/// Whether a comment documents the declaration below it.
///
/// Rust says so in the grammar, twice over. A `///` or `/**` comment carries
/// an `outer` marker and documents what follows; a `//!` one carries an
/// `inner` marker and documents the *file*, so it belongs to the header and
/// never to the item under it; a plain `//` carries neither and is an aside.
///
/// Both distinctions matter for the same reason: whatever this returns true
/// for is copied into the text an embedder reads. Without the first, a
/// `// TODO: rewrite this` becomes a function's description. Without the
/// second, every file's header is repeated as the doc of its first item — so
/// the module's own subject is attached to one arbitrary function, and the
/// two read as the same thing.
///
/// Go has no such distinction and needs none: a comment above a declaration
/// is its documentation, by the language's own convention.
fn is_doc_comment(language: Language, node: Node<'_>) -> bool {
    match language {
        Language::Rust => node.child_by_field_name("outer").is_some(),
        _ => true,
    }
}

/// Strip the comment markers a language writes documentation with.
fn strip_marker(text: &str) -> String {
    let text = text.trim();
    if let Some(body) = text.strip_prefix("/**").and_then(|rest| rest.strip_suffix("*/")) {
        return body
            .lines()
            .map(|line| line.trim().trim_start_matches('*').trim())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
    }
    if let Some(body) = text.strip_prefix("/*").and_then(|rest| rest.strip_suffix("*/")) {
        return body.trim().to_string();
    }
    text.trim_start_matches("//!")
        .trim_start_matches("///")
        .trim_start_matches("//")
        .trim()
        .to_string()
}

/// The file's own header comment.
fn header_doc(language: Language, root: Node<'_>, source: &[u8]) -> Option<String> {
    let joined = match language {
        // `//!` is the file's own doc; `///` above the first item is not, and
        // the two sit side by side at file scope. The grammar tells them apart
        // by which marker field the comment carries.
        Language::Rust => leading_comments(root, source, |node| node.child_by_field_name("inner").is_some()),
        // A leading string literal, exactly as a function's docstring works.
        Language::Python => {
            let first = root.named_child(0)?;
            let string = if first.kind() == "string" { first } else { first.named_child(0)? };
            if string.kind() != "string" {
                return None;
            }
            dedent(&unquote(string.utf8_text(source).ok()?))
        }
        // The comment above the `package` clause, or the file's first comment.
        Language::Go | Language::TypeScript => leading_comments(root, source, |_| true),
    };
    (!joined.is_empty()).then_some(joined)
}

/// The run of comments a file opens with, as far as they keep qualifying.
fn leading_comments(root: Node<'_>, source: &[u8], qualifies: impl Fn(Node<'_>) -> bool) -> String {
    let mut lines = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if !is_comment(child) {
            break;
        }
        if !qualifies(child) {
            // A comment that does not qualify ends the header: in Rust a
            // `///` line at file scope belongs to the item under it, and
            // everything past it belongs to that item too.
            break;
        }
        lines.push(strip_marker(child.utf8_text(source).unwrap_or_default()));
    }
    dedent(&lines.join("\n"))
}

/// Remove the indentation every line shares.
///
/// A Python docstring carries the indentation of the function it sits in, and
/// a summary that kept it would embed four spaces per line as if they meant
/// something. The first line is excluded from the measurement: it follows the
/// opening quote on the same row and has no indentation of its own.
fn dedent(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let common = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .enumerate()
        .map(|(position, line)| {
            if position == 0 {
                line.trim_start()
            } else if line.len() >= common {
                &line[common..]
            } else {
                line.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Whether the language marks this declaration as visible outside its module.
fn is_public(language: Language, node: Node<'_>, source: &[u8]) -> bool {
    match language {
        // `pub`, `pub(crate)`, `pub(super)` — all of them are a visibility
        // modifier child. A declaration without one is private.
        Language::Rust => {
            let mut cursor = node.walk();
            node.children(&mut cursor).any(|child| child.kind() == "visibility_modifier")
        }
        // `export` wraps the declaration; a method inside an exported class is
        // public along with the class, which the climb finds.
        Language::TypeScript => {
            let mut at = Some(node);
            while let Some(current) = at {
                if current.kind() == "export_statement" {
                    return true;
                }
                at = current.parent();
            }
            false
        }
        // Go and Python say it in the name, which `implied_public` reads.
        Language::Go | Language::Python => implied_public(language, &name_text(node, source)),
    }
}

/// Whether a name alone marks a declaration as public.
///
/// Go capitalizes what it exports; Python prefixes what it does not with an
/// underscore. Both are conventions rather than keywords, and both are what
/// every tool in those ecosystems goes by.
fn implied_public(language: Language, name: &str) -> bool {
    match language {
        Language::Go => name.chars().next().is_some_and(char::is_uppercase),
        Language::Python => !name.starts_with('_'),
        _ => false,
    }
}

/// The declared name of a node, empty when it has none.
fn name_text(node: Node<'_>, source: &[u8]) -> String {
    node.child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_in_a_signature_collapses_to_one_line() {
        assert_eq!(collapse("fn add(\n    a: i32,\n    b: i32,\n) -> i32"), "fn add( a: i32, b: i32, ) -> i32");
    }

    #[test]
    fn a_shared_indent_is_removed_and_a_deeper_one_is_kept() {
        let text = "Summary.\n\n        Indented example.\n    Back again.";
        assert_eq!(dedent(text), "Summary.\n\n    Indented example.\nBack again.");
    }

    #[test]
    fn comment_markers_are_stripped_for_every_form() {
        assert_eq!(strip_marker("/// A doc comment."), "A doc comment.");
        assert_eq!(strip_marker("//! A header."), "A header.");
        assert_eq!(strip_marker("// An ordinary one."), "An ordinary one.");
        assert_eq!(strip_marker("/** A block doc. */"), "A block doc.");
        assert_eq!(strip_marker("/**\n * First.\n * Second.\n */"), "First.\nSecond.");
    }

    /// The triple quote has to go before the single one, or both ends keep a
    /// stray pair.
    #[test]
    fn a_python_string_loses_its_quotes_and_its_prefix() {
        assert_eq!(unquote("\"\"\"Adds.\"\"\""), "Adds.");
        assert_eq!(unquote("'''Adds.'''"), "Adds.");
        assert_eq!(unquote("\"Adds.\""), "Adds.");
        assert_eq!(unquote("r\"\"\"Adds.\"\"\""), "Adds.");
    }

    #[test]
    fn go_exports_by_capital_and_python_hides_by_underscore() {
        assert!(implied_public(Language::Go, "Add"));
        assert!(!implied_public(Language::Go, "add"));
        assert!(implied_public(Language::Python, "add"));
        assert!(!implied_public(Language::Python, "_add"));
    }
}
