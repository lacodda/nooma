//! Reindexing only what changed, and staying honest about it.
//!
//! The thing these tests defend is not the speed - it is that the shortcut
//! gives the same answer as doing the work. An incremental pass that is fast
//! and subtly different from a full one is worse than no incremental pass at
//! all, because the difference shows up as search having quietly stopped
//! finding something.
//!
//! Every fixture is written for the test; nothing here comes from a real
//! repository.

use std::path::Path;
use std::process::Command;

use nooma_core::index::CHUNKER_VERSION;
use nooma_core::{RepoIndex, incremental, repo::Repo};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_SYSTEM", "")
        .env("GIT_AUTHOR_NAME", "nooma test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "nooma test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .expect("git is not on PATH");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A repository with three source files, committed.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "--initial-branch=main"]);
    write(root, ".gitignore", "target/\n");
    write(root, "src/ledger.rs", "pub struct Ledger {}\nimpl Ledger {\n    pub fn post() {}\n}\n");
    write(root, "src/util.rs", "pub fn helper() {}\n");
    write(root, "web/app.ts", "export const run = () => {};\n");
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "the fixture"]);
    dir
}

fn full(repo: &Repo) -> RepoIndex {
    incremental::update(repo, None).unwrap().0
}

#[test]
fn a_first_pass_parses_everything_and_a_second_parses_nothing() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let (first, one) = incremental::update(&repo, None).unwrap();
    assert_eq!(one.added, 3);
    assert_eq!(one.unchanged, 0);
    assert_eq!(one.parsed(), 3);
    assert!(!one.is_noop());

    let (_, two) = incremental::update(&repo, Some(&first)).unwrap();
    assert_eq!(two.unchanged, 3);
    assert_eq!(two.parsed(), 0);
    assert!(two.is_noop());
}

/// The contract the whole stage rests on: a reused entry is the entry a full
/// parse would have produced. If reuse and parsing can differ, the fast path
/// is a way of being wrong quickly.
#[test]
fn reuse_gives_exactly_what_parsing_gives() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let first = full(&repo);
    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");

    let (incremental_pass, update) = incremental::update(&repo, Some(&first)).unwrap();
    assert_eq!(update.changed, 1, "one file edited");
    assert_eq!(update.unchanged, 2, "the other two carried across");

    let full_pass = full(&repo);
    assert_eq!(
        incremental_pass, full_pass,
        "an index built by reusing must equal one built by parsing everything"
    );
}

#[test]
fn a_new_file_is_added_and_a_deleted_one_is_removed() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);

    write(dir.path(), "src/extra.rs", "pub fn added_later() {}\n");
    let (second, update) = incremental::update(&repo, Some(&first)).unwrap();
    assert_eq!(update.added, 1);
    assert_eq!(update.unchanged, 3);
    assert_eq!(second.files.len(), 4);

    std::fs::remove_file(dir.path().join("src/util.rs")).unwrap();
    let (third, update) = incremental::update(&repo, Some(&second)).unwrap();
    assert_eq!(update.removed, 1);
    assert_eq!(update.parsed(), 0, "a deletion needs no parser");
    assert!(!update.is_noop(), "but the index did change");
    assert_eq!(third.files.len(), 3);
    assert!(!third.files.iter().any(|f| f.path == "src/util.rs"));
}

/// A file touched without being changed must not be reparsed. Timestamps move
/// for reasons that have nothing to do with content - a checkout, a build, a
/// sync - and reparsing on a timestamp would give up most of the saving.
#[test]
fn a_file_rewritten_with_the_same_bytes_is_not_reparsed() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);

    let path = dir.path().join("src/util.rs");
    let same = std::fs::read(&path).unwrap();
    std::fs::write(&path, &same).unwrap();

    let (_, update) = incremental::update(&repo, Some(&first)).unwrap();
    assert!(update.is_noop(), "the bytes are the same, whatever the clock says");
    assert_eq!(update.unchanged, 3);
}

/// A file edited and then edited back is the state it started in. The hash
/// says so; anything tracking edits rather than content would not.
#[test]
fn an_edit_that_is_undone_leaves_nothing_to_do() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);

    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn temporary() {}\n");
    let (second, _) = incremental::update(&repo, Some(&first)).unwrap();
    write(dir.path(), "src/util.rs", "pub fn helper() {}\n");

    let (third, update) = incremental::update(&repo, Some(&second)).unwrap();
    assert_eq!(update.changed, 1, "the bytes did move, so it is parsed once more");
    assert_eq!(third, first, "and the answer is exactly where it started");
}

#[test]
fn a_clean_tree_is_clean_and_an_edited_one_is_dirty() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let clean = full(&repo);
    assert!(!clean.revision.dirty, "nothing has been touched since the commit");
    assert_eq!(clean.revision.commit, repo.commit());

    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");
    let edited = full(&repo);
    assert!(edited.revision.dirty, "an edit the commit does not hold");
    assert_eq!(edited.revision.commit, repo.commit(), "still the same commit, though");
}

/// A deletion is a difference from the commit, and it is the one direction a
/// question asked about the working tree cannot see: the walk only ever
/// reports files that exist.
#[test]
fn deleting_a_committed_file_makes_the_tree_dirty() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    assert!(!full(&repo).revision.dirty);

    std::fs::remove_file(dir.path().join("src/util.rs")).unwrap();
    assert!(full(&repo).revision.dirty, "the commit holds a file the tree has lost");
}

/// An untracked file is a difference too. It is in the tree and not in the
/// commit, which is the same kind of fact as an edit.
#[test]
fn an_untracked_file_makes_the_tree_dirty() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    assert!(!full(&repo).revision.dirty);

    write(dir.path(), "src/scratch.rs", "pub fn scratch() {}\n");
    assert!(full(&repo).revision.dirty);
}

/// Committing changes no file's bytes and still changes the revision. An index
/// that only watched files would keep claiming the tree is dirty.
#[test]
fn committing_changes_the_revision_without_changing_a_file() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");

    let before = full(&repo);
    assert!(before.revision.dirty);

    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-m", "the edit"]);
    let repo = Repo::discover(dir.path()).unwrap();

    let (after, update) = incremental::update(&repo, Some(&before)).unwrap();
    assert!(update.is_noop(), "no file's bytes moved");
    assert!(!after.revision.dirty, "but the tree now matches the commit");
    assert_ne!(after.revision.commit, before.revision.commit);
    assert_ne!(after, before, "so the index is not the one that was stored");
}

/// What is ignored in the working tree must be ignored in the commit too, or
/// a checked-in `target/` reads as a permanent difference and the tree is
/// never clean again.
#[test]
fn a_committed_but_ignored_file_does_not_make_the_tree_dirty() {
    let dir = fixture();
    // Committed before it was ignored, which is how this happens in life.
    write(dir.path(), "target/generated.rs", "pub fn generated() {}\n");
    git(dir.path(), &["add", "-f", "target/generated.rs"]);
    git(dir.path(), &["commit", "-m", "an artifact that slipped in"]);

    let repo = Repo::discover(dir.path()).unwrap();
    let index = full(&repo);

    assert!(!index.files.iter().any(|f| f.path.starts_with("target/")), "the walk skips it");
    assert!(!index.revision.dirty, "and so must the comparison with the commit");
}

/// The same, for a rule that names files rather than a directory.
///
/// Pruning an ignored directory keeps `target/` out whether or not the files
/// inside it are checked as well, so that case alone leaves the per-file rule
/// untested - and an untested guard is one that can be deleted without anything
/// going red.
#[test]
fn a_committed_file_ignored_by_name_does_not_make_the_tree_dirty() {
    let dir = fixture();
    write(dir.path(), ".nooma-ignore", "*.generated.rs\n");
    write(dir.path(), "src/schema.generated.rs", "pub fn generated() {}\n");
    git(dir.path(), &["add", "-f", ".nooma-ignore", "src/schema.generated.rs"]);
    git(dir.path(), &["commit", "-m", "a generated file that was committed"]);

    let repo = Repo::discover(dir.path()).unwrap();
    let index = full(&repo);

    assert!(
        !index.files.iter().any(|f| f.path.ends_with(".generated.rs")),
        "the walk skips it: {:?}",
        index.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert!(!index.revision.dirty, "and so must the comparison with the commit");
}

/// Changing how files are cut up changes what every entry means while the
/// bytes, and so the hashes, stay identical. Without the version the old
/// entries would be kept and extended by the new rules.
#[test]
fn entries_cut_up_by_other_rules_are_not_reused() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let mut stale = full(&repo);
    stale.chunker_version = CHUNKER_VERSION + 1;
    assert!(!stale.is_reusable());

    let (_, update) = incremental::update(&repo, Some(&stale)).unwrap();
    assert_eq!(update.unchanged, 0, "nothing may be carried across");
    assert_eq!(update.added, 3, "every file goes back to the parser");
}

#[test]
fn an_index_of_one_revision_is_the_same_bytes_every_run() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let first = full(&repo);
    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");
    let (through_reuse, _) = incremental::update(&repo, Some(&first)).unwrap();
    let through_parsing = full(&repo);

    assert_eq!(
        serde_json::to_string(&through_reuse).unwrap(),
        serde_json::to_string(&through_parsing).unwrap(),
        "the order files come back in must not depend on which ones were parsed"
    );
}

/// Every indexed file carries a summary, whether it was parsed on this pass or
/// carried across from the last one. A summary that survives only on the
/// parsing path would leave the cache full of files that answer "nothing to
/// say" — and since the reuse path is the common one, almost every file would.
#[test]
fn a_reused_file_keeps_its_summary() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let first = full(&repo);
    let summarized = |index: &RepoIndex| index.files.iter().filter(|f| f.summary.is_some()).count();
    assert_eq!(summarized(&first), 3, "a full pass summarizes every file");

    // One file changes; the other two must come across with their summaries.
    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");
    let (second, update) = incremental::update(&repo, Some(&first)).unwrap();
    assert_eq!(update.unchanged, 2);
    assert_eq!(summarized(&second), 3, "the two reused files kept theirs, and the parsed one got a new one");
}

/// The summary of an unchanged file is the same summary, not merely a present
/// one: the cache is only worth having if what it hands back equals what a
/// fresh parse would produce.
#[test]
fn a_reused_summary_equals_the_one_a_fresh_parse_gives() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let first = full(&repo);
    write(dir.path(), "src/util.rs", "pub fn helper() {}\npub fn second() {}\n");
    let (through_reuse, _) = incremental::update(&repo, Some(&first)).unwrap();
    let through_parsing = full(&repo);

    for (reused, parsed) in through_reuse.files.iter().zip(&through_parsing.files) {
        assert_eq!(reused.path, parsed.path);
        assert_eq!(reused.summary, parsed.summary, "{}: the cached summary differs from a fresh one", reused.path);
    }
}
