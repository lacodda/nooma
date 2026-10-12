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
use nooma_core::neutral::{AXES, LanguageAxes, ParallelSet};
use nooma_core::{Embedder, Library, Model as _, ModelSpec, Role};

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
    let hybrid = evaluation.hybrid(&mut model).unwrap();

    // Printed for the record: `cargo test --test meaning -- --nocapture`.
    for engine in [&words, &meaning, &hybrid] {
        println!(
            "{:<24} hit@1 {:.2}  hit@3 {:.2}  hit@10 {:.2}  MRR {:.2}",
            engine.engine, engine.overall.hit_at_1, engine.overall.hit_at_3, engine.overall.hit_at_10, engine.overall.mrr
        );
        for (group, m) in &engine.groups {
            println!("  {group:<10} hit@1 {:.2}  hit@3 {:.2}  MRR {:.2}", m.hit_at_1, m.hit_at_3, m.mrr);
        }
    }

    // The floors sit below what the model measured once its language was
    // taken out (overall MRR 0.86, hit@10 1.00; 1.00 within a language), by
    // enough to absorb a runtime update and not by enough to hide a broken
    // prefix or pooling, which cost far more than that.
    assert!(meaning.overall.mrr >= 0.8, "MRR {:.2}", meaning.overall.mrr);
    assert!(meaning.overall.hit_at_10 >= 0.95, "hit@10 {:.2}", meaning.overall.hit_at_10);
    for group in ["ru→ru", "en→en", "keywords"] {
        let m = meaning.groups[group];
        assert!(m.mrr >= 0.95, "{group}: MRR {:.2}", m.mrr);
    }

    // Across languages. As the model makes them, its vectors rank the near
    // miss in the question's own language above the answer in the other
    // (MRR 0.17 en→ru, 0.35 ru→en); compared without the language, the answer
    // leads or follows the near miss (0.67, 0.79). Words cannot find it at all.
    for group in ["ru→en", "en→ru"] {
        let m = meaning.groups[group];
        assert!(m.mrr >= 0.55, "{group}: MRR {:.2}", m.mrr);
        assert!(m.hit_at_3 >= 0.9, "{group}: hit@3 {:.2}", m.hit_at_3);
        assert!(m.mrr > words.groups[group].mrr + 0.5, "{group}: words {:.2}", words.groups[group].mrr);
    }

    // The one list `find` shows. Every question in this set is built so that
    // its words mislead - the decoys share them - and the words still must
    // not push the answer the meaning found out of the first three: that is
    // what weighing them by how much of the question they hold is for. Within
    // a language and for keywords it answers first, as either half alone
    // does; across languages the decoy that shares a word with the question
    // may stand above the answer, not far above (0.56 en→ru, 0.46 ru→en, and
    // 0.13, 0.24 with the language left in).
    assert!(hybrid.overall.mrr >= 0.7, "hybrid MRR {:.2}", hybrid.overall.mrr);
    assert!(hybrid.overall.mrr > words.overall.mrr);
    assert!(hybrid.overall.hit_at_3 >= 0.95, "hybrid hit@3 {:.2}", hybrid.overall.hit_at_3);
    for group in ["ru→ru", "en→en", "keywords"] {
        let m = hybrid.groups[group];
        assert!(m.mrr >= 0.95, "hybrid, {group}: MRR {:.2}", m.mrr);
    }
    for group in ["ru→en", "en→ru"] {
        let m = hybrid.groups[group];
        assert!(m.mrr >= 0.4, "hybrid, {group}: MRR {:.2}", m.mrr);
    }
}

/// The directions the catalogue removes from this model's vectors are the
/// ones the parallel set gives when measured with it now - not with the
/// runtime it was measured with once. Compared as what they remove, since a
/// direction and its opposite remove the same thing.
#[test]
fn the_catalogue_s_language_is_what_the_parallel_set_gives_this_model() {
    let mut model = model();
    let kept = model.language().expect("the default model's language is measured").clone();
    let set = ParallelSet::parse(include_str!("../data/parallel-en-ru.json")).unwrap();
    let measured = LanguageAxes::measure(&mut model, &set, AXES).unwrap();
    assert_eq!(measured.set, kept.set);
    assert_eq!(measured.recipe, kept.recipe);
    // The overlap of the two pairs of directions: one per direction kept,
    // when they span the same plane.
    for axis in &kept.axes {
        let along: f32 = measured.axes.iter().map(|m| m.iter().zip(axis).map(|(a, b)| a * b).sum::<f32>().powi(2)).sum();
        assert!(along > 0.999, "a kept direction lies {along:.4} in the measured plane");
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
