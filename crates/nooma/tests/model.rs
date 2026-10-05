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
/// folder into both halves, `find` answers with both lists, and `similar`
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
    assert!(found["hits"].as_array().unwrap().is_empty(), "no note holds these words");
    assert_eq!(found["meaning"]["state"], "ready");
    assert_eq!(found["meaning"]["behind"], 0);
    let first = &found["meaning"]["hits"][0];
    assert!(first["path"].as_str().unwrap().ends_with("kettle.md"), "{found}");
    let score = first["score"].as_f64().unwrap();
    assert!(score > 0.5 && score <= 1.0, "a cosine: {score}");

    let report = stdout(&nooma(&["find", "how do I get rid of limescale"], &models, store));
    assert!(report.contains("by meaning · multilingual-e5-small"), "{report}");

    let kettle = library.notes.path().join("kettle.md");
    let similar = json_of(&nooma(&["similar", kettle.to_str().unwrap(), "--json"], &models, store));
    let paths: Vec<&str> = similar["hits"].as_array().unwrap().iter().map(|hit| hit["path"].as_str().unwrap()).collect();
    assert_eq!(paths.len(), 1, "the example's own file is left out: {paths:?}");
    assert!(paths[0].ends_with("чай.md"));

    let passage = "Limescale builds up inside the kettle, and citric acid dissolves it.";
    let piped = json_of(&nooma_with_input(&["similar", "-", "--json"], &models, store, Some(passage)));
    assert_eq!(piped["example"], "-");
    assert!(piped["hits"][0]["path"].as_str().unwrap().ends_with("kettle.md"), "{piped}");

    let page = reference("find.md");
    for printed in [&indexed, &found, &similar] {
        let mut fields = Vec::new();
        keys(printed, &mut fields);
        for field in fields {
            assert!(page.contains(&format!("`{field}`")), "find.md does not mention `{field}`");
        }
    }
}
