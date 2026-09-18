//! Indexing a whole repository: what the walk includes, and what it leaves out.
//!
//! These tests build a git repository from nothing, because the questions they
//! ask — which commit is this, does `.gitignore` apply, does the index survive
//! a restart — have no answer without one. The fixture corpus is invented for
//! the test; no path, name or line here comes from a real project.

use std::path::Path;
use std::process::Command;

use nooma_core::{RepoIndex, Store, incremental, repo::Repo};

/// Run a git command in `dir`, failing loudly if git itself refuses.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        // A developer's global config can carry hooks, a signing key or a
        // template directory, any of which turns `commit` here into a failure
        // that has nothing to do with nooma.
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_SYSTEM", "")
        .env("GIT_AUTHOR_NAME", "nooma test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "nooma test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git is not on PATH");
    assert!(output.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&output.stderr));
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A repository with one commit, holding source in four languages plus the
/// kinds of file that must not reach the index.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    git(root, &["init", "--initial-branch=main"]);

    write(root, ".gitignore", "target/\nnode_modules/\n");
    write(root, ".nooma-ignore", "vendor/\n");

    write(root, "src/ledger.rs", "pub struct Ledger {}\nimpl Ledger {\n    pub fn post() {}\n}\n");
    write(root, "src/util.rs", "pub fn helper() {}\n");
    write(root, "web/app.ts", "import { Card } from \"./card\";\nexport const run = () => {};\n");
    write(root, "web/card.ts", "export class Card {}\n");
    write(root, "tools/build.py", "def build():\n    pass\n");
    write(root, "cmd/serve.go", "package cmd\n\nfunc Serve() {}\n");

    // Not source: no language claims these, so the walk should pass over them.
    write(root, "README.md", "# fixture\n");
    write(root, "Cargo.toml", "[package]\nname = \"fixture\"\n");

    // Ignored: build output and vendored code. Both parse perfectly well, so
    // nothing but the ignore rules keeps them out.
    write(root, "target/debug/generated.rs", "pub fn from_the_build_directory() {}\n");
    write(root, "node_modules/dep/index.ts", "export function fromADependency() {}\n");
    write(root, "vendor/copied.rs", "pub fn vendored() {}\n");

    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "the fixture"]);
    dir
}

fn index_of(root: &Path) -> (Repo, RepoIndex) {
    let repo = Repo::discover(root).unwrap();
    let (index, _) = incremental::update(&repo, None).unwrap();
    (repo, index)
}

fn paths(index: &RepoIndex) -> Vec<&str> {
    index.files.iter().map(|f| f.path.as_str()).collect()
}

#[test]
fn the_walk_takes_source_and_leaves_the_rest() {
    let dir = fixture();
    let (_, index) = index_of(dir.path());

    assert_eq!(
        paths(&index),
        ["cmd/serve.go", "src/ledger.rs", "src/util.rs", "tools/build.py", "web/app.ts", "web/card.ts"],
        "six source files, sorted; no markdown, no manifest, nothing ignored"
    );
}

/// The rule this project states out loud: `target/` and `node_modules/` are
/// never indexed by default. Without it the repository's own symbols sit under
/// its dependencies', and every search answers with a build artifact.
#[test]
fn gitignore_and_nooma_ignore_both_keep_files_out() {
    let dir = fixture();
    let (_, index) = index_of(dir.path());
    let indexed = paths(&index);

    for kept_out in ["target/debug/generated.rs", "node_modules/dep/index.ts", "vendor/copied.rs"] {
        assert!(!indexed.contains(&kept_out), "{kept_out} reached the index: {indexed:?}");
    }
    // The symbols are the real test: a path can be absent because the walk
    // renamed it, but a symbol only exists if the file was parsed.
    let names: Vec<&str> = index.files.iter().flat_map(|f| f.symbols.iter()).map(|s| s.name.as_str()).collect();
    for kept_out in ["from_the_build_directory", "fromADependency", "vendored"] {
        assert!(!names.contains(&kept_out), "{kept_out} reached the index: {names:?}");
    }
}

#[test]
fn every_indexed_file_carries_a_content_hash() {
    let dir = fixture();
    let (_, index) = index_of(dir.path());

    for file in &index.files {
        assert_eq!(file.content_hash.len(), 64, "{}: not a blake3 hex digest", file.path);
    }
    let hashes: std::collections::BTreeSet<&str> = index.files.iter().map(|f| f.content_hash.as_str()).collect();
    assert_eq!(hashes.len(), index.files.len(), "distinct files must hash differently");
}

#[test]
fn the_index_is_pinned_to_the_commit_it_was_built_from() {
    let dir = fixture();
    let (repo, index) = index_of(dir.path());

    assert_eq!(index.revision.commit, repo.commit());
    assert_eq!(index.revision.commit.len(), 40, "the full hex id, not an abbreviation");

    // A second commit must give a different answer — that is what makes the
    // pin worth storing.
    write(dir.path(), "src/extra.rs", "pub fn added_later() {}\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "one more"]);
    let (_, second) = index_of(dir.path());

    assert_ne!(second.revision.commit, index.revision.commit);
    assert!(paths(&second).contains(&"src/extra.rs"));
}

/// Two runs over one commit must produce the same bytes, or nothing built on
/// top of the index can tell a real change from reshuffled output.
#[test]
fn indexing_the_same_commit_twice_gives_the_same_bytes() {
    let dir = fixture();
    let (_, first) = index_of(dir.path());
    let (_, second) = index_of(dir.path());

    assert_eq!(serde_json::to_string(&first).unwrap(), serde_json::to_string(&second).unwrap());
}

#[test]
fn a_stored_index_comes_back_whole() {
    let dir = fixture();
    let store_dir = tempfile::tempdir().unwrap();
    let store = Store::open_at(store_dir.path()).unwrap();
    let (repo, index) = index_of(dir.path());

    store.save(&index).unwrap();
    let loaded = store.load(repo.root()).unwrap().expect("just saved");

    assert_eq!(loaded, index);
    assert!(loaded.symbol_count() > 0, "an index with no symbols proves nothing");
}

#[test]
fn a_relative_import_resolves_to_the_file_it_names() {
    let dir = fixture();
    let (_, index) = index_of(dir.path());
    let graph = index.module_dependencies();

    assert_eq!(
        graph.get("web/app.ts").map(Vec::as_slice),
        Some(["web/card.ts"].as_slice()),
        "`./card` from web/app.ts is web/card.ts"
    );
}

/// An import of something outside the repository is a fact about the file, not
/// an edge in its graph — and a wrong edge is worse than a missing one.
#[test]
fn an_import_with_no_file_here_is_not_an_edge() {
    let dir = fixture();
    write(dir.path(), "web/outside.ts", "import { useState } from \"react\";\n");
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "an outside import"]);

    let (_, index) = index_of(dir.path());
    let file = index.files.iter().find(|f| f.path == "web/outside.ts").expect("indexed");

    assert_eq!(file.imports.len(), 1, "the import is recorded on the file");
    assert_eq!(file.imports[0].module, "react");
    assert!(
        !index.module_dependencies().contains_key("web/outside.ts"),
        "but it is not an edge: there is no react in this repository"
    );
}

/// Found on a live run, not by any test: `canonicalize` on Windows returns
/// `\\?\C:\...`, and the prefix reached `--json`, the store key and every
/// message. Nothing else on the machine produces that spelling, so the path
/// never equalled what the caller passed.
#[test]
fn the_root_is_a_path_a_caller_would_recognise() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let root = repo.root().to_string_lossy();

    assert!(!root.starts_with(r"\\?\"), "the extended-length prefix leaked: {root}");
    // And it is still the same directory, not a different one that merely
    // looks tidier.
    assert!(repo.root().join(".gitignore").is_file(), "{root} is not the fixture");
}

#[test]
fn a_directory_that_is_not_a_repository_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let error = Repo::discover(dir.path()).unwrap_err();
    assert!(matches!(error, nooma_core::Error::NotARepository(_)), "expected a plain refusal, got: {error}");
}

/// `git init` and nothing else is a normal state, and it must not read as
/// "this repository has no symbols".
#[test]
fn a_repository_with_no_commits_says_so() {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "--initial-branch=main"]);

    let error = Repo::discover(dir.path()).unwrap_err();
    assert!(
        matches!(error, nooma_core::Error::EmptyRepository(_)),
        "expected an empty-repository refusal, got: {error}"
    );
}

#[test]
fn any_path_inside_the_work_tree_finds_the_repository() {
    let dir = fixture();
    let from_root = Repo::discover(dir.path()).unwrap();
    let from_subdirectory = Repo::discover(dir.path().join("src")).unwrap();
    let from_a_file = Repo::discover(dir.path().join("src/ledger.rs")).unwrap();

    assert_eq!(from_subdirectory.root(), from_root.root());
    assert_eq!(from_a_file.root(), from_root.root());
    assert_eq!(from_a_file.commit(), from_root.commit());
}
