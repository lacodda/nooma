//! Measuring search against questions whose answers are known.
//!
//! A search that "feels better" after a change has not been shown to be
//! better. A query set says, for each question, which documents answer it;
//! an evaluation asks each way of searching every question and counts where
//! the answer landed. The same set, run against the full-text index and
//! against each model's vectors, is how a model is chosen and how a change to
//! chunking or ranking is judged.
//!
//! # The query set
//!
//! JSON, written by hand:
//!
//! ```json
//! { "queries": [
//!   { "query": "how do I get my money back for the kettle",
//!     "expect": ["home/receipts.md"],
//!     "group": "en→ru" }
//! ] }
//! ```
//!
//! `expect` names documents by their path inside a source, with `/` between
//! folders, so one set works on any machine the folders are copied to. Every
//! expected document must be in the library: a misspelt path would otherwise
//! count as a miss for every engine, and read as all of them being worse.
//! `group` is free text; results are reported per group as well as overall.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::embed::{Embedder, Role};
use crate::error::{Error, Result};
use crate::library::Library;

/// How deep into the results an answer is looked for.
pub const DEPTH: usize = 10;

/// Questions with known answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuerySet {
    /// The questions.
    pub queries: Vec<EvalQuery>,
}

/// One question, and the documents that answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalQuery {
    /// What a person would type.
    pub query: String,
    /// The documents that answer it, by path inside their source.
    pub expect: Vec<String>,
    /// A name to report it under with others like it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

impl QuerySet {
    /// Read a query set from a JSON file.
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        let set: Self = serde_json::from_slice(&bytes).map_err(|e| Error::QuerySet {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        if set.queries.is_empty() {
            return Err(Error::QuerySet {
                path: path.to_path_buf(),
                reason: "it has no queries".to_string(),
            });
        }
        if let Some(query) = set.queries.iter().find(|q| q.expect.is_empty()) {
            return Err(Error::QuerySet {
                path: path.to_path_buf(),
                reason: format!("{:?} expects no document, so it can only be missed", query.query),
            });
        }
        Ok(set)
    }
}

/// How well one way of searching answered a set of questions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Metrics {
    /// Questions asked.
    pub queries: usize,
    /// The share answered by the first result.
    pub hit_at_1: f64,
    /// The share answered within the first three.
    pub hit_at_3: f64,
    /// The share answered within the first ten.
    pub hit_at_10: f64,
    /// The mean of one over the rank of the first answer, zero when it is not
    /// in the first ten: rewards putting the answer first, not just near.
    pub mrr: f64,
}

impl Metrics {
    fn of(ranks: &[Option<usize>]) -> Self {
        let n = ranks.len();
        if n == 0 {
            return Self::default();
        }
        let share = |k: usize| ranks.iter().filter(|r| r.is_some_and(|r| r <= k)).count() as f64 / n as f64;
        Self {
            queries: n,
            hit_at_1: share(1),
            hit_at_3: share(3),
            hit_at_10: share(DEPTH),
            mrr: ranks.iter().map(|r| r.map_or(0.0, |r| 1.0 / r as f64)).sum::<f64>() / n as f64,
        }
    }
}

/// Where one question's answer landed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    /// The question.
    pub query: String,
    /// Its group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The rank of the first expected document, from 1; `None` when it is not
    /// in the first ten.
    pub rank: Option<usize>,
    /// The first three documents found, by path inside their source.
    pub top: Vec<String>,
}

/// One way of searching, measured.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EngineReport {
    /// `fulltext`, or a model's id.
    pub engine: String,
    /// Every question together.
    pub overall: Metrics,
    /// Per group.
    pub groups: BTreeMap<String, Metrics>,
    /// Per question.
    pub outcomes: Vec<Outcome>,
    /// The mean time to answer one question, in milliseconds; for a model it
    /// includes turning the question into a vector.
    pub query_ms: f64,
}

/// A query set checked against a library, ready to be asked.
#[derive(Debug)]
pub struct Evaluation<'a> {
    library: &'a Library,
    set: QuerySet,
    /// Per question, the library's keys for the documents it expects.
    expected: Vec<BTreeSet<String>>,
}

impl<'a> Evaluation<'a> {
    /// Check that every expected document is in the library.
    pub fn new(library: &'a Library, set: QuerySet) -> Result<Self> {
        let present: BTreeSet<String> = library.stored_chunks()?.into_iter().map(|chunk| chunk.path).collect();
        let mut expected = Vec::with_capacity(set.queries.len());
        let mut unknown = BTreeSet::new();
        for query in &set.queries {
            let mut keys = BTreeSet::new();
            for rel in &query.expect {
                let found: Vec<String> = library
                    .sources()
                    .iter()
                    .map(|source| join(&source.path, rel))
                    .filter(|key| present.contains(key))
                    .collect();
                if found.is_empty() {
                    unknown.insert(rel.clone());
                }
                keys.extend(found);
            }
            expected.push(keys);
        }
        if !unknown.is_empty() {
            return Err(Error::QuerySet {
                path: PathBuf::from("<query set>"),
                reason: format!("these expected documents are not in the library: {unknown:?}"),
            });
        }
        Ok(Self { library, set, expected })
    }

    /// Ask every question of the full-text index.
    pub fn fulltext(&self) -> Result<EngineReport> {
        let finder = self.library.finder()?;
        self.run("fulltext", |query| {
            let hits = match &finder {
                Some(finder) => finder.search(query, DEPTH)?,
                None => Vec::new(),
            };
            Ok(hits.into_iter().map(|hit| hit.path.to_string_lossy().into_owned()).collect())
        })
    }

    /// Ask every question of one model's vectors, which must be up to date.
    pub fn semantic(&self, embedder: &mut dyn Embedder) -> Result<EngineReport> {
        let model = embedder.model_id().to_string();
        let index = self.library.semantic(embedder)?;
        self.run(&model, |query| {
            let vector = embedder.embed(&[query], Role::Query)?.pop().unwrap_or_default();
            Ok(index
                .search(&vector, DEPTH)
                .into_iter()
                .map(|hit| hit.path.to_string_lossy().into_owned())
                .collect())
        })
    }

    fn run(&self, engine: &str, mut ask: impl FnMut(&str) -> Result<Vec<String>>) -> Result<EngineReport> {
        let mut outcomes = Vec::with_capacity(self.set.queries.len());
        let mut ranks = Vec::with_capacity(self.set.queries.len());
        let started = Instant::now();
        for (query, expected) in self.set.queries.iter().zip(&self.expected) {
            let found = ask(&query.query)?;
            let rank = found.iter().take(DEPTH).position(|path| expected.contains(path)).map(|i| i + 1);
            ranks.push(rank);
            outcomes.push(Outcome {
                query: query.query.clone(),
                group: query.group.clone(),
                rank,
                top: found.iter().take(3).map(|path| self.relative(path)).collect(),
            });
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;

        let mut by_group: BTreeMap<String, Vec<Option<usize>>> = BTreeMap::new();
        for (query, rank) in self.set.queries.iter().zip(&ranks) {
            if let Some(group) = &query.group {
                by_group.entry(group.clone()).or_default().push(*rank);
            }
        }
        Ok(EngineReport {
            engine: engine.to_string(),
            overall: Metrics::of(&ranks),
            groups: by_group.into_iter().map(|(group, ranks)| (group, Metrics::of(&ranks))).collect(),
            outcomes,
            query_ms: elapsed / self.set.queries.len().max(1) as f64,
        })
    }

    /// A library key as a path inside its source, the way a query set names
    /// documents.
    fn relative(&self, key: &str) -> String {
        let path = Path::new(key);
        self.library
            .sources()
            .iter()
            .find_map(|source| path.strip_prefix(&source.path).ok())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

/// The library's key for a document named inside a source.
fn join(source: &Path, rel: &str) -> String {
    let mut path = source.to_path_buf();
    for part in rel.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_count_ranks_within_their_cutoffs() {
        let m = Metrics::of(&[Some(1), Some(2), Some(4), None]);
        assert_eq!(m.queries, 4);
        assert!((m.hit_at_1 - 0.25).abs() < 1e-9);
        assert!((m.hit_at_3 - 0.5).abs() < 1e-9);
        assert!((m.hit_at_10 - 0.75).abs() < 1e-9);
        assert!((m.mrr - (1.0 + 0.5 + 0.25) / 4.0).abs() < 1e-9);
        assert_eq!(Metrics::of(&[]), Metrics::default());
    }
}
