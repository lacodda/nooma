//! Reading a repository's commit messages as documents.
//!
//! The thing worth defending here is the incremental rule. A commit's id is
//! the hash of everything about it, so a commit already held never needs
//! re-reading — but "already held" decides only whether to *read* it, never
//! whether it belongs. What belongs is what HEAD reaches, and the two part
//! company under `--amend` and rebase, where the stored history keeps an
//! object the repository has replaced. So these tests check that a reused read
//! equals a full one in every case, including the ones where reuse would
//! otherwise resurrect a commit that is gone.
//!
//! Every fixture is built by the test; nothing here comes from a real
//! repository.

use std::path::Path;
use std::process::Command;

use nooma_core::{History, history, repo::Repo};

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

fn commit(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", message]);
}

/// A repository with three commits, each touching a different file.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "--initial-branch=main"]);

    write(root, "src/ledger.rs", "pub fn post() {}\n");
    commit(root, "feat: add the ledger");

    write(root, "src/env.rs", "pub fn path() {}\n");
    commit(
        root,
        "fix: correct the PATH handling on Windows\n\nThe registry wants REG_EXPAND_SZ and setx truncates.",
    );

    write(root, "docs/guide.md", "# Guide\n");
    commit(root, "docs: write the guide");

    dir
}

/// Read everything, the way a first pass does.
fn full(repo: &Repo) -> History {
    history::read(repo, None, 1000).unwrap().0
}

#[test]
fn a_first_read_takes_every_commit_newest_first() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let (read, update) = history::read(&repo, None, 1000).unwrap();
    assert_eq!(update.read, 3);
    assert_eq!(update.kept, 0);
    let summaries: Vec<&str> = read.commits.iter().map(|c| c.summary.as_str()).collect();
    assert_eq!(
        summaries,
        vec!["docs: write the guide", "fix: correct the PATH handling on Windows", "feat: add the ledger"]
    );
}

/// The whole point of the stage's second task: the reason for a change lives
/// in its message and nowhere else, so a question about the reason has to be
/// answerable from the history.
#[test]
fn a_commit_is_found_by_what_its_message_says() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let read = full(&repo);

    let found = read.matching("path handling");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].summary, "fix: correct the PATH handling on Windows");

    // The explanation is in the body, which is the half a summary-only search
    // would lose.
    let by_body = read.matching("REG_EXPAND_SZ");
    assert_eq!(by_body.len(), 1, "the body has to be searchable too");
}

#[test]
fn a_commit_records_the_paths_it_touched() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let read = full(&repo);

    let touching = read.touching("src/env.rs");
    assert_eq!(touching.len(), 1);
    assert_eq!(touching[0].summary, "fix: correct the PATH handling on Windows");

    // A directory names everything beneath it.
    assert_eq!(read.touching("src").len(), 2);
    assert_eq!(read.touching("docs").len(), 1);
}

/// A second read after one new commit reads exactly that one, and the result
/// equals a full read. Reading fewer commits is worth nothing if the history
/// that comes out differs from the history that reading them all would give.
#[test]
fn a_second_read_takes_only_what_is_new_and_agrees_with_a_full_read() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);

    write(dir.path(), "src/util.rs", "pub fn helper() {}\n");
    commit(dir.path(), "feat: add a helper");

    let repo = Repo::discover(dir.path()).unwrap();
    let (incremental, update) = history::read(&repo, Some(&first), 1000).unwrap();
    assert_eq!(update.read, 1, "only the new commit is read");
    assert_eq!(update.kept, 3, "the three already held are carried across");

    let complete = full(&repo);
    assert_eq!(incremental, complete, "the shortcut must give what doing the work gives");
}

/// Nothing new means nothing read — and the history still comes out whole,
/// rather than empty because the walk stopped immediately.
#[test]
fn a_read_with_nothing_new_keeps_everything() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);

    let (again, update) = history::read(&repo, Some(&first), 1000).unwrap();
    assert!(update.is_noop());
    assert_eq!(update.read, 0);
    assert_eq!(again.commits.len(), 3, "a noop pass must not empty the history");
    assert_eq!(again, first);
}

/// An amended commit is a different object with a different id, so it reads as
/// new rather than as a changed version of one held. What must not happen is
/// the old commit surviving beside it: the walk from HEAD no longer reaches
/// the original, so it is not part of this history any more.
#[test]
fn an_amended_commit_replaces_rather_than_joins_the_one_it_rewrote() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let first = full(&repo);
    let original = first.commits[0].id.clone();

    git(dir.path(), &["commit", "--amend", "-m", "docs: write the guide, better"]);

    let repo = Repo::discover(dir.path()).unwrap();
    let (rewritten, _) = history::read(&repo, Some(&first), 1000).unwrap();

    assert_eq!(rewritten.commits.len(), 3, "the rewrite replaced a commit, it did not add one");
    assert!(!rewritten.holds(&original), "the commit that was rewritten is no longer reachable");
    assert_eq!(rewritten.commits[0].summary, "docs: write the guide, better");
    assert_eq!(rewritten, full(&repo), "and the result still equals a full read");
}

/// The stored history belongs to a different line of commits — a fresh clone,
/// a reset, a branch switched underneath. Stitching the two together would
/// claim a sequence that never existed, so the walk that never reaches the
/// stored history keeps none of it.
#[test]
fn a_history_from_another_line_of_commits_is_not_stitched_on() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let mut foreign = History::empty(repo.root().to_path_buf(), "f".repeat(40));
    foreign.commits = vec![nooma_core::CommitDoc {
        id: "f".repeat(40),
        summary: "feat: something that never happened here".into(),
        body: None,
        author: "Someone Else".into(),
        time: 0,
        paths: vec!["src/ghost.rs".into()],
    }];

    let (read, update) = history::read(&repo, Some(&foreign), 1000).unwrap();
    assert_eq!(update.read, 3);
    assert_eq!(update.kept, 0, "none of a foreign history may be carried across");
    assert!(!read.holds(&"f".repeat(40)));
    assert_eq!(read, full(&repo));
}

/// The cap is what keeps a first `index` from appearing to hang on a
/// repository with a hundred thousand commits. It takes the newest, which is
/// what every question here is about.
#[test]
fn the_limit_takes_the_newest_commits() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();

    let (capped, update) = history::read(&repo, None, 2).unwrap();
    assert_eq!(update.read, 2);
    assert_eq!(capped.commits.len(), 2);
    assert_eq!(capped.commits[0].summary, "docs: write the guide");
    assert_eq!(capped.commits[1].summary, "fix: correct the PATH handling on Windows");
}

/// The root commit has no parent to compare against, so everything it
/// introduced is what it touched. Reporting nothing there would lose the
/// commit that created the project.
#[test]
fn the_first_commit_reports_what_it_introduced() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let read = full(&repo);

    let root = read.commits.last().unwrap();
    assert_eq!(root.summary, "feat: add the ledger");
    assert_eq!(root.paths, vec!["src/ledger.rs"]);
}

/// A commit that deletes a file touched it, and the path is only visible on
/// the parent's side. A comparison that walks the new tree alone never
/// mentions it.
#[test]
fn a_deletion_counts_as_touching_the_path() {
    let dir = fixture();
    std::fs::remove_file(dir.path().join("src/ledger.rs")).unwrap();
    commit(dir.path(), "refactor: drop the ledger");

    let repo = Repo::discover(dir.path()).unwrap();
    let read = full(&repo);

    let touching = read.touching("src/ledger.rs");
    assert_eq!(touching.len(), 2, "the commit that added it and the one that removed it");
    assert_eq!(touching[0].summary, "refactor: drop the ledger");
}

#[test]
fn a_history_survives_a_round_trip_through_the_store() {
    let dir = fixture();
    let store_dir = tempfile::tempdir().unwrap();
    let store = nooma_core::Store::open_at(store_dir.path()).unwrap();
    let repo = Repo::discover(dir.path()).unwrap();

    let read = full(&repo);
    history::save(&store, &read).unwrap();
    assert_eq!(history::load(&store, repo.root()).unwrap(), Some(read));
}

#[test]
fn a_history_never_read_is_not_an_error() {
    let store_dir = tempfile::tempdir().unwrap();
    let store = nooma_core::Store::open_at(store_dir.path()).unwrap();
    assert_eq!(history::load(&store, Path::new("/never/seen")).unwrap(), None);
}

/// The same guard the file index has, for the same reason: a stored document
/// written under another format must be refused rather than read under
/// today's meaning.
#[test]
fn a_foreign_history_format_is_refused_and_says_to_reindex() {
    let store_dir = tempfile::tempdir().unwrap();
    let store = nooma_core::Store::open_at(store_dir.path()).unwrap();
    let root = Path::new("/some/repo");

    let mut stale = History::empty(root.to_path_buf(), "a".repeat(40));
    stale.format_version = history::HISTORY_FORMAT_VERSION + 1;
    std::fs::write(history::path_for(&store, root), serde_json::to_vec(&stale).unwrap()).unwrap();

    let error = history::load(&store, root).unwrap_err();
    assert!(error.is_stale_index(), "expected a stale-index error, got: {error}");
}

/// The history is stored beside the index, not on top of it. One file holding
/// both would mean every commit read rewrote every symbol.
#[test]
fn a_history_and_an_index_do_not_share_a_file() {
    let store_dir = tempfile::tempdir().unwrap();
    let store = nooma_core::Store::open_at(store_dir.path()).unwrap();
    let root = Path::new("/some/repo");
    assert_ne!(history::path_for(&store, root), store.path_for(root));
}
