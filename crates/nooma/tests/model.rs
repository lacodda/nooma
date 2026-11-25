//! `nooma model` and `nooma eval`, run as a script would run them.
//!
//! Models go into a temporary `--models` folder and the library into a
//! temporary `--store`, never the user's. The one test that needs the real
//! model reads it from `NOOMA_MODELS` or nooma's own folder, and fails with
//! the command that fetches it when it is not there.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Run nooma on a temporary library; `--models` goes only to the commands
/// that load or fetch a model, as it would from a script.
fn nooma(args: &[&str], models: &Path, store: &Path) -> Output {
    nooma_with_input(args, models, store, None)
}

fn nooma_with_input(args: &[&str], models: &Path, store: &Path, input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nooma"));
    command.args(args).arg("--store").arg(store);
    if matches!(args.first(), Some(&"model" | &"eval" | &"index" | &"find" | &"similar")) {
        command.arg("--models").arg(models);
    }
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("the binary did not start");
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().expect("the binary did not finish")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json_of(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("expected one JSON document ({e}); stdout: {text}; stderr: {}", stderr(output)))
}

/// Every key of every object in a JSON document, however deep.
fn keys(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                out.push(key.clone());
                keys(value, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                keys(item, out);
            }
        }
        _ => {}
    }
}

fn reference(page: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/content/docs/reference").join(page);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_list_names_every_model_and_says_none_is_here() {
    let models = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let output = nooma(&["model", "list", "--json"], models.path(), store.path());
    assert!(output.status.success(), "{}", stderr(&output));
    let listed = json_of(&output);
    let entries = listed["models"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries.iter().filter(|m| m["default"] == true).count(), 1);
    assert!(entries.iter().all(|m| m["present"] == false), "an empty folder holds no model");
    assert!(entries.iter().all(|m| m["revision"].as_str().is_some_and(|r| r.len() == 40)));

    let page = reference("model.md");
    let mut printed = Vec::new();
    keys(&listed, &mut printed);
    for field in printed {
        assert!(
            page.contains(&format!("`{field}`")),
            "`model list --json` prints `{field}`, which the reference page does not mention"
        );
    }
}

#[test]
fn an_unknown_model_is_refused_with_the_names_there_are() {
    let models = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let output = nooma(&["model", "fetch", "gpt-embeddings"], models.path(), store.path());
    assert!(!output.status.success());
    assert!(stderr(&output).contains("multilingual-e5-small"), "{}", stderr(&output));
}

/// Removing a model removes what was computed with it: vectors from one
/// model mean nothing to another, and would only take up room.
#[test]
fn removing_a_model_removes_its_vectors_too() {
    let models = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(models.path().join("bge-m3/onnx")).unwrap();
    std::fs::write(models.path().join("bge-m3/config.json"), "{}").unwrap();
    std::fs::create_dir_all(store.path().join("vectors/bge-m3")).unwrap();
    std::fs::write(store.path().join("vectors/bge-m3/meta.json"), "{}").unwrap();
    std::fs::create_dir_all(store.path().join("vectors/multilingual-e5-small")).unwrap();

    let output = nooma(&["model", "remove", "bge-m3"], models.path(), store.path());
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("removed bge-m3 and its vectors"), "{}", stdout(&output));
    assert!(!models.path().join("bge-m3").exists());
    assert!(!store.path().join("vectors/bge-m3").exists());
    assert!(store.path().join("vectors/multilingual-e5-small").exists(), "another model's vectors stay");
}

struct Library {
    notes: tempfile::TempDir,
    store: tempfile::TempDir,
}

fn library() -> Library {
    let notes = tempfile::tempdir().unwrap();
    std::fs::write(
        notes.path().join("kettle.md"),
        "# Kettle\n\nDescale the kettle with citric acid once a month.\n",
    )
    .unwrap();
    std::fs::write(
        notes.path().join("чай.md"),
        "# Чай\n\nЗелёный чай заваривают водой около восьмидесяти градусов.\n",
    )
    .unwrap();
    let store = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let output = nooma(&["source", "add", notes.path().to_str().unwrap()], empty.path(), store.path());
    assert!(output.status.success(), "{}", stderr(&output));
    Library { notes, store }
}

fn write_set(dir: &Path, expect: &str) -> PathBuf {
    let path = dir.join("questions.json");
    let set = serde_json::json!({ "queries": [
        { "query": "how do I get rid of limescale", "expect": [expect], "group": "en→en" },
        { "query": "какой температуры вода для зелёного чая", "expect": ["чай.md"], "group": "ru→ru" },
    ]});
    std::fs::write(&path, serde_json::to_vec(&set).unwrap()).unwrap();
    path
}

#[test]
fn eval_names_the_fetch_for_a_model_that_is_not_here() {
    let library = library();
    let models = tempfile::tempdir().unwrap();
    let set = write_set(library.notes.path(), "kettle.md");
    let output = nooma(&["eval", set.to_str().unwrap()], models.path(), library.store.path());
    assert!(!output.status.success());
    assert!(stderr(&output).contains("nooma model fetch multilingual-e5-small"), "{}", stderr(&output));
}

#[test]
fn eval_refuses_a_set_that_expects_a_document_the_library_lacks() {
    let library = library();
    let set = write_set(library.notes.path(), "kettel.md");
    let output = nooma(&["eval", set.to_str().unwrap()], &real_models(), library.store.path());
    assert!(!output.status.success());
    assert!(stderr(&output).contains("kettel.md"), "{}", stderr(&output));
}

fn real_models() -> PathBuf {
    let dir = match std::env::var_os("NOOMA_MODELS") {
        Some(dir) => PathBuf::from(dir),
        None => nooma_core::model::default_models_dir().expect("a models folder"),
    };
    let spec = nooma_core::ModelSpec::default_model();
    assert!(
        spec.missing(&dir).is_empty(),
        "the model {} is not in {}. Fetch it once with `cargo run -p nooma -- model fetch`, \
         or point NOOMA_MODELS at a folder that has it. This test runs the real model and does not skip.",
        spec.id,
        dir.display()
    );
    dir
}

/// The whole path with the real model: vectors computed, both engines
/// measured, every printed field on the reference page - and the second run
/// computes nothing, because the vectors were kept.
#[test]
fn eval_measures_both_halves_and_keeps_the_vectors() {
    let library = library();
    let set = write_set(library.notes.path(), "kettle.md");
    let models = real_models();

    let first = json_of(&nooma(&["eval", set.to_str().unwrap(), "--json"], &models, library.store.path()));
    assert_eq!(first["queries"], 2);
    let engines = first["engines"].as_array().unwrap();
    assert_eq!(engines[0]["engine"], "fulltext");
    assert_eq!(engines[1]["engine"], "multilingual-e5-small");
    assert_eq!(engines[1]["overall"]["hit_at_1"], 1.0, "{}", engines[1]);
    // The graph beside exact search over the same vectors: what the
    // approximation costs, measured on every run.
    assert_eq!(engines[2]["engine"], "multilingual-e5-small exact");
    assert_eq!(engines[2]["overall"], engines[1]["overall"]);
    // And the one list `find` shows, words and meaning together.
    assert_eq!(engines[3]["engine"], "multilingual-e5-small hybrid");
    assert_eq!(engines[3]["overall"]["hit_at_1"], 1.0, "{}", engines[3]);
    assert_eq!(first["vectors"][0]["report"]["embedded"], 2);

    let page = reference("eval.md");
    let mut printed = Vec::new();
    keys(&first, &mut printed);
    // Group names are the query set's own, not fields.
    printed.retain(|key| !key.contains('→'));
    for field in printed {
        assert!(
            page.contains(&format!("`{field}`")),
            "`eval --json` prints `{field}`, which the reference page does not mention"
        );
    }

    let second = json_of(&nooma(&["eval", set.to_str().unwrap(), "--json"], &models, library.store.path()));
    assert_eq!(second["vectors"][0]["report"]["embedded"], 0, "the vectors were kept");
}

/// The whole path a person takes with the real model: `index` reads the
/// folder into both halves, `find` answers with one list from both, and `similar`
/// finds what a passage is like - leaving out the file it came from. Every
/// field printed is on the reference page.
#[test]
fn index_computes_the_vectors_and_find_and_similar_answer_by_meaning() {
    let library = library();
    let models = real_models();
    let store = library.store.path();

    let indexed = json_of(&nooma(&["index", "--json"], &models, store));
    assert_eq!(indexed["documents"], 2);
    assert_eq!(indexed["vectors"]["embedded"], 2);
    let again = json_of(&nooma(&["index", "--json"], &models, store));
    assert_eq!(again["vectors"]["embedded"], 0, "the vectors were kept");

    let found = json_of(&nooma(&["find", "how do I get rid of limescale", "--json"], &models, store));
    assert_eq!(found["meaning"]["state"], "ready");
    assert_eq!(found["meaning"]["behind"], 0);
    let first = &found["hits"][0];
    assert!(first["path"].as_str().unwrap().ends_with("kettle.md"), "{found}");
    assert_eq!(first["words"], serde_json::Value::Null, "no note holds these words: {found}");
    let cosine = first["meaning"]["score"].as_f64().unwrap();
    assert!(cosine > 0.5 && cosine <= 1.0, "a cosine: {cosine}");

    let report = stdout(&nooma(&["find", "how do I get rid of limescale"], &models, store));
    assert!(report.contains("(meaning 0."), "{report}");

    let kettle = library.notes.path().join("kettle.md");
    let similar = json_of(&nooma(&["similar", kettle.to_str().unwrap(), "--json"], &models, store));
    let paths: Vec<&str> = similar["hits"].as_array().unwrap().iter().map(|hit| hit["path"].as_str().unwrap()).collect();
    assert_eq!(paths.len(), 1, "the example's own file is left out: {paths:?}");
    assert!(paths[0].ends_with("чай.md"));

    let passage = "Limescale builds up inside the kettle, and citric acid dissolves it.";
    let piped = json_of(&nooma_with_input(&["similar", "-", "--json"], &models, store, Some(passage)));
    assert_eq!(piped["example"], "-");
    assert!(piped["hits"][0]["path"].as_str().unwrap().ends_with("kettle.md"), "{piped}");

    // A PDF is a passage too, its text read as the library reads it: the
    // handbook's first page says to descale with a citric solution.
    let handbook = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nooma-core/tests/pdf/handbook.pdf");
    let from_pdf = json_of(&nooma(&["similar", handbook.to_str().unwrap(), "--json"], &models, store));
    assert!(from_pdf["hits"][0]["path"].as_str().unwrap().ends_with("kettle.md"), "{from_pdf}");

    let page = reference("find.md");
    for printed in [&indexed, &found, &similar] {
        let mut fields = Vec::new();
        keys(printed, &mut fields);
        for field in fields {
            assert!(page.contains(&format!("`{field}`")), "find.md does not mention `{field}`");
        }
    }
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

/// Commits ranked by what their messages say, with the real model: the
/// question shares no word with the commit that answers it, and a filter by
/// words would find nothing. The messages are read once - a second question
/// reads none - and every printed field is on the reference page.
#[test]
fn history_ranks_commits_by_what_their_messages_mean() {
    let repo = tempfile::tempdir().unwrap();
    let store = tempfile::tempdir().unwrap();
    let models = real_models();
    git(repo.path(), &["init", "--initial-branch=main"]);
    for (file, message) in [
        ("ledger.rs", "feat: add the ledger\n\nEvery entry is posted twice, once to each side."),
        (
            "env.rs",
            "fix: correct the PATH handling on Windows\n\nThe registry wants REG_EXPAND_SZ and setx truncates the value.",
        ),
        ("guide.md", "docs: explain how to restore a backup after the disk fails"),
        ("theme.css", "style: darker borders on the settings dialog"),
    ] {
        std::fs::write(repo.path().join(file), message).unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-m", message]);
    }
    let repo_path = repo.path().to_str().unwrap();
    let models_path = models.to_str().unwrap();
    let ask = |question: &str, more: &[&str]| {
        let mut args = vec!["repo", "history", repo_path, "--meaning", question, "--models", models_path];
        args.extend(more);
        nooma(&args, &models, store.path())
    };

    let first_output = ask("the environment variable was cut short", &["--top", "2", "--json"]);
    assert!(first_output.status.success(), "{}", stderr(&first_output));
    let first = json_of(&first_output);
    let commits = first["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2, "the top two of four: {first}");
    assert_eq!(commits[0]["summary"], "fix: correct the PATH handling on Windows");
    assert_eq!(first["model"], "multilingual-e5-small");
    let scores: Vec<f64> = commits.iter().map(|commit| commit["score"].as_f64().unwrap()).collect();
    assert!(scores[0] > scores[1], "closest first: {scores:?}");
    assert_eq!(commits[0]["paths"], serde_json::json!(["env.rs"]), "the commits printed have their paths");
    assert!(stderr(&first_output).contains("computing 4 vectors"), "{}", stderr(&first_output));

    let page = reference("repo.md");
    let mut printed = Vec::new();
    keys(&first, &mut printed);
    for field in printed {
        assert!(
            page.contains(&format!("`{field}`")),
            "`repo history --meaning --json` prints `{field}`, which the reference page does not mention"
        );
    }

    // Another question, in another language: no message is read again, and
    // the answer is found across the languages.
    let second_output = ask("как восстановить данные из резервной копии", &["--json"]);
    let second = json_of(&second_output);
    assert_eq!(second["commits"][0]["summary"], "docs: explain how to restore a backup after the disk fails");
    assert_eq!(
        second["commits"].as_array().unwrap().len(),
        4,
        "fewer commits than the top asks for: all of them"
    );
    assert!(
        !stderr(&second_output).contains("computing"),
        "the vectors were kept: {}",
        stderr(&second_output)
    );

    // The filters narrow first, and the ranking orders what they left.
    let narrowed = json_of(&ask("the environment variable was cut short", &["--matching", "feat", "--json"]));
    let summaries: Vec<&str> = narrowed["commits"].as_array().unwrap().iter().filter_map(|c| c["summary"].as_str()).collect();
    assert_eq!(summaries, ["feat: add the ledger"]);

    // For a person: the short id, how close, the summary.
    let listed = stdout(&ask("the environment variable was cut short", &["--top", "1"]));
    let line = listed.lines().next().unwrap_or_default();
    assert!(line.ends_with("fix: correct the PATH handling on Windows"), "{listed}");
    assert!(line.split_whitespace().nth(1).is_some_and(|score| score.parse::<f32>().is_ok()), "{listed}");
}
