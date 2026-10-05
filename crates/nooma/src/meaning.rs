//! Search by meaning on the command line: loading the model, computing the
//! vectors with a progress line, and asking the vector index.
//!
//! Every command that searches by meaning answers from the stored index and
//! says how much of the library it covers. Computing vectors is indexing -
//! an hour for a large library the first time - so it belongs to
//! `nooma index`, never to a search that was asked for an answer.

use std::io::IsTerminal as _;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use nooma_core::embed::OnnxEmbedder;
use nooma_core::library::indexing_threads;
use nooma_core::model::ModelSpec;
use nooma_core::{Embedder as _, Finder, Hit, Library, Progress, Role};

use crate::model::size;

/// Whether search by meaning could answer, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// It answered.
    Ready,
    /// The model has not been fetched.
    NoModel,
    /// The model is here and no vectors have been computed with it.
    NoVectors,
}

impl State {
    /// How `--json` names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NoModel => "no-model",
            Self::NoVectors => "no-vectors",
        }
    }
}

/// What search by meaning found.
#[derive(Debug)]
pub struct Meaning {
    /// The model that would answer.
    pub model: &'static str,
    /// Whether it could.
    pub state: State,
    /// Documents its index does not cover yet.
    pub behind: usize,
    /// The documents found, closest first; the score is a cosine.
    pub hits: Vec<Hit>,
}

impl Meaning {
    /// As `--json` prints it.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "state": self.state.name(),
            "behind": self.behind,
            "hits": self.hits,
        })
    }

    fn not(state: State) -> Self {
        Self {
            model: ModelSpec::default_model().id,
            state,
            behind: 0,
            hits: Vec::new(),
        }
    }
}

/// The model nooma uses, loaded; `None` when it has not been fetched.
pub fn load(models: &Path) -> Result<Option<OnnxEmbedder>> {
    let spec = ModelSpec::default_model();
    if !spec.missing(models).is_empty() {
        return Ok(None);
    }
    Ok(Some(OnnxEmbedder::load(spec, models, indexing_threads())?))
}

/// Search the vector index for a text: a question, or a passage to find
/// more like.
///
/// The model is loaded only when there is an index to search - loading it
/// costs two seconds, and a library with no vectors would pay them for
/// nothing.
pub fn search(library: &Library, finder: Option<&Finder>, models: &Path, text: &str, role: Role, limit: usize, skip: Option<&Path>) -> Result<Meaning> {
    let spec = ModelSpec::default_model();
    if !spec.missing(models).is_empty() {
        return Ok(Meaning::not(State::NoModel));
    }
    let (Some(finder), Some(index)) = (finder, library.semantic(spec)?) else {
        return Ok(Meaning::not(State::NoVectors));
    };
    let Some(mut embedder) = load(models)? else {
        return Ok(Meaning::not(State::NoModel));
    };
    let vector = embedder.embed(&[text], role)?.pop().unwrap_or_default();
    Ok(Meaning {
        model: spec.id,
        state: State::Ready,
        behind: index.behind(),
        hits: index.hits(finder, &vector, limit, skip)?,
    })
}

/// The line a person reads when search by meaning could not answer, or
/// answered over part of the library.
pub fn caveat(meaning: &Meaning) -> Option<String> {
    let spec = ModelSpec::default_model();
    match meaning.state {
        State::NoModel => Some(format!(
            "to search by meaning too, fetch the model once: `nooma model fetch` ({}, the one download nooma makes)",
            size(spec.bytes())
        )),
        State::NoVectors => Some("no vectors yet: `nooma index` computes them".to_string()),
        State::Ready if meaning.behind > 0 => Some(format!(
            "{} documents are not covered by meaning yet: `nooma index` brings them in",
            meaning.behind
        )),
        State::Ready => None,
    }
}

/// A progress line for computing vectors, with the time left once there is
/// enough of a pace to tell it by.
pub fn progress_for(model: &str) -> impl Fn(Progress) + Sync + '_ {
    let terminal = std::io::stderr().is_terminal();
    let started = Instant::now();
    move |progress: Progress| {
        if progress.total == 0 {
            return;
        }
        if !terminal {
            if progress.done == 0 {
                eprintln!("{model}: computing {} vectors", progress.total);
            }
            return;
        }
        let elapsed = started.elapsed().as_secs_f64();
        let left = if progress.done > 0 && elapsed >= 20.0 {
            let seconds = elapsed / progress.done as f64 * (progress.total - progress.done) as f64;
            format!(" · {} left", duration(seconds))
        } else {
            String::new()
        };
        eprint!("\r{model}: {} / {} passages{left}   ", progress.done, progress.total);
        if progress.done == progress.total {
            eprintln!();
        }
    }
}

/// A rough duration for a person: `40 s`, `12 min`, `1 h 20 min`.
fn duration(seconds: f64) -> String {
    let seconds = seconds.round() as u64;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds.div_ceil(60)),
        _ => format!("{} h {} min", seconds / 3600, (seconds % 3600) / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_reads_in_the_unit_it_is_counted_in() {
        assert_eq!(duration(40.2), "40 s");
        assert_eq!(duration(61.0), "2 min");
        assert_eq!(duration(4800.0), "1 h 20 min");
    }
}
