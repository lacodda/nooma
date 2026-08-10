//! nooma — local semantic search over your files.
//!
//! Two indexes are queried together: `tantivy` for exact matches and a vector
//! index for meaning. Neither half alone is the product — see
//! `docs/adr/0001-hybrid-index.md`.

fn main() {
    println!("nooma {}", env!("CARGO_PKG_VERSION"));
    println!("Scaffold only — indexing and search land in v0.1.0.");
}
