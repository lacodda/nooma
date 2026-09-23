//! That the commands can actually be run, and that `--json` is machine-readable.
//!
//! A clap definition can compile, print a help page and still refuse every
//! invocation: two arguments sharing a value name parse as one argument given
//! twice. Nothing catches that but running the binary, which is what these
//! tests do. `debug_assert` catches some of it at startup; the rest needs a
//! real command line.

use std::path::Path;
use std::process::{Command, Output};

/// The binary under test, as cargo built it for this run.
fn nooma() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nooma"))
}

fn run(args: &[&str]) -> Output {
    nooma().args(args).output().expect("the binary did not start")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The one JSON document a `--json` command prints, or a message naming what
/// it printed instead.
fn json_of(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("expected one JSON document ({e}); stdout: {text}; stderr: {}", stderr(output)))
}

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

/// A repository and a store directory to index it into.
fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "--initial-branch=main"]);
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    std::fs::write(
        repo.path().join("src/ledger.rs"),
        "pub struct Ledger {}\nimpl Ledger {\n    pub fn post() {}\n}\n",
    )
    .unwrap();
    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "fixture"]);
    (repo, store)
}

/// Every argument definition is checked at startup under `debug_assert`; a
/// clash that clap will not tolerate makes the binary panic before it parses.
#[test]
fn the_command_tree_is_well_formed() {
    for args in [
        vec!["--help"],
        vec!["repo", "--help"],
        vec!["repo", "index", "--help"],
        vec!["repo", "symbols", "--help"],
        vec!["repo", "deps", "--help"],
        vec!["repo", "summary", "--help"],
        vec!["repo", "history", "--help"],
        vec!["repo", "status", "--help"],
        vec!["find", "--help"],
        vec!["index", "--help"],
        vec!["source", "--help"],
        vec!["source", "add", "--help"],
        vec!["source", "remove", "--help"],
        vec!["source", "list", "--help"],
    ] {
        let output = run(&args);
        assert!(output.status.success(), "nooma {args:?}: {}", stderr(&output));
    }
}

/// Every flag has to describe itself on the short help page.
///
/// clap reads a doc comment's body as an argument's long help, and one long
/// help anywhere in a command strips the short descriptions off every flag in
/// it: `--help` then lists bare names, and the reason for a flag lives only in
/// `help <command>`, which nobody types. One overlong doc comment on one
/// argument did this to the whole `symbols` page.
#[test]
fn every_flag_describes_itself_on_the_help_page() {
    for command in ["index", "symbols", "deps", "summary", "history", "status"] {
        let output = run(&["repo", command, "--help"]);
        assert!(output.status.success(), "{command}: {}", stderr(&output));
        let help = stdout(&output);
        for line in help.lines().filter(|l| l.trim_start().starts_with("--")) {
            // A described flag is "      --json    What it does"; a stripped
            // one is "      --json" and nothing else.
            let described = line.split_whitespace().count() > 2;
            assert!(described, "nooma repo {command} --help has a bare flag:\n{help}");
        }
    }
    for args in [vec!["find"], vec!["index"], vec!["source", "add"], vec!["source", "list"]] {
        let mut with_help = args.clone();
        with_help.push("--help");
        let help = stdout(&run(&with_help));
        for line in help.lines().filter(|l| l.trim_start().starts_with("--")) {
            assert!(line.split_whitespace().count() > 2, "nooma {args:?} --help has a bare flag:\n{help}");
        }
    }
}

/// The defect this file was written for: `--path` as a filter clashed with the
/// positional `PATH` for the repository, and clap answered every invocation
/// with "the argument '[PATH]' cannot be used multiple times". The help page
/// listed both quite happily.
#[test]
fn a_filter_can_be_combined_with_the_repository_it_filters() {
    let (repo, store) = fixture();
    let repo = repo.path().to_str().unwrap();
    let store = store.path().to_str().unwrap();

    assert!(run(&["repo", "index", "--store", store, repo]).status.success());

    let output = run(&["repo", "symbols", "--store", store, "--under", "src/", repo]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Ledger"), "{}", stdout(&output));

    let output = run(&["repo", "symbols", "--store", store, "--kind", "function", repo]);
    assert!(output.status.success(), "{}", stderr(&output));
    let listed = stdout(&output);
    assert!(listed.contains("post"), "{listed}");
    assert!(!listed.contains("struct"), "a kind filter must exclude the rest: {listed}");

    let output = run(&["repo", "symbols", "--store", store, "--name", "ledg", repo]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Ledger"), "the name filter ignores case");
}

/// `--json` has to be parseable on its own, with every human word on stderr —
/// otherwise redirecting stdout to a file yields a file that does not parse.
#[test]
fn json_output_is_json_and_nothing_else() {
    let (repo, store) = fixture();
    let repo = repo.path().to_str().unwrap();
    let store = store.path().to_str().unwrap();

    for args in [
        vec!["repo", "index", "--json", "--store", store, repo],
        vec!["repo", "status", "--json", "--store", store, repo],
        vec!["repo", "symbols", "--json", "--store", store, repo],
        vec!["repo", "deps", "--json", "--store", store, repo],
    ] {
        let output = run(&args);
        assert!(output.status.success(), "nooma {args:?}: {}", stderr(&output));
        let text = stdout(&output);
        serde_json::from_str::<serde_json::Value>(text.trim()).unwrap_or_else(|e| panic!("nooma {args:?} did not print JSON ({e}): {text}"));
        assert_eq!(text.trim().lines().count(), 1, "one document, one line: {text}");
    }
}

#[test]
fn status_reports_what_it_knows() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    let before = run(&["repo", "status", "--json", "--store", store_path, repo_path]);
    let before: serde_json::Value = serde_json::from_str(stdout(&before).trim()).unwrap();
    assert_eq!(before["indexed"], false);
    assert_eq!(before["current"], false);

    run(&["repo", "index", "--store", store_path, repo_path]);

    let after = run(&["repo", "status", "--json", "--store", store_path, repo_path]);
    let after: serde_json::Value = serde_json::from_str(stdout(&after).trim()).unwrap();
    assert_eq!(after["indexed"], true);
    assert_eq!(after["current"], true);
    assert_eq!(after["commit"], after["indexed_commit"]);
    assert_eq!(after["symbols"], 3, "Ledger, post, helper");

    // A new commit makes the stored index stale, and `status` is the command
    // that says so rather than answering with the previous commit's truth.
    std::fs::write(repo.path().join("src/extra.rs"), "pub fn added() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "more"]);

    let stale = run(&["repo", "status", "--json", "--store", store_path, repo_path]);
    let stale: serde_json::Value = serde_json::from_str(stdout(&stale).trim()).unwrap();
    assert_eq!(stale["indexed"], true);
    assert_eq!(stale["current"], false, "a new commit makes the index stale");
    assert_ne!(stale["commit"], stale["indexed_commit"]);
}

/// A reading command brings the index up to date rather than refusing.
///
/// v0.1.0 refused, because a full reparse was the only way to fix a stale
/// index and doing that behind a `symbols` call would have been a surprise.
/// Reparsing only what changed is cheap enough that the refusal became the
/// surprise instead: a caller asking what is in a repository wants the answer,
/// not an errand.
#[test]
fn a_reading_command_brings_the_index_up_to_date() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    run(&["repo", "index", "--store", store_path, repo_path]);
    std::fs::write(repo.path().join("src/extra.rs"), "pub fn added_later() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "more"]);

    let output = run(&["repo", "symbols", "--store", store_path, "--name", "added_later", repo_path]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("added_later"), "the new symbol is found: {}", stdout(&output));
    // The work is reported, not done in silence - and on stderr, so `--json`
    // still redirects to a file that parses.
    assert!(stderr(&output).contains("parsed"), "say what was reparsed: {}", stderr(&output));
}

/// Work done on the caller's behalf can be declined.
#[test]
fn no_refresh_answers_from_what_is_stored() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    run(&["repo", "index", "--store", store_path, repo_path]);
    std::fs::write(repo.path().join("src/extra.rs"), "pub fn added_later() {}\n").unwrap();

    let output = run(&["repo", "symbols", "--no-refresh", "--store", store_path, "--name", "added_later", repo_path]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !stdout(&output).contains("added_later"),
        "the stored index predates the edit: {}",
        stdout(&output)
    );
}

#[test]
fn a_repository_that_was_never_indexed_says_so() {
    let (repo, store) = fixture();
    let output = run(&[
        "repo",
        "symbols",
        "--no-refresh",
        "--store",
        store.path().to_str().unwrap(),
        repo.path().to_str().unwrap(),
    ]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("nooma repo index"), "say what to do: {}", stderr(&output));
}

/// Only what changed is parsed, and the counts say so.
#[test]
fn a_second_pass_reuses_what_did_not_change() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    let first = json_of(&run(&["repo", "index", "--json", "--store", store_path, repo_path]));
    assert_eq!(first["added"], 2, "both files are new to an empty store");
    assert_eq!(first["unchanged"], 0);

    let second = json_of(&run(&["repo", "index", "--json", "--store", store_path, repo_path]));
    assert_eq!(second["added"], 0);
    assert_eq!(second["changed"], 0);
    assert_eq!(second["unchanged"], 2, "nothing changed, so nothing is parsed again");
    assert_eq!(second["symbols"], first["symbols"], "reuse must give the same answer as parsing");

    // One file edited: one file parsed, the other carried across.
    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\npub fn second() {}\n").unwrap();
    let third = json_of(&run(&["repo", "index", "--json", "--store", store_path, repo_path]));
    assert_eq!(third["changed"], 1);
    assert_eq!(third["unchanged"], 1);
    assert_eq!(third["dirty"], true, "the tree no longer matches the commit");

    // `--force` parses everything regardless of what the hashes say.
    let forced = json_of(&run(&["repo", "index", "--json", "--force", "--store", store_path, repo_path]));
    assert_eq!(forced["unchanged"], 0, "--force means parse it all");
    assert_eq!(forced["added"], 2);
    assert_eq!(forced["symbols"], third["symbols"], "forcing must not change the answer");
}

/// A dirty index is not current, and no command makes it current: the tree
/// has edits and will keep having them until they are committed. Telling the
/// caller to reindex there prescribes the command they just ran.
#[test]
fn status_does_not_prescribe_a_command_that_would_change_nothing() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\npub fn second() {}\n").unwrap();
    run(&["repo", "index", "--store", store_path, repo_path]);

    let output = run(&["repo", "status", "--store", store_path, repo_path]);
    assert!(output.status.success(), "{}", stderr(&output));
    let said = stdout(&output);
    assert!(said.contains("uncommitted"), "say why it is not current: {said}");
    assert!(
        !said.contains("nooma repo index"),
        "the index was just built and rebuilding changes nothing: {said}"
    );

    // Still honestly not current, for a caller reading the machine answer.
    let status = json_of(&run(&["repo", "status", "--json", "--store", store_path, repo_path]));
    assert_eq!(status["current"], false);
    assert_eq!(status["indexed_dirty"], true);
}

/// Committing changes no file's bytes and still changes what the index
/// describes. Skipping the write when no file changed left the store claiming
/// the previous revision, and `status` then reported a tree as dirty
/// immediately after it had been committed - telling the caller to run the
/// command it had just run.
#[test]
fn committing_makes_the_index_clean_again() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    run(&["repo", "index", "--store", store_path, repo_path]);
    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\npub fn second() {}\n").unwrap();

    let edited = json_of(&run(&["repo", "index", "--json", "--store", store_path, repo_path]));
    assert_eq!(edited["dirty"], true);

    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "the edit"]);

    let committed = json_of(&run(&["repo", "index", "--json", "--store", store_path, repo_path]));
    assert_eq!(committed["unchanged"], 2, "the bytes on disk did not move");
    assert_eq!(committed["dirty"], false, "but the tree now matches the commit");

    let status = json_of(&run(&["repo", "status", "--json", "--store", store_path, repo_path]));
    assert_eq!(status["current"], true, "status must agree with the index just written");
    assert_eq!(status["indexed_dirty"], false);
    assert_eq!(status["commit"], status["indexed_commit"]);
}

/// The `--json` shape is a contract another product reads, and the reference
/// page is where it is written down. Prose drifts from code silently; this
/// compares the two, so a renamed field is a failed build rather than a
/// caller's surprise.
#[test]
fn the_reference_page_documents_the_fields_that_are_printed() {
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/content/docs/reference/repo.md");
    let page = std::fs::read_to_string(&reference).unwrap_or_else(|e| panic!("{}: {e}", reference.display()));

    let (repo, store) = fixture();
    let repo = repo.path().to_str().unwrap();
    let store = store.path().to_str().unwrap();
    run(&["repo", "index", "--store", store, repo]);

    for command in ["index", "status", "symbols", "deps", "summary", "history"] {
        let output = run(&["repo", command, "--json", "--store", store, repo]);
        let printed: serde_json::Value = serde_json::from_str(stdout(&output).trim()).unwrap();
        let object = printed.as_object().expect("every --json prints one object");

        for field in object.keys() {
            assert!(
                page.contains(&format!("`{field}`")),
                "`nooma repo {command} --json` prints `{field}`, which the reference page \
                 does not mention"
            );
        }
        // And one level down, where the rows a caller iterates over live.
        for value in object.values() {
            let Some(row) = value.as_array().and_then(|a| a.first()).and_then(|v| v.as_object()) else {
                continue;
            };
            for field in row.keys() {
                assert!(
                    page.contains(&format!("`{field}`")),
                    "a row of `nooma repo {command} --json` carries `{field}`, which the \
                     reference page does not mention"
                );
            }
        }
    }
}

#[test]
fn a_directory_that_is_not_a_repository_fails_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(&["repo", "index", dir.path().to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("not inside a git repository"),
        "say what is wrong: {}",
        stderr(&output)
    );
}

#[test]
fn an_unknown_kind_lists_the_known_ones() {
    let output = run(&["repo", "symbols", "--kind", "macro"]);
    assert!(!output.status.success());
    let complaint = stderr(&output);
    assert!(complaint.contains("function"), "name the alternatives: {complaint}");
}

/// The contract the stage was asked for: one module, one line a packet can
/// carry. The header is the part that says what the module is *about*, which
/// is what a list of symbol names cannot say at any length.
#[test]
fn summary_gives_a_module_its_header_and_its_public_surface() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(
        repo.path().join("src/ledger.rs"),
        "//! Keeps entries in balance.\n\n/// Post an entry.\npub fn post() {}\n\nfn internal() {}\n",
    )
    .unwrap();

    let printed = json_of(&run(&["repo", "summary", "--json", "--store", store_path, repo_path]));
    let modules = printed["modules"].as_array().expect("modules is a list");
    let ledger = modules
        .iter()
        .find(|m| m["path"] == "src/ledger.rs")
        .unwrap_or_else(|| panic!("no summary for src/ledger.rs: {printed}"));

    assert_eq!(ledger["header"], "Keeps entries in balance.");
    assert_eq!(ledger["public"], 1, "one public declaration, not two");

    let entries = ledger["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "a private function is off the surface by default");
    assert_eq!(entries[0]["name"], "post");
    assert_eq!(entries[0]["signature"], "pub fn post()");
    assert_eq!(entries[0]["doc"], "Post an entry.");
}

/// `--all` is for the caller reading a module rather than advertising it.
#[test]
fn summary_all_includes_what_the_language_keeps_private() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(repo.path().join("src/ledger.rs"), "pub fn post() {}\nfn internal() {}\n").unwrap();

    let printed = json_of(&run(&["repo", "summary", "--json", "--all", "--store", store_path, repo_path]));
    let ledger = printed["modules"].as_array().unwrap().iter().find(|m| m["path"] == "src/ledger.rs").unwrap();
    let names: Vec<&str> = ledger["entries"].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect();
    assert_eq!(names, vec!["post", "internal"]);
}

#[test]
fn history_finds_a_commit_by_what_its_message_says() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(repo.path().join("src/env.rs"), "pub fn path() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "fix: correct the PATH handling"]);

    let printed = json_of(&run(&[
        "repo",
        "history",
        "--json",
        "--matching",
        "path handling",
        "--store",
        store_path,
        repo_path,
    ]));
    let commits = printed["commits"].as_array().expect("commits is a list");
    assert_eq!(commits.len(), 1, "one commit matches: {printed}");
    assert_eq!(commits[0]["summary"], "fix: correct the PATH handling");
    assert_eq!(commits[0]["paths"].as_array().unwrap().len(), 1);
}

#[test]
fn history_finds_the_commits_that_touched_a_path() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\npub fn more() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "feat: extend the helper"]);

    let printed = json_of(&run(&[
        "repo",
        "history",
        "--json",
        "--touching",
        "src/util.rs",
        "--store",
        store_path,
        repo_path,
    ]));
    let summaries: Vec<&str> = printed["commits"].as_array().unwrap().iter().filter_map(|c| c["summary"].as_str()).collect();
    assert_eq!(summaries, vec!["feat: extend the helper", "fixture"]);
}

/// Naming both narrows by both. A caller who says two things means both, and
/// an implementation that let the second filter replace the first would answer
/// a question nobody asked.
#[test]
fn history_filters_combine_rather_than_replace_each_other() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    std::fs::write(repo.path().join("src/util.rs"), "pub fn helper() {}\npub fn more() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "feat: extend the helper"]);

    // `fixture` also touched src/util.rs, and `feat` also matches only the
    // newer commit: each filter alone leaves more than their intersection.
    let printed = json_of(&run(&[
        "repo",
        "history",
        "--json",
        "--matching",
        "extend",
        "--touching",
        "src/util.rs",
        "--store",
        store_path,
        repo_path,
    ]));
    let commits = printed["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1, "both filters apply: {printed}");
    assert_eq!(commits[0]["summary"], "feat: extend the helper");
}

/// Asking twice must not read twice: the second pass has nothing new, and the
/// stored history answers it whole.
#[test]
fn a_second_history_pass_reads_nothing_and_still_answers() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    let first = json_of(&run(&["repo", "history", "--json", "--store", store_path, repo_path]));
    let second_output = run(&["repo", "history", "--json", "--store", store_path, repo_path]);
    let second = json_of(&second_output);

    assert_eq!(first["commits"], second["commits"], "the answer must not change");
    assert!(!stderr(&second_output).contains("read 1"), "nothing new to read: {}", stderr(&second_output));
}

/// The promise of ADR 0004, checked rather than asserted in prose.
///
/// A default build must not merely decline to use the CLI — it must not be
/// able to. `--prose` is defined behind the feature, so in a build without it
/// clap refuses the argument outright. "Unknown flag" and "flag that quietly
/// did nothing" look identical to whoever forgot the feature, and only one of
/// them is honest.
#[cfg(not(feature = "prose"))]
#[test]
fn a_default_build_has_no_prose_flag_to_pass() {
    let (repo, store) = fixture();
    let output = run(&[
        "repo",
        "summary",
        "--prose",
        "--store",
        store.path().to_str().unwrap(),
        repo.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success(), "a build without the feature must refuse --prose outright");
    assert!(
        stderr(&output).contains("unexpected argument"),
        "clap should refuse it as unknown, not accept and ignore it: {}",
        stderr(&output)
    );
}

/// A build *with* the feature still does nothing unless asked.
///
/// The feature decides whether the code exists; the flag decides whether it
/// runs. Without `--prose` no process is started and nothing is spent, which
/// is what makes the summary command safe to run on a machine that happens to
/// have Claude Code installed.
#[cfg(feature = "prose")]
#[test]
fn the_feature_alone_does_not_start_anything() {
    let (repo, store) = fixture();
    let printed = json_of(&run(&[
        "repo",
        "summary",
        "--json",
        "--store",
        store.path().to_str().unwrap(),
        repo.path().to_str().unwrap(),
    ]));
    for module in printed["modules"].as_array().unwrap() {
        assert!(
            module.get("prose").is_none(),
            "a summary without --prose must carry no generated text: {module}"
        );
    }
    // And nothing was written where prose would be cached.
    assert!(!store.path().join("prose.json").exists(), "no call was asked for, so no cache should exist");
}
