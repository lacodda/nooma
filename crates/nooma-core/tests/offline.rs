//! The offline promise, checked in the dependency graph rather than trusted.
//!
//! nooma says no byte of a person's files leaves the machine, and that the
//! one network call it makes is fetching a model, when asked. The first half
//! holds only if the crate that reads files cannot open a connection at all:
//! no HTTP client, no TLS, no git transport able to fetch. So this asks cargo
//! what `nooma-core` links - with the model runner switched on, as nooma
//! builds it - and fails if any of it could reach the network. `nooma-fetch`
//! is the positive control: it must show the client, or the check below
//! would pass on a query that silently found nothing.

use std::process::Command;

/// Crates whose presence means the code could open a connection.
const NETWORK: &[&str] = &[
    "ureq",
    "reqwest",
    "hyper",
    "curl",
    "isahc",
    "attohttpc",
    "hf-hub",
    "rustls",
    "native-tls",
    "openssl",
    "tokio",
];

/// Every package in a crate's normal dependency graph, with the features
/// each is built with: `gix-transport feature "blocking-client"`.
fn graph(package: &str, features: &str) -> String {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new(env!("CARGO"))
        .current_dir(&workspace)
        .args(["tree", "--offline", "--locked", "-p", package, "-e", "normal,features", "--prefix", "none"])
        .args(if features.is_empty() { vec![] } else { vec!["--features", features] })
        .output()
        .expect("cargo did not start");
    assert!(output.status.success(), "cargo tree failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn packages(graph: &str) -> Vec<&str> {
    graph
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn the_crate_that_reads_files_links_nothing_that_reaches_the_network() {
    let core = graph("nooma-core", "semantic");
    assert!(core.contains("fastembed"), "the model runner must be in the graph being checked");
    let linked = packages(&core);
    let found: Vec<&&str> = NETWORK.iter().filter(|name| linked.contains(name)).collect();
    assert!(found.is_empty(), "nooma-core links {found:?}, which can reach the network");

    // gix is here to read repositories. Its transport crate comes along for
    // URLs and types; the features that make it a client must not.
    let clients: Vec<&str> = core
        .lines()
        .filter(|line| line.starts_with("gix-transport feature") && (line.contains("client") || line.contains("http")))
        .collect();
    assert!(clients.is_empty(), "gix can fetch: {clients:?}");
}

#[test]
fn the_fetching_crate_shows_its_client_so_the_check_is_not_blind() {
    let fetch = graph("nooma-fetch", "");
    let linked = packages(&fetch);
    assert!(linked.contains(&"ureq"), "the check would pass on a graph it cannot read");
    assert!(linked.contains(&"rustls"));
}
