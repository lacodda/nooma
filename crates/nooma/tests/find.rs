//! `nooma source`, `nooma index`, `nooma find` and `nooma similar`, run as a
//! script would run them, without a model.
//!
//! The corpus is written here, synthetic, in English and Russian. The library
//! goes into a temporary `--store`, never the user's data directory, and the
//! commands that would load a model are pointed at an empty `--models`
//! folder: these tests are about the exact half and about what the half by
//! meaning says when it cannot answer. The real model is run in `model.rs`.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn nooma(store: &Path, models: &Path, args: &[&str]) -> Output {
    nooma_with_input(store, models, args, None)
}

fn nooma_with_input(store: &Path, models: &Path, args: &[&str], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nooma"));
    command.args(args).arg("--store").arg(store);
    if matches!(args.first(), Some(&"index" | &"find" | &"similar")) {
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

fn json_of(output: &Output) -> serde_json::Value {
    let text = stdout(output);
    serde_json::from_str(text.trim()).unwrap_or_else(|e| {
        panic!(
            "expected one JSON document ({e}); stdout: {text}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

struct Fixture {
    notes: tempfile::TempDir,
    store: tempfile::TempDir,
    /// Empty: no model is on this machine, as far as these tests go.
    models: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(
            notes.path().join("receipts.md"),
            "# Appliance receipts\n\n## March\n\nFiled the warranty claim for the espresso machine. Invoice INV-2025-0114.\n",
        )
        .unwrap();
        std::fs::write(notes.path().join("договоры.md"), "# Договоры\n\nДоговор поставки продлевается автоматически.\n").unwrap();
        Self {
            notes,
            store: tempfile::tempdir().unwrap(),
            models: tempfile::tempdir().unwrap(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        nooma(self.store.path(), self.models.path(), args)
    }

    fn run_with_input(&self, args: &[&str], input: &str) -> Output {
        nooma_with_input(self.store.path(), self.models.path(), args, Some(input))
    }

    fn added(self) -> Self {
        let output = self.run(&["source", "add", self.notes.path().to_str().unwrap()]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        self
    }
}

#[test]
fn find_refreshes_first_and_prints_json() {
    let fixture = Fixture::new().added();
    let output = fixture.run(&["find", "warranties", "--json"]);
    assert!(output.status.success());
    let json = json_of(&output);
    assert_eq!(json["refreshed"], true);
    assert!(json["indexed_at"].is_i64());
    assert_eq!(json["all_words"], true);
    let hits = json["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["title"], "Appliance receipts");
    assert_eq!(hits[0]["words"]["rank"], 1);
    assert_eq!(hits[0]["words"]["share"], 1.0);
    assert_eq!(hits[0]["meaning"], serde_json::Value::Null, "no model, so the words alone");
    assert!(hits[0]["modified"].is_i64());
    assert_eq!(hits[0]["headings"], serde_json::json!(["Appliance receipts", "March"]));
    let fragment = hits[0]["fragment"].as_str().unwrap();
    let [start, end] = [&hits[0]["highlights"][0][0], &hits[0]["highlights"][0][1]].map(|v| v.as_u64().unwrap() as usize);
    assert_eq!(&fragment[start..end], "warranty");
}

#[test]
fn find_stems_russian_on_the_command_line() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["find", "договоров", "--json"]));
    assert_eq!(json["hits"][0]["title"], "Договоры");
}

#[test]
fn no_refresh_answers_from_what_is_stored() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["find", "espresso", "--no-refresh", "--json"]));
    assert_eq!(json["refreshed"], false);
    assert!(json["hits"].as_array().unwrap().is_empty(), "nothing was indexed yet");
    assert!(fixture.run(&["index"]).status.success());
    let json = json_of(&fixture.run(&["find", "espresso", "--no-refresh", "--json"]));
    assert_eq!(json["hits"].as_array().unwrap().len(), 1);
}

#[test]
fn index_reports_what_it_did() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["index", "--json"]));
    assert_eq!(json["documents"], 2);
    assert_eq!(json["indexed"], 2);
    let again = json_of(&fixture.run(&["index", "--json"]));
    assert_eq!(again["indexed"], 0);
}

/// Without the model, everything exact works as before, and each command
/// says how to get the other half rather than leaving it out in silence.
#[test]
fn without_the_model_find_answers_exactly_and_says_how_to_get_meaning() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["find", "warranty", "--json"]));
    assert_eq!(json["hits"].as_array().unwrap().len(), 1);
    assert_eq!(json["meaning"]["state"], "no-model");
    assert_eq!(json["meaning"]["model"], "multilingual-e5-small");

    let report = fixture.run(&["find", "warranty"]);
    assert!(report.status.success());
    let text = stdout(&report);
    assert!(text.starts_with("Appliance receipts · March (words)"), "{text}");
    assert!(text.contains("nooma model fetch"), "{text}");
    assert!(!text.contains("meaning 0."), "nothing by meaning without a model: {text}");
}

/// When no document holds every word, the list is the documents holding
/// some, and both the report and the JSON say so rather than passing them
/// off as matches.
#[test]
fn find_says_when_its_hits_hold_only_some_of_the_words() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["find", "warranty zeppelin", "--json"]));
    assert_eq!(json["all_words"], false);
    assert_eq!(json["hits"][0]["words"]["share"], 0.5);
    let text = stdout(&fixture.run(&["find", "warranty zeppelin"]));
    assert!(text.starts_with("no document holds every word of \"warranty zeppelin\""), "{text}");
    assert!(text.contains("(some words)"), "{text}");
}

#[test]
fn without_the_model_index_reads_the_words_and_says_how_to_get_meaning() {
    let fixture = Fixture::new().added();
    let json = json_of(&fixture.run(&["index", "--json"]));
    assert_eq!(json["documents"], 2);
    assert_eq!(json["vectors"], serde_json::Value::Null);
    let again = fixture.run(&["index"]);
    assert!(stdout(&again).contains("nooma model fetch"), "{}", stdout(&again));
}

/// A passage has only the half by meaning to answer it, so without the model
/// there is no answer - and the command fails saying why.
#[test]
fn similar_without_the_model_fails_saying_what_is_missing() {
    let fixture = Fixture::new().added();
    let example = fixture.notes.path().join("receipts.md");
    let output = fixture.run(&["similar", example.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nooma model fetch"));
}

#[test]
fn similar_refuses_an_empty_passage() {
    let fixture = Fixture::new().added();
    let output = fixture.run_with_input(&["similar", "-"], "  \n\t\n");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("passage is empty"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn index_without_sources_says_what_to_do() {
    let fixture = Fixture::new();
    let output = fixture.run(&["index"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nooma source add"));
}

#[test]
fn source_list_and_remove() {
    let fixture = Fixture::new().added();
    let listed = json_of(&fixture.run(&["source", "list", "--json"]));
    assert_eq!(listed["sources"].as_array().unwrap().len(), 1);
    assert_eq!(listed["indexed_at"], serde_json::Value::Null);

    let again = fixture.run(&["source", "add", fixture.notes.path().to_str().unwrap()]);
    assert!(!again.status.success(), "the same folder twice overlaps itself");

    assert!(fixture.run(&["source", "remove", fixture.notes.path().to_str().unwrap()]).status.success());
    let listed = json_of(&fixture.run(&["source", "list", "--json"]));
    assert!(listed["sources"].as_array().unwrap().is_empty());
}

/// Every field a `--json` command prints is named on the reference page, so
/// a new field cannot ship undocumented.
#[test]
fn the_reference_page_documents_the_fields_that_are_printed() {
    let reference = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src/content/docs/reference/find.md");
    let page = std::fs::read_to_string(&reference).unwrap_or_else(|e| panic!("{}: {e}", reference.display()));
    let fixture = Fixture::new().added();
    for args in [vec!["index", "--json"], vec!["source", "list", "--json"], vec!["find", "warranty", "--json"]] {
        let printed = json_of(&fixture.run(&args));
        assert!(printed.is_object(), "every --json prints one object");
        let mut fields = Vec::new();
        keys(&printed, &mut fields);
        for field in fields {
            assert!(
                page.contains(&format!("`{field}`")),
                "`nooma {args:?}` prints `{field}`, which the reference page does not mention"
            );
        }
    }
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
