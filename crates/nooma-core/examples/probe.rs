//! A scratch probe for the gix API, run by hand against a live repository.
//!
//! gix moves its surface between 0.x releases without a migration guide, so a
//! small runnable probe is cheaper than discovering the shape through the
//! compiler inside real code.
//!
//! `cargo run -p nooma-core --example probe -- <path>`

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = gix::discover(&path)?;
    let commit = repo.head_commit()?;
    println!("commit {}", commit.id());

    let tree = commit.tree()?;

    // What a tree's entries look like at the top level.
    for entry in tree.iter().take(5) {
        let entry = entry?;
        println!("  entry {:?} {:?}", entry.filename(), entry.mode());
    }

    // Looking one file up by path, and reading its committed bytes.
    let found = tree.clone().lookup_entry_by_path("Cargo.toml")?;
    match found {
        Some(entry) => {
            let object = entry.object()?;
            let blob = object.into_blob();
            println!("Cargo.toml: {} committed bytes", blob.data.len());
            println!("blake3 {}", blake3::hash(&blob.data).to_hex());
        }
        None => println!("Cargo.toml is not in this commit"),
    }

    Ok(())
}
