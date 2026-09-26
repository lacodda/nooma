//! Search by meaning with the model nooma ships, measured on the bilingual
//! corpus in `tests/meaning`.
//!
//! The corpus is built to be hard in the ways that matter: each answer has a
//! near miss from the same domain and a decoy that shares its words and not
//! its meaning, and most questions are asked in the other language or in
//! words the answer does not use. A model that pooled wrongly, dropped its
//! query prefix or read Russian as noise would still produce vectors - well
//! formed, normalized and wrong - and only a measurement like this one says
//! so.
//!
//! The model is read from `NOOMA_MODELS`, or from nooma's own models folder.
//! When it is not there the test fails with the command that fetches it; it
//! does not skip itself, because a test that passes by not running is how a
//! broken model would ship.

use std::path::{Path, PathBuf};

use nooma_core::embed::OnnxEmbedder;
use nooma_core::eval::{Evaluation, QuerySet};
use nooma_core::{Embedder, Library, ModelSpec, Role};

fn models_dir() -> PathBuf {
    match std::env::var_os("NOOMA_MODELS") {
        Some(dir) => PathBuf::from(dir),
        None => nooma_core::model::default_models_dir().expect("a models folder"),
    }
}

fn model() -> OnnxEmbedder {
    let spec = ModelSpec::default_model();
    let dir = models_dir();
    let missing = spec.missing(&dir);
    assert!(
        missing.is_empty(),
        "the model {} is not in {} (missing {missing:?}). Fetch it once with \
         `cargo run -p nooma -- model fetch`, or point NOOMA_MODELS at a folder that has it. \
         This test measures the real model and does not skip.",
        spec.id,
        dir.display()
    );
    OnnxEmbedder::load(spec, &dir, 2).expect("the model loads")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn the_model_finds_a_document_by_meaning_in_either_language() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/meaning");
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus");
    copy_dir(&here.join("corpus"), &corpus);
    let mut library = Library::open_at(dir.path().join("store")).unwrap();
    library.add_source(&corpus, Vec::new()).unwrap();
    library.update().unwrap();

    let mut model = model();
    let report = library.update_vectors(&mut model, &|_| {}).unwrap();
    assert_eq!(report.embedded, report.passages);

    let set = QuerySet::read(&here.join("queries.json")).unwrap();
    let evaluation = Evaluation::new(&library, set).unwrap();
    let words = evaluation.fulltext().unwrap();
    let meaning = evaluation.semantic(&mut model).unwrap();

    // Printed for the record: `cargo test --test meaning -- --nocapture`.
    for engine in [&words, &meaning] {
        println!(
            "{:<24} hit@1 {:.2}  hit@3 {:.2}  hit@10 {:.2}  MRR {:.2}",
            engine.engine, engine.overall.hit_at_1, engine.overall.hit_at_3, engine.overall.hit_at_10, engine.overall.mrr
        );
        for (group, m) in &engine.groups {
            println!("  {group:<10} hit@1 {:.2}  hit@3 {:.2}  MRR {:.2}", m.hit_at_1, m.hit_at_3, m.mrr);
        }
    }

    // The floors sit below what the model measured when it was chosen
    // (overall MRR 0.63, hit@10 0.94; 1.00 within a language), by enough to
    // absorb a runtime update and not by enough to hide a broken prefix or
    // pooling, which cost far more than that.
    assert!(meaning.overall.mrr >= 0.55, "MRR {:.2}", meaning.overall.mrr);
    assert!(meaning.overall.hit_at_10 >= 0.9, "hit@10 {:.2}", meaning.overall.hit_at_10);
    for group in ["ru→ru", "en→en", "keywords"] {
        let m = meaning.groups[group];
        assert!(m.mrr >= 0.95, "{group}: MRR {:.2}", m.mrr);
    }

    // Across languages this model is weak, and the test says so rather than
    // hiding it: it ranks the near miss in the question's own language above
    // the answer in the other (hit@1 was 0.00 both ways when it was chosen).
    // What it must still do is find the answer somewhere near the top, where
    // the words of the question cannot find it at all.
    let across: Vec<_> = meaning
        .outcomes
        .iter()
        .filter(|o| matches!(o.group.as_deref(), Some("ru→en" | "en→ru")))
        .collect();
    let found = across.iter().filter(|o| o.rank.is_some()).count() as f64 / across.len() as f64;
    assert!(found >= 0.7, "across languages, in the first ten: {found:.2}");
    for group in ["ru→en", "en→ru"] {
        assert!(
            meaning.groups[group].mrr > words.groups[group].mrr,
            "{group}: meaning {:.2} does not beat words {:.2}",
            meaning.groups[group].mrr,
            words.groups[group].mrr
        );
    }
}

/// The model was trained to read "query: " and "passage: " as two different
/// things. Leaving the prefixes off costs ranking quality and raises no error,
/// so the difference itself is what is checked.
#[test]
fn a_query_and_a_passage_are_read_as_different_things() {
    let mut model = model();
    let spec = ModelSpec::default_model();
    let as_query = model.embed(&["kettle"], Role::Query).unwrap().pop().unwrap();
    let as_passage = model.embed(&["kettle"], Role::Passage).unwrap().pop().unwrap();
    assert_eq!(as_query.len(), spec.dimensions);
    let norm: f32 = as_query.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-4, "unit length, so a dot product is a cosine: {norm}");
    if spec.query_prefix != spec.passage_prefix {
        let cosine: f32 = as_query.iter().zip(&as_passage).map(|(a, b)| a * b).sum();
        assert!(cosine < 0.999, "the prefixes made no difference: {cosine}");
    }
}

/// A passage's vector must not depend on what else was in its batch: the
/// store keys a vector by its text alone and reuses it anywhere.
#[test]
fn a_passage_reads_the_same_alone_and_in_a_batch() {
    let mut model = model();
    let alone = model.embed(&["Descale the kettle with citric acid."], Role::Passage).unwrap().pop().unwrap();
    let long = "A much longer passage about something else entirely, which pads the batch. ".repeat(12);
    let batched = model
        .embed(&[long.as_str(), "Descale the kettle with citric acid."], Role::Passage)
        .unwrap()
        .pop()
        .unwrap();
    let cosine: f32 = alone.iter().zip(&batched).map(|(a, b)| a * b).sum();
    assert!(cosine > 0.9999, "padding changed the vector: {cosine}");
}
