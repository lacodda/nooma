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
        vec!["repo", "status", "--help"],
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
    for command in ["index", "symbols", "deps", "status"] {
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

/// Reading a stale index and saying nothing would answer with the previous
/// commit's truth. It has to be a refusal, and one that says what to do.
#[test]
fn a_stale_index_is_refused_rather_than_read() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    run(&["repo", "index", "--store", store_path, repo_path]);
    std::fs::write(repo.path().join("src/extra.rs"), "pub fn added() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-m", "more"]);

    let output = run(&["repo", "symbols", "--store", store_path, repo_path]);
    assert!(!output.status.success(), "a stale index must not answer as if it were current");
    let complaint = stderr(&output);
    assert!(complaint.contains("nooma repo index"), "say what to do: {complaint}");
}

#[test]
fn indexing_twice_says_it_did_not_have_to() {
    let (repo, store) = fixture();
    let repo_path = repo.path().to_str().unwrap();
    let store_path = store.path().to_str().unwrap();

    let first = run(&["repo", "index", "--json", "--store", store_path, repo_path]);
    let first: serde_json::Value = serde_json::from_str(stdout(&first).trim()).unwrap();
    assert_eq!(first["rebuilt"], true);

    let second = run(&["repo", "index", "--json", "--store", store_path, repo_path]);
    let second: serde_json::Value = serde_json::from_str(stdout(&second).trim()).unwrap();
    assert_eq!(second["rebuilt"], false, "the same commit needs no second parse");
    assert_eq!(second["symbols"], first["symbols"]);

    let forced = run(&["repo", "index", "--json", "--force", "--store", store_path, repo_path]);
    let forced: serde_json::Value = serde_json::from_str(stdout(&forced).trim()).unwrap();
    assert_eq!(forced["rebuilt"], true, "--force means do it anyway");
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

    for command in ["index", "status", "symbols", "deps"] {
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
