//! Measure a model's language on the parallel set and write what the
//! catalogue keeps of it, `data/axes/<model>.json`.
//!
//! ```text
//! cargo run --release -p nooma-core --features semantic --example language_axes -- multilingual-e5-small
//! ```
//!
//! The model is read from the models directory, as `nooma model fetch` left
//! it, or from `NOOMA_MODELS`. Run again whenever the parallel set or the
//! model's recipe changes: the catalogue's tests say when one has.

use std::path::{Path, PathBuf};

use nooma_core::embed::OnnxEmbedder;
use nooma_core::neutral::{AXES, LanguageAxes, ParallelSet};
use nooma_core::{Model as _, ModelSpec};

fn main() -> anyhow::Result<()> {
    let id = std::env::args().nth(1).unwrap_or_else(|| nooma_core::model::DEFAULT_MODEL.to_string());
    let spec = ModelSpec::find(&id).ok_or_else(|| anyhow::anyhow!("{id} is not in the catalogue"))?;
    let models = match std::env::var_os("NOOMA_MODELS").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => nooma_core::model::default_models_dir()?,
    };
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let set = ParallelSet::parse(&std::fs::read_to_string(crate_dir.join("data/parallel-en-ru.json"))?)?;
    let mut embedder = OnnxEmbedder::load(spec, &models, nooma_core::library::indexing_threads())?;
    let axes = LanguageAxes::measure(&mut embedder, &set, AXES)?;
    let out = crate_dir.join("data/axes").join(format!("{}.json", embedder.model_id()));
    std::fs::create_dir_all(out.parent().expect("a file in a directory"))?;
    std::fs::write(&out, format!("{}\n", serde_json::to_string(&axes)?))?;
    println!(
        "{}: {} directions from {} pairs -> {}",
        spec.id,
        axes.axes.len(),
        set.pairs.len(),
        out.display()
    );
    Ok(())
}
