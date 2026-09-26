//! Fetching against a server on this machine, so the parts that break quietly
//! - resuming, refusing a changed file - are exercised without the network.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nooma_core::model::{ModelFile, ModelSpec, Pooling};
use nooma_fetch::{FetchError, FetchProgress, Fetcher};
use sha2::{Digest, Sha256};

/// A server that answers GETs by path, with or without ranges, and counts the
/// bytes of body it sends.
struct Server {
    base: String,
    sent: Arc<Mutex<u64>>,
    requests: Arc<Mutex<u32>>,
}

impl Server {
    fn start(files: HashMap<String, Vec<u8>>, honour_range: bool) -> Self {
        Self::stalling(files, honour_range, None)
    }

    /// A server whose first answer sends this many bytes of the body and
    /// then holds the connection open, sending nothing more.
    fn stalling(files: HashMap<String, Vec<u8>>, honour_range: bool, stall_after: Option<usize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let sent = Arc::new(Mutex::new(0u64));
        let counter = sent.clone();
        let asked = Arc::new(Mutex::new(0u32));
        let requests = asked.clone();
        std::thread::spawn(move || {
            let mut stall_after = stall_after;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut range: Option<usize> = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = value.trim().trim_end_matches('-').parse().ok();
                    }
                }
                // `/<owner>/<repo>/resolve/<revision>/<file path>`
                let name = path
                    .split_once("/resolve/")
                    .and_then(|(_, rest)| rest.split_once('/'))
                    .map(|(_, file)| file.to_string())
                    .unwrap_or_default();
                let Some(body) = files.get(&name) else {
                    write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                };
                let (status, from) = match range {
                    Some(from) if honour_range => ("206 Partial Content", from),
                    _ => ("200 OK", 0),
                };
                let slice = &body[from..];
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", slice.len()).unwrap();
                *requests.lock().unwrap() += 1;
                if let Some(bytes) = stall_after.take() {
                    stream.write_all(&slice[..bytes]).unwrap();
                    stream.flush().unwrap();
                    *counter.lock().unwrap() += bytes as u64;
                    // Held open and silent, on its own thread so the next
                    // request is answered.
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        drop(stream);
                    });
                    continue;
                }
                stream.write_all(slice).unwrap();
                *counter.lock().unwrap() += slice.len() as u64;
            }
        });
        Self { base, sent, requests: asked }
    }

    /// Requests answered with a file, since the server started.
    fn requests(&self) -> u32 {
        *self.requests.lock().unwrap()
    }

    /// Body bytes sent since the last call.
    fn sent(&self) -> u64 {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

fn sha256(bytes: &[u8]) -> &'static str {
    let digest = Sha256::digest(bytes);
    Box::leak(digest.iter().map(|b| format!("{b:02x}")).collect::<String>().into_boxed_str())
}

/// Two files: a large one under a folder and a small one at the top.
fn contents() -> (Vec<u8>, Vec<u8>) {
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    (big, br#"{"pad_token_id": 1}"#.to_vec())
}

fn spec(big: &[u8], small: &[u8]) -> &'static ModelSpec {
    let files: &'static [ModelFile] = Box::leak(Box::new([
        ModelFile {
            path: "onnx/model.onnx",
            bytes: big.len() as u64,
            sha256: sha256(big),
        },
        ModelFile {
            path: "config.json",
            bytes: small.len() as u64,
            sha256: sha256(small),
        },
    ]));
    Box::leak(Box::new(ModelSpec {
        id: "test-model",
        repository: "someone/test-model",
        revision: "0000000000000000000000000000000000000000",
        license: "MIT",
        files,
        graph: "onnx/model.onnx",
        weights: &[],
        dimensions: 4,
        pooling: Pooling::Mean,
        query_prefix: "",
        passage_prefix: "",
        max_tokens: 16,
    }))
}

fn serve(big: &[u8], small: &[u8], honour_range: bool) -> Server {
    Server::start(
        HashMap::from([("onnx/model.onnx".to_string(), big.to_vec()), ("config.json".to_string(), small.to_vec())]),
        honour_range,
    )
}

fn quiet() -> impl FnMut(FetchProgress<'_>) {
    |_| {}
}

fn part_of(models: &Path) -> PathBuf {
    models.join("test-model/onnx/model.onnx.part")
}

fn placed(models: &Path) -> PathBuf {
    models.join("test-model/onnx/model.onnx")
}

#[test]
fn a_model_is_fetched_whole_and_then_only_checked() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let server = serve(&big, &small, true);
    let models = tempfile::tempdir().unwrap();

    let first = Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap();
    assert_eq!(first.downloaded, vec!["onnx/model.onnx", "config.json"]);
    assert_eq!(first.received, (big.len() + small.len()) as u64);
    assert_eq!(std::fs::read(placed(models.path())).unwrap(), big);
    assert!(!part_of(models.path()).exists());
    assert!(spec.missing(models.path()).is_empty());
    server.sent();

    let second = Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap();
    assert!(second.downloaded.is_empty());
    assert_eq!(second.verified.len(), 2);
    assert_eq!(server.sent(), 0, "a checked file is not asked for again");
}

#[test]
fn an_interrupted_fetch_resumes_where_it_stopped() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let server = serve(&big, &small, true);
    let models = tempfile::tempdir().unwrap();
    let part = part_of(models.path());
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, &big[..100_000]).unwrap();

    let fetched = Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap();
    let rest = (big.len() - 100_000 + small.len()) as u64;
    assert_eq!(server.sent(), rest, "only the rest is sent");
    assert_eq!(fetched.received, rest);
    assert_eq!(std::fs::read(placed(models.path())).unwrap(), big);
}

/// A server that ignores the range sends the whole file; appending it to the
/// part would make a file a third too long.
#[test]
fn a_server_that_ignores_the_range_gets_the_file_started_again() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let server = serve(&big, &small, false);
    let models = tempfile::tempdir().unwrap();
    let part = part_of(models.path());
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    std::fs::write(&part, &big[..100_000]).unwrap();

    Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap();
    assert_eq!(std::fs::read(placed(models.path())).unwrap(), big);
}

#[test]
fn a_changed_file_upstream_is_refused_and_left_nowhere() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let mut changed = big.clone();
    changed[12_345] ^= 1;
    let server = serve(&changed, &small, true);
    let models = tempfile::tempdir().unwrap();

    let error = Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap_err();
    assert!(matches!(error, FetchError::Mismatch { .. }), "{error}");
    assert!(!placed(models.path()).exists());
    assert!(!part_of(models.path()).exists(), "a bad part is not resumed from next time");
}

#[test]
fn a_file_in_place_with_the_wrong_bytes_is_fetched_again() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let server = serve(&big, &small, true);
    let models = tempfile::tempdir().unwrap();
    let dest = placed(models.path());
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let mut wrong = big.clone();
    wrong[0] ^= 1;
    std::fs::write(&dest, &wrong).unwrap();

    let fetched = Fetcher::new(&server.base).fetch(spec, models.path(), &mut quiet()).unwrap();
    assert!(fetched.downloaded.contains(&"onnx/model.onnx"));
    assert_eq!(std::fs::read(&dest).unwrap(), big);
}

/// A connection that goes quiet half way is not waited on for ever: the
/// window ends it, and the rest is asked for on a new one.
#[test]
fn a_connection_that_goes_quiet_is_replaced_and_the_rest_fetched() {
    let (big, small) = contents();
    let spec = spec(&big, &small);
    let server = Server::stalling(
        HashMap::from([("onnx/model.onnx".to_string(), big.clone()), ("config.json".to_string(), small.clone())]),
        true,
        Some(120_000),
    );
    let models = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let fetched = Fetcher::new(&server.base)
        .window(std::time::Duration::from_millis(400))
        .fetch(spec, models.path(), &mut quiet())
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "waited on the silent connection");
    // The model file took two requests: the one that went quiet, and the
    // one that fetched the rest. One request would mean the server never
    // stalled, and this test proved nothing.
    assert_eq!(server.requests(), 3, "two for the model file, one for the config");
    assert_eq!(fetched.received, (big.len() + small.len()) as u64, "nothing fetched twice");
    assert_eq!(std::fs::read(placed(models.path())).unwrap(), big);
}
