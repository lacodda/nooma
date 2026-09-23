//! `nooma source`, `nooma index` and `nooma find`, run as a script would run
//! them.
//!
//! The corpus is written here, synthetic, in English and Russian. The library
//! goes into a temporary `--store`, never the user's data directory.

use std::path::Path;
use std::process::{Command, Output};

fn nooma(store: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nooma"))
        .args(args)
        .arg("--store")
        .arg(store)
        .output()
        .expect("the binary did not start")
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
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        nooma(self.store.path(), args)
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
    let hits = json["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["title"], "Appliance receipts");
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
        let object = printed.as_object().expect("every --json prints one object");
        let rows = object.values().filter_map(|v| v.as_array().and_then(|a| a.first()).and_then(|v| v.as_object()));
        for field in object.keys().chain(rows.flat_map(|row| row.keys())) {
            assert!(
                page.contains(&format!("`{field}`")),
                "`nooma {args:?}` prints `{field}`, which the reference page does not mention"
            );
        }
    }
}
