//! What each language's queries actually pull out of source.
//!
//! A tree-sitter query that compiles and captures nothing is the failure this
//! file exists to catch: the grammar accepts it, the parse succeeds, and the
//! index comes out empty. Compiling every query — which `lang` does — proves
//! only that the node names exist, not that they are the ones a declaration
//! uses.
//!
//! Every fixture here is written for the test. Nothing in this crate's test
//! corpus comes from a real repository: nooma indexes personal archives, and
//! the rule for this project is that not one fragment of real content reaches
//! the code, the tests or the docs.

use nooma_core::{Import, Symbol, SymbolKind, lang::Language, symbols};

/// Parse a fixture, or say which language failed rather than unwrapping.
fn parse(language: Language, source: &str) -> (Vec<Symbol>, Vec<Import>) {
    symbols::parse(language, source.as_bytes()).unwrap_or_else(|| panic!("{language}: the parser returned nothing"))
}

/// The names of every symbol of a kind, in source order.
fn names_of(symbols: &[Symbol], kind: SymbolKind) -> Vec<&str> {
    symbols.iter().filter(|s| s.kind == kind).map(|s| s.name.as_str()).collect()
}

/// One symbol by name, or a message naming what was found instead.
fn find<'a>(symbols: &'a [Symbol], name: &str) -> &'a Symbol {
    symbols.iter().find(|s| s.name == name).unwrap_or_else(|| {
        let found: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        panic!("no symbol named `{name}`; found: {found:?}")
    })
}

fn modules(imports: &[Import]) -> Vec<&str> {
    imports.iter().map(|i| i.module.as_str()).collect()
}

const RUST: &str = r#"
use std::collections::BTreeMap;
use super::widgets::Button;

pub const LIMIT: usize = 10;
static GREETING: &str = "hello";

pub mod inner {
    pub fn nested() {}
}

pub struct Ledger {
    balance: i64,
}

pub enum Entry {
    Debit,
    Credit,
}

pub trait Balanced {
    fn is_balanced(&self) -> bool;
}

pub type Amount = i64;

impl Ledger {
    pub fn post(&mut self, amount: i64) {
        self.balance += amount;
    }
}

fn free_function() {}
"#;

#[test]
fn rust_symbols_are_found_with_their_kinds() {
    let (symbols, _) = parse(Language::Rust, RUST);

    assert_eq!(names_of(&symbols, SymbolKind::Function), ["nested", "post", "free_function"]);
    assert_eq!(
        names_of(&symbols, SymbolKind::Type),
        ["Ledger", "Entry", "Balanced", "Amount"],
        "`Ledger` once: the impl block lends its span to `post` but declares nothing"
    );
    assert_eq!(names_of(&symbols, SymbolKind::Module), ["inner"]);
    assert_eq!(names_of(&symbols, SymbolKind::Constant), ["LIMIT", "GREETING"]);
}

#[test]
fn rust_lines_point_at_the_declaration() {
    let (symbols, _) = parse(Language::Rust, RUST);
    // The raw string opens with a newline, so its first line of code is line
    // 2 — the same count an editor would show for this text in a file.
    assert_eq!(find(&symbols, "Ledger").line, 12);
    assert_eq!(find(&symbols, "free_function").line, 33);
}

#[test]
fn a_method_belongs_to_its_impl_and_a_free_function_to_nobody() {
    let (symbols, _) = parse(Language::Rust, RUST);
    assert_eq!(find(&symbols, "post").parent.as_deref(), Some("Ledger"));
    assert_eq!(find(&symbols, "nested").parent.as_deref(), Some("inner"));
    assert_eq!(find(&symbols, "free_function").parent, None);
    assert_eq!(find(&symbols, "inner").parent, None);
}

/// Found on a live run over this repository, not by any test: every type with
/// an `impl` block was listed twice, once for the declaration and once for the
/// block. Searching for a type then returns it once per impl it has.
#[test]
fn an_impl_block_does_not_declare_the_type_again() {
    let source = "
pub struct Ledger {}
impl Ledger {
    pub fn post() {}
}
impl Ledger {
    pub fn balance() {}
}
impl<T> Ledger<T> {
    pub fn generic() {}
}
";
    let (symbols, _) = parse(Language::Rust, source);

    assert_eq!(names_of(&symbols, SymbolKind::Type), ["Ledger"], "three impl blocks, one declaration");
    // The spans still do their job: each method knows its type.
    for method in ["post", "balance", "generic"] {
        assert_eq!(find(&symbols, method).parent.as_deref(), Some("Ledger"), "{method}");
    }
}

#[test]
fn rust_imports_are_recorded_whole() {
    let (_, imports) = parse(Language::Rust, RUST);
    assert_eq!(modules(&imports), ["std::collections::BTreeMap", "super::widgets::Button"]);
}

#[test]
fn a_braced_use_is_one_dependency_not_several() {
    let (_, imports) = parse(Language::Rust, "use std::io::{Read, Write};");
    assert_eq!(modules(&imports), ["std::io::{Read, Write}"]);
}

const TYPESCRIPT: &str = r#"
import { useState } from "react";
import Button from "./widgets/Button";
export { Card } from "./widgets/Card";
const legacy = require("./legacy");
const lazy = await import("./lazy");

export const TIMEOUT = 5000;

export function render(): void {}

export const Panel = ({ title }: { title: string }) => <div>{title}</div>;

export class Store {
  save(): void {}
}

export interface Entry {
  id: string;
}

export type Amount = number;

export enum Status {
  Open,
}
"#;

#[test]
fn typescript_symbols_are_found_with_their_kinds() {
    let (symbols, _) = parse(Language::TypeScript, TYPESCRIPT);

    assert_eq!(
        names_of(&symbols, SymbolKind::Function),
        ["render", "Panel", "save"],
        "`Panel` is an arrow function bound to a const — a function, not a constant"
    );
    assert_eq!(names_of(&symbols, SymbolKind::Type), ["Store", "Entry", "Amount", "Status"]);
    assert_eq!(names_of(&symbols, SymbolKind::Constant), ["legacy", "lazy", "TIMEOUT"]);
}

#[test]
fn a_typescript_method_belongs_to_its_class() {
    let (symbols, _) = parse(Language::TypeScript, TYPESCRIPT);
    assert_eq!(find(&symbols, "save").parent.as_deref(), Some("Store"));
    assert_eq!(find(&symbols, "render").parent, None);
}

#[test]
fn typescript_imports_cover_every_spelling() {
    let (_, imports) = parse(Language::TypeScript, TYPESCRIPT);
    assert_eq!(
        modules(&imports),
        ["react", "./widgets/Button", "./widgets/Card", "./legacy", "./lazy"],
        "static, re-export, require and dynamic import all name a module"
    );
}

#[test]
fn a_one_string_call_is_not_an_import() {
    let (_, imports) = parse(Language::TypeScript, r#"const label = t("./looks-like-a-path");"#);
    assert!(imports.is_empty(), "only `require` names a module, not every call: {imports:?}");
}

const PYTHON: &str = r#"
import os.path
import numpy as np
from . import widgets
from .models import Entry
from collections.abc import Iterable

TIMEOUT = 30
retries: int = 3
lowercase_is_not_a_constant = 1

def render():
    def helper():
        pass

class Store:
    def save(self):
        pass
"#;

#[test]
fn python_symbols_are_found_with_their_kinds() {
    let (symbols, _) = parse(Language::Python, PYTHON);

    assert_eq!(names_of(&symbols, SymbolKind::Function), ["render", "helper", "save"]);
    assert_eq!(names_of(&symbols, SymbolKind::Type), ["Store"]);
    assert_eq!(
        names_of(&symbols, SymbolKind::Constant),
        ["TIMEOUT"],
        "the convention is the only rule Python has: `retries` is typed but lowercase, \
         and a lowercase binding is a variable"
    );
}

#[test]
fn a_python_method_belongs_to_its_class_and_a_nested_def_to_its_function() {
    let (symbols, _) = parse(Language::Python, PYTHON);
    assert_eq!(find(&symbols, "save").parent.as_deref(), Some("Store"));
    assert_eq!(find(&symbols, "helper").parent.as_deref(), Some("render"));
    assert_eq!(find(&symbols, "render").parent, None);
}

#[test]
fn python_imports_keep_the_relative_dots() {
    let (_, imports) = parse(Language::Python, PYTHON);
    assert_eq!(modules(&imports), ["os.path", "numpy", ".", ".models", "collections.abc"]);
}

const GO: &str = r#"
package ledger

import "fmt"

import (
	"net/http"
	"github.com/lacodda/example/store"
)

const Limit = 10

var greeting = "hello"

type Ledger struct {
	balance int64
}

type Balanced interface {
	IsBalanced() bool
}

type Amount = int64

func Post(amount int64) {}

func (l *Ledger) Balance() int64 {
	return l.balance
}
"#;

#[test]
fn go_symbols_are_found_with_their_kinds() {
    let (symbols, _) = parse(Language::Go, GO);

    assert_eq!(
        names_of(&symbols, SymbolKind::Function),
        ["Post", "Balance"],
        "a method is a function; Go declares it beside its type, not inside it"
    );
    assert_eq!(names_of(&symbols, SymbolKind::Type), ["Ledger", "Balanced", "Amount"]);
    assert_eq!(names_of(&symbols, SymbolKind::Module), ["ledger"]);
    assert_eq!(names_of(&symbols, SymbolKind::Constant), ["Limit", "greeting"]);
}

#[test]
fn go_imports_cover_the_single_and_the_grouped_form() {
    let (_, imports) = parse(Language::Go, GO);
    assert_eq!(
        modules(&imports),
        ["fmt", "net/http", "github.com/lacodda/example/store"],
        "quotes are trimmed, and a parenthesised block is as good as a lone import"
    );
}

#[test]
fn a_file_the_grammar_chokes_on_still_yields_what_it_could_read() {
    // tree-sitter recovers rather than refusing; half a file is better than
    // none, and an index that dropped every file with a syntax error would
    // drop exactly the file someone is in the middle of editing.
    let (symbols, _) = parse(Language::Rust, "fn good() {} fn (((");
    assert_eq!(names_of(&symbols, SymbolKind::Function), ["good"]);
}

#[test]
fn an_empty_file_is_not_an_error() {
    let (symbols, imports) = parse(Language::Rust, "");
    assert!(symbols.is_empty());
    assert!(imports.is_empty());
}
