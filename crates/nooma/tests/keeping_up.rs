//! `nooma health` and `nooma index --watch`, run as a person runs them.
//!
//! The notes are synthetic and written here; the library goes into a
//! temporary `--store`, and no model is on the machine as far as these tests
//! go - an empty `--models` folder.

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn nooma(store: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nooma"));
    command.args(args).arg("--store").arg(store);
    command
}

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("the binary did not start");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

struct Fixture {
    notes: tempfile::TempDir,
    store: tempfile::TempDir,
    models: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let notes = tempfile::tempdir().unwrap();
        std::fs::write(notes.path().join("kettle.md"), "# Kettle\n\nDescale the kettle with citric acid.\n").unwrap();
        let fixture = Self {
            notes,
            store: tempfile::tempdir().unwrap(),
            models: tempfile::tempdir().unwrap(),
        };
        run(nooma(fixture.store.path(), &["source", "add"]).arg(fixture.notes.path()));
        fixture
    }

    fn index(&self, extra: &[&str]) -> Command {
        let mut command = nooma(self.store.path(), &["index"]);
        command.args(extra).arg("--models").arg(self.models.path());
        command
    }
}

#[test]
fn health_says_what_was_not_read_and_why() {
    let fixture = Fixture::new();
    run(&mut nooma(fixture.store.path(), &["health"]));
    std::fs::write(fixture.notes.path().join("huge.txt"), vec![b'a'; 17 * 1024 * 1024]).unwrap();
    std::fs::write(fixture.notes.path().join("damaged.pdf"), b"%PDF-1.7\nnot really").unwrap();
    let first = stdout(&run(&mut fixture.index(&[])));
    assert!(first.contains("skipped") && first.contains("huge.txt"), "{first}");

    // Not tried again, and said all the same.
    let second = stdout(&run(&mut fixture.index(&[])));
    assert!(!second.contains("skipped"), "{second}");
    assert!(second.contains("2 files are not in the index: `nooma health` says which and why"), "{second}");

    let text = stdout(&run(&mut nooma(fixture.store.path(), &["health"])));
    assert!(text.contains("not read: 2"), "{text}");
    assert!(text.contains("  too large\n"), "{text}");
    assert!(text.contains("huge.txt - larger than 16 MB (17 MB)"), "{text}");
    assert!(text.contains("  damaged, or not readable by their reader\n"), "{text}");
    let json: serde_json::Value = serde_json::from_slice(&run(&mut nooma(fixture.store.path(), &["health", "--json"])).stdout).unwrap();
    let whys: Vec<&str> = json["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|skipped| skipped["why"].as_str().unwrap())
        .collect();
    assert_eq!(whys, ["too_large", "damaged"]);
    assert_eq!(json["skipped"][0]["size"], 17 * 1024 * 1024);

    std::fs::remove_file(fixture.notes.path().join("huge.txt")).unwrap();
    std::fs::remove_file(fixture.notes.path().join("damaged.pdf")).unwrap();
    run(&mut fixture.index(&[]));
    let clear = stdout(&run(&mut nooma(fixture.store.path(), &["health"])));
    assert_eq!(clear.trim(), "every file found was read");
}

/// The watching process, killed when the test is done with it.
struct Watching(Child);

impl Drop for Watching {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn index_watch_keeps_the_index_up_to_date() {
    let fixture = Fixture::new();
    let mut child = fixture
        .index(&["--watch", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary did not start");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    // Read for as long as it runs: a pipe closed under it fails its writes.
    let mut errors = BufReader::new(child.stderr.take().unwrap()).lines();
    let watching = Watching(child);

    // The first update, as one line, then the word that it watches.
    let first: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(first["documents"], 1);
    let said = errors
        .by_ref()
        .map_while(Result::ok)
        .find(|line| line.contains("watching"))
        .expect("it never said it watches");
    assert!(said.contains("the folder for changes"), "{said}");
    std::thread::sleep(Duration::from_millis(500));

    std::fs::write(fixture.notes.path().join("roses.md"), "# Roses\n\nPrune the roses in February.\n").unwrap();
    let next: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!((next["indexed"].as_u64(), next["documents"].as_u64()), (Some(1), Some(2)), "{next}");

    // What it wrote is what another nooma finds.
    let started = Instant::now();
    let found = loop {
        let mut find = nooma(fixture.store.path(), &["find", "prune roses", "--no-refresh", "--json"]);
        let output = run(find.arg("--models").arg(fixture.models.path()));
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if json["hits"].as_array().is_some_and(|hits| !hits.is_empty()) || started.elapsed() > Duration::from_secs(10) {
            break json;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(found["hits"][0]["title"], "Roses");
    drop(watching);
    drop(errors);
}
