//! What a module summary says about a file, in each of the four languages.
//!
//! The summary is the first thing this crate produces that is meant to be
//! *read* rather than looked up, and every part of it is lifted out of the
//! source rather than computed. So the tests here check the lifting: that the
//! signature stops where the body starts, that the doc found is the one the
//! author attached, and that a comment which is not documentation stays out.
//!
//! Every fixture is written for the test. Nothing in this crate's corpus comes
//! from a real repository — nooma indexes personal archives, and the rule for
//! this project is that not one fragment of real content reaches the code, the
//! tests or the docs.

use nooma_core::{ModuleSummary, SummaryEntry, lang::Language, symbols};

/// Parse a fixture and summarize it through the path the indexer uses.
///
/// `describe` rather than a `FileIndex` built here: a fixture assembled by
/// hand would go on passing after the indexer stopped attaching summaries at
/// all, which is the one regression these tests are for.
fn summarize(language: Language, source: &str) -> ModuleSummary {
    let (symbols, imports) = symbols::parse(language, source.as_bytes()).unwrap_or_else(|| panic!("{language}: the parser returned nothing"));
    let file = symbols::describe(format!("fixture.{language}"), language, String::new(), symbols, imports, source.as_bytes());
    file.summary.unwrap_or_else(|| panic!("{language}: no summary"))
}

/// One entry by name, or a message naming what was found instead.
fn entry<'a>(summary: &'a ModuleSummary, name: &str) -> &'a SummaryEntry {
    summary.entries.iter().find(|e| e.name == name).unwrap_or_else(|| {
        let found: Vec<&str> = summary.entries.iter().map(|e| e.name.as_str()).collect();
        panic!("no entry named `{name}`; found: {found:?}")
    })
}

const RUST: &str = r#"//! The ledger module.
//!
//! Keeps entries in balance.

use std::collections::BTreeMap;

/// A ledger that holds entries.
pub struct Ledger {
    balance: i64,
}

impl Ledger {
    /// Add an entry to the ledger.
    ///
    /// Returns the new balance.
    pub fn add(
        &mut self,
        amount: i64,
    ) -> i64 {
        self.balance += amount;
        self.balance
    }

    fn secret(&self) -> i64 {
        self.balance
    }
}

// An ordinary comment, not documentation.
pub fn helper() -> bool {
    true
}

/// A private detail.
fn hidden() {}
"#;

#[test]
fn a_rust_summary_leads_with_the_module_header() {
    let summary = summarize(Language::Rust, RUST);
    assert_eq!(summary.header.as_deref(), Some("The ledger module.\n\nKeeps entries in balance."));
}

#[test]
fn a_rust_signature_stops_where_the_body_starts() {
    let summary = summarize(Language::Rust, RUST);
    // Written across four lines in the fixture, and read back as one.
    assert_eq!(entry(&summary, "add").signature, "pub fn add( &mut self, amount: i64, ) -> i64");
}

#[test]
fn a_rust_doc_comment_is_attached_and_an_ordinary_comment_is_not() {
    let summary = summarize(Language::Rust, RUST);
    assert_eq!(
        entry(&summary, "add").doc.as_deref(),
        Some("Add an entry to the ledger.\n\nReturns the new balance.")
    );
    // `// An ordinary comment` sits directly above `helper`. Treating it as
    // documentation would put an aside into the text an embedder reads.
    assert_eq!(entry(&summary, "helper").doc, None);
}

/// `pub(crate)` is visible to the crate and to nothing outside it. Counting
/// it as public puts a crate's internals on the surface it advertises — and
/// that surface is exactly what a summary is for.
#[test]
fn rust_restricted_visibility_is_not_public() {
    let source = "pub fn open() {}
pub(crate) fn shared() {}
pub(super) fn upward() {}
pub(in crate::inner) fn scoped() {}
";
    let summary = summarize(Language::Rust, source);
    assert!(entry(&summary, "open").public);
    assert!(!entry(&summary, "shared").public, "pub(crate) is not visible outside the crate");
    assert!(!entry(&summary, "upward").public);
    assert!(!entry(&summary, "scoped").public);
}

#[test]
fn rust_visibility_comes_from_the_modifier() {
    let summary = summarize(Language::Rust, RUST);
    assert!(entry(&summary, "Ledger").public);
    assert!(entry(&summary, "add").public);
    assert!(!entry(&summary, "secret").public, "a method without `pub` is not public");
    assert!(!entry(&summary, "hidden").public, "a documented function is still private without `pub`");
}

const TYPESCRIPT: &str = r#"/**
 * The widgets module.
 */

import { useState } from "react";

/**
 * A button that can be pressed.
 */
export function Button(label: string): JSX.Element {
  return <button>{label}</button>;
}

/** The default panel. */
export default function Panel(): JSX.Element {
  return <div />;
}

/** Not exported. */
function internal(): void {}

export interface Props {
  label: string;
}
"#;

/// The case the probe caught: a doc comment sits above `export`, not above the
/// declaration inside it. Reading only the declaration's own siblings finds
/// nothing — and what it finds nothing for is every exported symbol, which is
/// exactly the public API a summary exists to describe.
#[test]
fn a_typescript_doc_is_found_through_the_export_wrapper() {
    let summary = summarize(Language::TypeScript, TYPESCRIPT);
    assert_eq!(entry(&summary, "Button").doc.as_deref(), Some("A button that can be pressed."));
    assert_eq!(entry(&summary, "Panel").doc.as_deref(), Some("The default panel."));
    assert_eq!(entry(&summary, "internal").doc.as_deref(), Some("Not exported."));
}

#[test]
fn typescript_visibility_comes_from_the_export() {
    let summary = summarize(Language::TypeScript, TYPESCRIPT);
    assert!(entry(&summary, "Button").public);
    assert!(entry(&summary, "Panel").public);
    assert!(entry(&summary, "Props").public);
    assert!(!entry(&summary, "internal").public, "a declaration without `export` is not public");
}

/// `export` is kept: it is half of what an exported signature says, and the
/// stage this summary serves is about the public surface of a module.
#[test]
fn a_typescript_signature_keeps_the_export_and_the_return_type() {
    let summary = summarize(Language::TypeScript, TYPESCRIPT);
    assert_eq!(entry(&summary, "Button").signature, "export function Button(label: string): JSX.Element");
    assert_eq!(entry(&summary, "internal").signature, "function internal(): void");
}

const PYTHON: &str = r#""""The ledger module.

Keeps entries in balance.
"""

import json


def add(a: int, b: int) -> int:
    """Add two numbers.

    Returns their sum.
    """
    return a + b


def _hidden() -> None:
    """Not part of the surface."""


class Ledger:
    """A ledger."""

    def total(self) -> int:
        return 0
"#;

#[test]
fn a_python_docstring_is_read_from_inside_the_body() {
    let summary = summarize(Language::Python, PYTHON);
    assert_eq!(summary.header.as_deref(), Some("The ledger module.\n\nKeeps entries in balance."));
    // The docstring is indented by the function it sits in; the indentation
    // is the function's, not the text's, and carrying it would embed four
    // spaces a line as though they meant something.
    assert_eq!(entry(&summary, "add").doc.as_deref(), Some("Add two numbers.\n\nReturns their sum."));
}

#[test]
fn a_python_signature_keeps_its_annotations() {
    let summary = summarize(Language::Python, PYTHON);
    assert_eq!(entry(&summary, "add").signature, "def add(a: int, b: int) -> int:");
}

#[test]
fn python_hides_what_starts_with_an_underscore() {
    let summary = summarize(Language::Python, PYTHON);
    assert!(entry(&summary, "add").public);
    assert!(entry(&summary, "Ledger").public);
    assert!(!entry(&summary, "_hidden").public);
}

const GO: &str = r#"// Package ledger keeps entries in balance.
package ledger

import "fmt"

// Add adds two numbers.
//
// It returns their sum.
func Add(a int, b int) int {
	return a + b
}

func unexported() {}

// Ledger holds entries.
type Ledger struct {
	balance int64
}
"#;

#[test]
fn a_go_doc_is_the_comment_above_the_declaration() {
    let summary = summarize(Language::Go, GO);
    assert_eq!(summary.header.as_deref(), Some("Package ledger keeps entries in balance."));
    assert_eq!(entry(&summary, "Add").doc.as_deref(), Some("Add adds two numbers.\n\nIt returns their sum."));
    assert_eq!(entry(&summary, "Ledger").doc.as_deref(), Some("Ledger holds entries."));
}

/// Go hangs a type's definition off a `type` field rather than a `body` one,
/// and the declaration the doc sits above is the outer `type_declaration`
/// while the name is on the `type_spec` inside it. A body-only rule finds no
/// node here, and every Go type in the repository loses its signature and its
/// documentation while the functions beside it keep theirs.
#[test]
fn a_go_type_has_a_signature_though_it_has_no_body_field() {
    let summary = summarize(Language::Go, GO);
    assert_eq!(entry(&summary, "Ledger").signature, "type Ledger struct");
}

#[test]
fn go_exports_what_it_capitalizes() {
    let summary = summarize(Language::Go, GO);
    assert!(entry(&summary, "Add").public);
    assert!(entry(&summary, "Ledger").public);
    assert!(!entry(&summary, "unexported").public);
}

/// `//!` documents the file, `///` documents what follows it, and the two sit
/// side by side at file scope. Reading the header as the first item's doc
/// attaches the module's own subject to one arbitrary function — and then the
/// module and that function embed as the same thing, which is the one failure
/// a semantic index cannot recover from.
#[test]
fn a_rust_file_header_is_not_the_first_items_doc() {
    let source = "//! The module's own subject.\n\n/// The function's own subject.\npub fn first() {}\n";
    let summary = summarize(Language::Rust, source);
    assert_eq!(summary.header.as_deref(), Some("The module's own subject."));
    assert_eq!(entry(&summary, "first").doc.as_deref(), Some("The function's own subject."));
}

/// The same, with nothing between the header and the item: the blank-line rule
/// cannot be what separates them, so the marker has to.
#[test]
fn a_rust_header_touching_an_item_is_still_not_its_doc() {
    let source = "//! The module's own subject.\npub fn first() {}\n";
    let summary = summarize(Language::Rust, source);
    assert_eq!(summary.header.as_deref(), Some("The module's own subject."));
    assert_eq!(entry(&summary, "first").doc, None);
}

/// A comment separated from a declaration by a blank line is about something
/// else — usually the section above it. Attaching it anyway is the kind of
/// quiet wrongness that only shows up as search returning the wrong module.
///
/// Written in Go on purpose. The same fixture in Rust stays green whatever
/// the blank-line rule does, because the doc-marker guard rejects a plain
/// `//` before the gap is ever measured — so a Rust fixture would test the
/// marker twice and the gap not at all. Go has no marker, which leaves the
/// gap as the only thing that can reject the comment.
#[test]
fn a_comment_across_a_blank_line_is_not_a_doc() {
    let source = "package p\n\n// A note about the section above.\n\nfunc After() {}\n";
    let summary = summarize(Language::Go, source);
    assert_eq!(entry(&summary, "After").doc, None);
}

/// The gap is measured between a comment and whatever follows it, not between
/// every comment and the declaration: a run of comment lines is one doc, and
/// measuring each line against the declaration would keep only the last as
/// soon as the doc grew past a single line.
#[test]
fn a_run_of_comment_lines_is_one_doc() {
    let source = "package p\n\n// First line.\n// Second line.\n// Third line.\nfunc After() {}\n";
    let summary = summarize(Language::Go, source);
    assert_eq!(entry(&summary, "After").doc.as_deref(), Some("First line.\nSecond line.\nThird line."));
}

/// The text is what an embedder will be handed, so it is asserted whole
/// rather than by substring: a check for "does it contain the name" passes on
/// output nobody would want to read.
#[test]
fn the_text_is_the_header_and_the_public_surface() {
    let source = "//! A small module.\n\n/// Adds two numbers.\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n/// Kept to itself.\nfn private() {}\n";
    let summary = summarize(Language::Rust, source);
    assert_eq!(
        summary.to_text(),
        "fixture.rust\n\nA small module.\n\npub fn add(a: i32, b: i32) -> i32\nAdds two numbers."
    );
    assert_eq!(summary.public_count(), 1, "a private function has no place on the surface");
}

/// A file with nothing in it still summarizes: an empty answer is a fact, and
/// a caller that has to guard against `None` per file would guard wrongly.
#[test]
fn an_empty_file_summarizes_to_nothing_rather_than_failing() {
    let summary = summarize(Language::Rust, "");
    assert_eq!(summary.header, None);
    assert!(summary.entries.is_empty());
    assert_eq!(summary.to_text(), "fixture.rust");
}

/// A directive is addressed to a tool, not to a reader, and it sits exactly
/// where a header is looked for. Measured on a real TypeScript repository: 88
/// of the 109 headers found were directives, `@vitest-environment jsdom` for
/// every test file in the tree. Since the header is the field most likely to
/// say what a module is about, letting a directive fill it would make every
/// test file in a project embed as the same thing.
#[test]
fn a_directive_is_not_a_module_header() {
    let ts = summarize(Language::TypeScript, "// @ts-check\nexport function f() {}\n");
    assert_eq!(ts.header, None);

    let vitest = summarize(Language::TypeScript, "/** @vitest-environment jsdom */\nexport function f() {}\n");
    assert_eq!(vitest.header, None);

    let eslint = summarize(Language::TypeScript, "/* eslint-disable no-console */\nexport function f() {}\n");
    assert_eq!(eslint.header, None);

    let go = summarize(Language::Go, "//go:build linux\n\npackage p\n\nfunc F() {}\n");
    assert_eq!(go.header, None);

    let url = summarize(Language::TypeScript, "// https://astro.build/config\nexport function f() {}\n");
    assert_eq!(url.header, None);
}

/// A directive is skipped rather than ending the run: it is routinely followed
/// by the comment that does describe the file, and stopping at the directive
/// would lose that one too.
#[test]
fn a_header_after_a_directive_is_still_found() {
    let summary = summarize(Language::TypeScript, "// @ts-check\n// The widgets module.\nexport function f() {}\n");
    assert_eq!(summary.header.as_deref(), Some("The widgets module."));
}

/// The same rule where a declaration's own doc is read: a build constraint
/// above a function is not a description of the function.
#[test]
fn a_directive_above_a_declaration_is_not_its_doc() {
    let summary = summarize(Language::Go, "package p\n\n//go:noinline\nfunc Slow() {}\n");
    assert_eq!(entry(&summary, "Slow").doc, None);
}

/// A comment that merely begins with a word a directive also begins with is
/// still prose. The test guards the rule from being widened into a filter that
/// eats descriptions.
#[test]
fn ordinary_prose_is_not_mistaken_for_a_directive() {
    let summary = summarize(Language::Go, "// Global state lives here, deliberately.\npackage p\n\nfunc F() {}\n");
    assert_eq!(summary.header.as_deref(), Some("Global state lives here, deliberately."));

    let typed = summarize(Language::TypeScript, "// Type checking happens at the edges.\nexport function f() {}\n");
    assert_eq!(typed.header.as_deref(), Some("Type checking happens at the edges."));
}
