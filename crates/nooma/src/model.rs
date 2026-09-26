//! `nooma model` — the embedding models: which there are, which are on this
//! machine, fetching one and removing one.
//!
//! Fetching is the one thing nooma does over the network, and it happens here
//! and only when asked. Every other command that needs a model reads it from
//! disk, and says to run `nooma model fetch` when it is not there.

use std::io::IsTerminal as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use nooma_core::model::{self, MODELS, ModelSpec};

use crate::library::StoreArgs;

/// Where models are kept, for every command that loads or fetches one.
#[derive(Debug, Args)]
pub struct ModelsArgs {
    /// Keep models in this directory instead of the user's data directory
    #[arg(long = "models", value_name = "DIR", global = true, env = "NOOMA_MODELS")]
    dir: Option<PathBuf>,
}

impl ModelsArgs {
    pub(crate) fn dir(&self) -> Result<PathBuf> {
        match &self.dir {
            Some(dir) => Ok(dir.clone()),
            None => model::default_models_dir().context("finding where to keep models"),
        }
    }
}

/// The embedding models.
#[derive(Debug, Args)]
pub struct ModelArgs {
    #[command(subcommand)]
    command: ModelCommand,
    #[command(flatten)]
    models: ModelsArgs,
    #[command(flatten)]
    store: StoreArgs,
}

#[derive(Debug, Subcommand)]
enum ModelCommand {
    /// List the models nooma can run, and which are on this machine
    List {
        /// Print machine-readable JSON instead of a report
        #[arg(long)]
        json: bool,
    },
    /// Download a model; the only thing nooma does over the network
    Fetch {
        /// The model; the one nooma uses when left out
        model: Option<String>,
    },
    /// Delete a model, and the vectors computed with it
    Remove {
        /// The model
        model: String,
    },
}

/// The model with this id, or an error listing the ones there are.
pub(crate) fn spec(id: &str) -> Result<&'static ModelSpec> {
    ModelSpec::find(id).with_context(|| {
        let known: Vec<&str> = MODELS.iter().map(|spec| spec.id).collect();
        format!("there is no model {id:?}; the models are {}", known.join(", "))
    })
}

pub fn run(args: ModelArgs) -> Result<()> {
    let dir = args.models.dir()?;
    match args.command {
        ModelCommand::List { json } => list(&dir, json),
        ModelCommand::Fetch { model } => {
            let spec = match model {
                Some(id) => spec(&id)?,
                None => ModelSpec::default_model(),
            };
            fetch(spec, &dir)
        }
        ModelCommand::Remove { model } => {
            let spec = spec(&model)?;
            let files = spec.remove(&dir).with_context(|| format!("removing {}", spec.dir(&dir).display()))?;
            let library = args.store.open()?;
            let vectors = library.remove_vectors(spec.id)?;
            match (files, vectors) {
                (false, false) => println!("{} was not on this machine", spec.id),
                _ => println!("removed {}{}", spec.id, if vectors { " and its vectors" } else { "" }),
            }
            Ok(())
        }
    }
}

fn list(dir: &std::path::Path, json: bool) -> Result<()> {
    if json {
        let models: Vec<serde_json::Value> = MODELS
            .iter()
            .map(|spec| {
                serde_json::json!({
                    "id": spec.id,
                    "repository": spec.repository,
                    "revision": spec.revision,
                    "license": spec.license,
                    "dimensions": spec.dimensions,
                    "bytes": spec.bytes(),
                    "present": spec.missing(dir).is_empty(),
                    "default": spec.id == model::DEFAULT_MODEL,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "dir": dir, "models": models }))?);
        return Ok(());
    }
    let width = MODELS.iter().map(|spec| spec.id.len()).max().unwrap_or(0);
    for spec in MODELS {
        let state = if spec.missing(dir).is_empty() { "on this machine" } else { "not fetched" };
        let default = if spec.id == model::DEFAULT_MODEL { "  · used by nooma" } else { "" };
        println!("{:<width$}  {:>4} dims  {:>7}  {state}{default}", spec.id, spec.dimensions, size(spec.bytes()));
    }
    println!();
    println!("models live in {}", dir.display());
    Ok(())
}

fn fetch(spec: &'static ModelSpec, dir: &std::path::Path) -> Result<()> {
    if !spec.missing(dir).is_empty() {
        eprintln!("fetching {} ({}) from {}", spec.id, size(spec.bytes()), spec.repository);
    }
    let terminal = std::io::stderr().is_terminal();
    let mut current = String::new();
    let mut shown = 0u64;
    let fetched = nooma_fetch::fetch(spec, dir, &mut |progress| {
        if progress.file != current {
            if terminal && !current.is_empty() {
                eprintln!();
            }
            current = progress.file.to_string();
            shown = 0;
            if !terminal {
                eprintln!("  {} ({})", progress.file, size(progress.total));
            }
        }
        // A redraw per megabyte is plenty, and keeps a slow terminal from
        // becoming the bottleneck.
        if terminal && (progress.done - shown >= 1 << 20 || progress.done == progress.total) {
            shown = progress.done;
            eprint!("\r  {}  {} / {}   ", progress.file, size(progress.done), size(progress.total));
        }
    });
    if terminal && !current.is_empty() {
        eprintln!();
    }
    let fetched = match fetched {
        Ok(fetched) => fetched,
        Err(error) => bail!("fetching {}: {error}", spec.id),
    };
    if fetched.downloaded.is_empty() {
        println!("{} is on this machine, every file checked against its pinned hash", spec.id);
    } else {
        println!(
            "fetched {} ({} received), every file checked against its pinned hash",
            spec.id,
            size(fetched.received)
        );
    }
    println!("in {}", spec.dir(dir).display());
    Ok(())
}

/// A size for a person: `471 MB`, `2.3 GB`.
pub(crate) fn size(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    let mb = bytes as f64 / MB;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{} KB", bytes.div_ceil(1000))
    }
}
