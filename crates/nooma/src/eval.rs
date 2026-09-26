//! `nooma eval` — how well search answers a set of questions whose answers
//! are known, by the full-text index and by each model's vectors, side by
//! side.
//!
//! It is how the model was chosen and how a change to chunking or ranking is
//! judged: on the reader's own folders, with questions the reader wrote.
//! Vectors a model has not computed yet are computed first, and kept - the
//! second run over the same library costs only the questions.

use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Args;
use nooma_core::embed::OnnxEmbedder;
use nooma_core::eval::{EngineReport, Evaluation, Metrics, QuerySet};
use nooma_core::library::indexing_threads;
use nooma_core::model::ModelSpec;
use nooma_core::{Progress, VectorReport};

use crate::library::{StoreArgs, refresh};
use crate::model::{ModelsArgs, spec};

/// Measure search against questions with known answers.
#[derive(Debug, Args)]
pub struct EvalArgs {
    /// The query set: questions and the documents that answer them (JSON)
    queries: PathBuf,
    /// A model to measure beside the full-text index; repeatable. The one nooma uses when left out
    #[arg(long = "model", value_name = "ID")]
    model: Vec<String>,
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    /// Answer from the stored index without bringing it up to date
    #[arg(long)]
    no_refresh: bool,
    #[command(flatten)]
    store: StoreArgs,
    #[command(flatten)]
    models: ModelsArgs,
}

pub fn run(args: EvalArgs) -> Result<()> {
    let library = args.store.open()?;
    if library.sources().is_empty() {
        anyhow::bail!("no sources to measure against: `nooma source add <folder>` first");
    }
    if !args.no_refresh {
        refresh(&library)?;
    }
    let models_dir = args.models.dir()?;
    let specs: Vec<&'static ModelSpec> = if args.model.is_empty() {
        vec![ModelSpec::default_model()]
    } else {
        args.model.iter().map(|id| spec(id)).collect::<Result<_>>()?
    };
    // Every model is checked before any is run: finding the third missing
    // after the first two spent an hour computing vectors wastes the hour.
    for spec in &specs {
        let missing = spec.missing(&models_dir);
        if !missing.is_empty() {
            anyhow::bail!("the model {} is not on this machine — fetch it with `nooma model fetch {}`", spec.id, spec.id);
        }
    }

    let set = QuerySet::read(&args.queries)?;
    let evaluation = Evaluation::new(&library, set.clone())?;
    let status = library.status()?;

    let mut engines = vec![evaluation.fulltext()?];
    let mut vectors = Vec::new();
    for spec in specs {
        let loading = Instant::now();
        let mut embedder = OnnxEmbedder::load(spec, &models_dir, indexing_threads())?;
        let load_ms = loading.elapsed().as_millis();
        let report = library.update_vectors(&mut embedder, &progress_for(spec.id))?;
        if std::io::stderr().is_terminal() {
            eprintln!();
        }
        engines.push(evaluation.semantic(&mut embedder)?);
        vectors.push((report, load_ms));
    }

    if args.json {
        let out = serde_json::json!({
            "queries": set.queries.len(),
            "documents": status.documents,
            "chunks": status.chunks,
            "engines": engines,
            "vectors": vectors.iter().map(|(report, load_ms)| serde_json::json!({
                "model": report.model,
                "load_ms": load_ms,
                "report": report,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!("{} questions · {} documents · {} chunks", set.queries.len(), status.documents, status.chunks);
    println!();
    print_table("", &engines, |engine| Some(engine.overall), true);
    let groups: std::collections::BTreeSet<&String> = engines.iter().flat_map(|engine| engine.groups.keys()).collect();
    for group in groups {
        println!();
        print_table(group, &engines, |engine| engine.groups.get(group).copied(), false);
    }
    println!();
    for (report, load_ms) in &vectors {
        print_vectors(report, *load_ms);
    }
    for engine in &engines {
        let missed: Vec<_> = engine.outcomes.iter().filter(|outcome| outcome.rank.is_none()).collect();
        if missed.is_empty() {
            continue;
        }
        println!();
        println!("missed by {} ({}):", engine.engine, missed.len());
        for outcome in missed.iter().take(12) {
            println!("  {:?} → {}", outcome.query, outcome.top.first().map_or("nothing", String::as_str));
        }
        if missed.len() > 12 {
            println!("  … and {} more (--json lists them all)", missed.len() - 12);
        }
    }
    Ok(())
}

fn progress_for(model: &str) -> impl Fn(Progress) + Sync + '_ {
    let terminal = std::io::stderr().is_terminal();
    move |progress: Progress| {
        if progress.total == 0 {
            return;
        }
        if terminal {
            eprint!("\r{model}: {} / {} passages", progress.done, progress.total);
        } else if progress.done == 0 {
            eprintln!("{model}: computing {} vectors", progress.total);
        }
    }
}

fn print_table(title: &str, engines: &[EngineReport], metrics: impl Fn(&EngineReport) -> Option<Metrics>, timing: bool) {
    let width = engines.iter().map(|engine| engine.engine.len()).max().unwrap_or(0).max(title.len());
    let mut header = format!("{title:<width$}  hit@1  hit@3  hit@10    MRR");
    if timing {
        header.push_str("  ms/question");
    } else {
        header.push_str("  questions");
    }
    println!("{header}");
    for engine in engines {
        let Some(m) = metrics(engine) else { continue };
        let mut line = format!(
            "{:<width$}  {:>5.2}  {:>5.2}  {:>6.2}  {:>5.2}",
            engine.engine, m.hit_at_1, m.hit_at_3, m.hit_at_10, m.mrr
        );
        if timing {
            line.push_str(&format!("  {:>11.1}", engine.query_ms));
        } else {
            line.push_str(&format!("  {:>9}", m.queries));
        }
        println!("{line}");
    }
}

fn print_vectors(report: &VectorReport, load_ms: u128) {
    if report.embedded == 0 {
        println!("{}: loaded in {load_ms} ms; all {} passages already had vectors", report.model, report.passages);
        return;
    }
    let per_second = report.embedded as f64 / (report.embed_ms.max(1) as f64 / 1000.0);
    println!(
        "{}: loaded in {load_ms} ms; computed {} of {} passages in {:.0} s ({per_second:.1} a second)",
        report.model,
        report.embedded,
        report.passages,
        report.embed_ms as f64 / 1000.0
    );
    if let Some(reason) = &report.rebuilt {
        println!("  started the vectors again: {reason}");
    }
}
