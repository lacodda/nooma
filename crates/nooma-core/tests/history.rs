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
fn a_commit_is_found_by_the_paths_it_touched() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let mut read = full(&repo);

    let touching = read.touching(&repo, "src/env.rs").unwrap();
    assert_eq!(touching.len(), 1);
    assert_eq!(touching[0].summary, "fix: correct the PATH handling on Windows");

    // A directory names everything beneath it.
    assert_eq!(read.touching(&repo, "src").unwrap().len(), 2);
    assert_eq!(read.touching(&repo, "docs").unwrap().len(), 1);
    // A prefix that is not a whole path component matches nothing: `src`
    // must not pull in `srcery/`.
    assert_eq!(read.touching(&repo, "sr").unwrap().len(), 0);
}

/// A pass over the history reads messages. The paths cost a comparison of
/// two trees per commit, a search of the messages never asks for them, and on
/// a first pass they were most of the wait - so nothing reads them until
/// something asks.
#[test]
fn a_read_takes_no_paths_until_they_are_asked_for() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let mut read = full(&repo);
    assert!(read.commits.iter().all(|commit| commit.paths.is_none()));

    // Asked for one commit, read for that one.
    let newest = read.commits[0].id.clone();
    assert_eq!(read.read_paths(&repo, |commit| commit.id == newest).unwrap(), 1);
    assert_eq!(read.commits[0].paths.as_deref(), Some(&["docs/guide.md".to_string()][..]));
    assert!(read.commits[1].paths.is_none());

    // Asked for all, the one already read is not read again.
    assert_eq!(read.read_paths(&repo, |_| true).unwrap(), 2);
    assert_eq!(read.read_paths(&repo, |_| true).unwrap(), 0);
}

/// Paths once read are a property of the commit, like its message: the next
/// pass carries them across, and reads none for the commit that is new.
#[test]
fn paths_once_read_are_kept_by_the_next_read() {
    let dir = fixture();
    let repo = Repo::discover(dir.path()).unwrap();
    let mut first = full(&repo);
    first.read_paths(&repo, |_| true).unwrap();

    write(dir.path(), "src/util.rs", "pub fn helper() {}\n");
    commit(dir.path(), "feat: add a helper");

    let repo = Repo::discover(dir.path()).unwrap();
    let (second, _) = history::read(&repo, Some(&first), 1000).unwrap();
    assert!(second.commits[0].paths.is_none(), "the new commit's were never asked for");
    assert!(second.commits[1..].iter().all(|commit| commit.paths.is_some()));
}

/// Git's own answer for a commit against its first parent.
fn git_paths(root: &Path, id: &str, has_parent: bool) -> Vec<String> {
    let mut args = vec!["-c", "core.quotepath=false", "diff-tree", "-r", "--no-renames", "--name-only", "--no-commit-id"];
    let parent = format!("{id}^1");
    if has_parent {
        args.extend([parent.as_str(), id]);
    } else {
        args.extend(["--root", id]);
    }
    let output = Command::new("git").args(&args).current_dir(root).output().expect("git is not on PATH");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    let mut paths: Vec<String> = String::from_utf8(output.stdout).unwrap().lines().map(str::to_string).collect();
    paths.sort();
    paths
}

/// The two trees are compared level by level, and a directory both sides
/// hold under one id is never opened. That is where the speed is, and where
/// a path could be dropped without a word - so every shape a change takes is
/// built here, and each commit's paths are held to what git itself reports.
#[test]
fn the_paths_of_a_commit_are_the_ones_git_reports() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "--initial-branch=main"]);
    // The mode is changed in the index alone, below. Where git reads modes
    // from the disk, the next `add` would see the file as it lies there and
    // take the change back in a commit that is about something else.
    git(root, &["config", "core.fileMode", "false"]);

    write(root, "README.md", "# Fixture\n");
    write(root, "src/a/one.rs", "fn one() {}\n");
    write(root, "src/a/deep/two.rs", "fn two() {}\n");
    write(root, "src/b/three.rs", "fn three() {}\n");
    write(root, "docs/заметка.md", "# Note\n");
    commit(root, "feat: the root, several levels deep");

    write(root, "src/a/deep/two.rs", "fn two() -> u8 { 2 }\n");
    commit(root, "fix: one file, three levels down");

    std::fs::remove_dir_all(root.join("src/b")).unwrap();
    commit(root, "refactor: a whole directory removed");

    std::fs::remove_file(root.join("src/a/one.rs")).unwrap();
    write(root, "src/a/one.rs/inner.rs", "fn inner() {}\n");
    commit(root, "refactor: a file becomes a directory");

    std::fs::remove_dir_all(root.join("src/a/deep")).unwrap();
    write(root, "src/a/deep", "now a file\n");
    commit(root, "refactor: a directory becomes a file");

    std::fs::rename(root.join("README.md"), root.join("docs/README.md")).unwrap();
    commit(root, "docs: a file moved between directories");

    git(root, &["update-index", "--chmod=+x", "docs/README.md"]);
    git(root, &["commit", "-m", "chore: only a mode changed"]);

    // A merge: the side branch changes one file, main another, and the
    // merge is compared with main alone.
    git(root, &["checkout", "-b", "side"]);
    write(root, "side.txt", "from the side\n");
    commit(root, "feat: on the side");
    git(root, &["checkout", "main"]);
    write(root, "main.txt", "from main\n");
    commit(root, "feat: on main");
    git(root, &["merge", "--no-ff", "side", "-m", "merge: the side"]);

    git(root, &["commit", "--allow-empty", "-m", "chore: nothing changed"]);

    let repo = Repo::discover(root).unwrap();
    let ids = repo.commit_ids(1000).unwrap();
    assert_eq!(ids.len(), 11);
    for (at, id) in ids.iter().enumerate() {
        let summary = repo.commit_doc(id).unwrap().summary;
        let has_parent = at + 1 < ids.len() && summary != "feat: the root, several levels deep";
        assert_eq!(repo.commit_paths(id).unwrap(), git_paths(root, id, has_parent), "{summary}");
    }

    let paths_of = |summary: &str| {
        let id = ids.iter().find(|id| repo.commit_doc(id).unwrap().summary == summary).unwrap();
        repo.commit_paths(id).unwrap()
    };
    // Not held to git alone: were both wrong alike, these would still say.
    assert_eq!(paths_of("fix: one file, three levels down"), ["src/a/deep/two.rs"]);
    assert_eq!(paths_of("refactor: a file becomes a directory"), ["src/a/one.rs", "src/a/one.rs/inner.rs"]);
    assert_eq!(paths_of("merge: the side"), ["side.txt"]);
    assert_eq!(paths_of("chore: only a mode changed"), ["docs/README.md"]);
    assert!(paths_of("chore: nothing changed").is_empty());
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
        paths: Some(vec!["src/ghost.rs".into()]),
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
    assert_eq!(repo.commit_paths(&root.id).unwrap(), vec!["src/ledger.rs"]);
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
    let mut read = full(&repo);

    let touching = read.touching(&repo, "src/ledger.rs").unwrap();
    assert_eq!(touching.len(), 2, "the commit that added it and the one that removed it");
    assert_eq!(touching[0].summary, "refactor: drop the ledger");
}

#[test]
fn a_history_survives_a_round_trip_through_the_store() {
    let dir = fixture();
    let store_dir = tempfile::tempdir().unwrap();
    let store = nooma_core::Store::open_at(store_dir.path()).unwrap();
    let repo = Repo::discover(dir.path()).unwrap();

    // Some paths read and some not: both are stored as they are.
    let mut read = full(&repo);
    let newest = read.commits[0].id.clone();
    read.read_paths(&repo, |commit| commit.id == newest).unwrap();
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
