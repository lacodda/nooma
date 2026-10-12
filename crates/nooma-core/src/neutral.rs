//! The meaning without the language.
//!
//! A multilingual model places a text by what it says, and, apart from that,
//! by the language it says it in. The second part is small, but it is the
//! same for every text of one language, so it adds up: a Russian question is
//! a little closer to every Russian note than to any English one, and in a
//! library of thousands of notes in both languages the English document that
//! answers it never reaches the first ten. Measured on a real library with
//! `multilingual-e5-small`, a question in one language found the answer in
//! the other for none of 25 questions.
//!
//! # Where the language is
//!
//! A pair of texts that say the same thing in two languages differ in their
//! language and in nothing else, so the difference of their vectors points
//! along the language. Over a few hundred pairs on every kind of subject, the
//! directions those differences share are found - here, the first two, which
//! stand well apart from the rest - and removed from every vector before two
//! are compared: a query and a passage are compared in what is left, the
//! meaning. The pairs live in `data/parallel-en-ru.json`, written for nooma.
//!
//! The obvious shortcut - subtracting the average vector of each alphabet -
//! takes the subject away with the language, because in a real library they
//! go together: the English documents are manuals, the Russian ones notes.
//! It shone on an invented test set and lost on the real one. The two texts
//! of a pair share their subject, so their difference cannot carry it.
//!
//! The directions belong to a model, and are measured once for it and kept
//! in the catalogue beside it: see [`crate::model`]. What is removed is
//! measured, not chosen - `docs/adr/0008-meaning-without-the-language.md`
//! has the numbers.

use serde::{Deserialize, Serialize};

use crate::embed::{Embedder, Role, normalize};
use crate::error::{Error, Result};

/// The directions removed for a model whose language was measured: the two
/// that stand apart from the rest. One cost a little, four gained nothing,
/// eight and more began to take the meaning too.
pub const AXES: usize = 2;

/// Texts that say the same thing in English and in Russian.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ParallelSet {
    /// The pairs.
    pub pairs: Vec<Pair>,
}

/// One text in two languages.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pair {
    /// In English.
    pub en: String,
    /// In Russian.
    pub ru: String,
}

impl ParallelSet {
    /// Read a set from its JSON.
    pub fn parse(json: &str) -> Result<Self> {
        let set: Self = serde_json::from_str(json).map_err(|e| Error::Embedding(format!("the parallel set could not be read: {e}")))?;
        if set.pairs.is_empty() {
            return Err(Error::Embedding("the parallel set has no pairs".to_string()));
        }
        Ok(set)
    }

    /// What names the set: a hash of its texts, not of its file - a checkout
    /// that turns the line endings does not make it another set.
    pub fn hash(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for pair in &self.pairs {
            for text in [&pair.en, &pair.ru] {
                hasher.update(&(text.len() as u64).to_le_bytes());
                hasher.update(text.as_bytes());
            }
        }
        hasher.finalize().to_hex().to_string()
    }
}

/// The directions along which one model's vectors say which language a text
/// is in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageAxes {
    /// The parallel set they were measured on, by [`ParallelSet::hash`].
    pub set: String,
    /// The recipe of the model they were measured with: see
    /// [`crate::Model::recipe`].
    pub recipe: String,
    /// Unit vectors at right angles to each other, the strongest first.
    pub axes: Vec<Vec<f32>>,
}

impl LanguageAxes {
    /// Measure a model's language on a parallel set: every text read as a
    /// query and as a passage, since a model with prefixes places the two
    /// apart, and the strongest `count` directions of the pairs' differences.
    pub fn measure(embedder: &mut dyn Embedder, set: &ParallelSet, count: usize) -> Result<Self> {
        let en: Vec<&str> = set.pairs.iter().map(|pair| pair.en.as_str()).collect();
        let ru: Vec<&str> = set.pairs.iter().map(|pair| pair.ru.as_str()).collect();
        let mut differences = Vec::with_capacity(set.pairs.len() * 2);
        for role in [Role::Query, Role::Passage] {
            let english = embed_all(embedder, &en, role)?;
            let russian = embed_all(embedder, &ru, role)?;
            for (e, r) in english.iter().zip(&russian) {
                differences.push(r.iter().zip(e).map(|(r, e)| f64::from(r - e)).collect::<Vec<f64>>());
            }
        }
        Ok(Self {
            set: set.hash(),
            recipe: embedder.recipe(),
            axes: strongest_directions(&differences, embedder.dimensions(), count),
        })
    }

    /// Take the language out of a vector, and scale what is left back to
    /// unit length so that a dot product is a cosine again.
    pub fn remove(&self, vector: &mut [f32]) {
        for axis in &self.axes {
            let along: f32 = vector.iter().zip(axis).map(|(v, a)| v * a).sum();
            for (v, a) in vector.iter_mut().zip(axis) {
                *v -= along * a;
            }
        }
        normalize(vector);
    }

    /// What names the space vectors are compared in once these are removed:
    /// an index built in one is not searched in another.
    pub fn space(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for axis in &self.axes {
            for x in axis {
                hasher.update(&x.to_le_bytes());
            }
        }
        hasher.finalize().to_hex().to_string()
    }
}

/// Every text as a vector, in batches the size the store uses.
fn embed_all(embedder: &mut dyn Embedder, texts: &[&str], role: Role) -> Result<Vec<Vec<f32>>> {
    let mut out = Vec::with_capacity(texts.len());
    for batch in texts.chunks(16) {
        let vectors = embedder.embed(batch, role)?;
        if vectors.len() != batch.len() || vectors.iter().any(|v| v.len() != embedder.dimensions()) {
            return Err(Error::Embedding(format!(
                "{} returned vectors of the wrong number or length",
                embedder.model_id()
            )));
        }
        out.extend(vectors);
    }
    Ok(out)
}

/// The `count` directions the rows lean along most: the leading eigenvectors
/// of the rows' second-moment matrix, found one at a time by repeated
/// multiplication, each kept at right angles to those before it. Each is
/// signed so that its largest component is positive, so one set of rows
/// always gives the same vectors.
fn strongest_directions(rows: &[Vec<f64>], dimensions: usize, count: usize) -> Vec<Vec<f32>> {
    let mut moment = vec![0.0f64; dimensions * dimensions];
    for row in rows {
        for i in 0..dimensions {
            let ri = row[i];
            for j in 0..dimensions {
                moment[i * dimensions + j] += ri * row[j];
            }
        }
    }
    let mut found: Vec<Vec<f64>> = Vec::with_capacity(count);
    for n in 0..count.min(dimensions) {
        // Any start not at right angles to the answer converges; one fixed
        // start keeps the result the same from run to run.
        let mut v: Vec<f64> = (0..dimensions).map(|i| ((i + n + 1) as f64).sin()).collect();
        for _ in 0..10_000 {
            let mut next: Vec<f64> = (0..dimensions)
                .map(|i| moment[i * dimensions..(i + 1) * dimensions].iter().zip(&v).map(|(m, x)| m * x).sum())
                .collect();
            for before in &found {
                let along: f64 = next.iter().zip(before).map(|(a, b)| a * b).sum();
                for (a, b) in next.iter_mut().zip(before) {
                    *a -= along * b;
                }
            }
            let norm = next.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm == 0.0 {
                break;
            }
            for x in &mut next {
                *x /= norm;
            }
            let moved = next.iter().zip(&v).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            v = next;
            if moved < 1e-12 {
                break;
            }
        }
        let largest = v.iter().copied().fold(0.0f64, |held, x| if x.abs() > held.abs() { x } else { held });
        if largest < 0.0 {
            for x in &mut v {
                *x = -*x;
            }
        }
        found.push(v);
    }
    found.into_iter().map(|v| v.into_iter().map(|x| x as f32).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET: &str = include_str!("../data/parallel-en-ru.json");

    #[test]
    fn the_parallel_set_reads_and_its_name_ignores_line_endings() {
        let set = ParallelSet::parse(SET).unwrap();
        assert!(set.pairs.len() >= 400, "{} pairs", set.pairs.len());
        assert!(set.pairs.iter().all(|pair| !pair.en.trim().is_empty() && !pair.ru.trim().is_empty()));
        let turned = ParallelSet::parse(&SET.replace('\n', "\r\n")).unwrap();
        assert_eq!(turned.hash(), set.hash());
        let mut other = set.clone();
        other.pairs[0].ru.push('!');
        assert_ne!(other.hash(), set.hash());
    }

    /// Rows spread mostly along one axis, less along a second, a little along
    /// the rest: the two directions come back in that order, signed.
    #[test]
    fn the_strongest_directions_come_out_in_order() {
        let dimensions = 6;
        let mut rows = Vec::new();
        for i in 0..300 {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let mut row = vec![0.0; dimensions];
            match i % 3 {
                0 => row[2] = -5.0 * sign,
                1 => row[4] = 2.0 * sign,
                _ => row[0] = 0.3 * sign,
            }
            rows.push(row);
        }
        let found = strongest_directions(&rows, dimensions, 2);
        assert!((found[0][2] - 1.0).abs() < 1e-4, "{:?}", found[0]);
        assert!((found[1][4] - 1.0).abs() < 1e-4, "{:?}", found[1]);
        let dot: f32 = found[0].iter().zip(&found[1]).map(|(a, b)| a * b).sum();
        assert!(dot.abs() < 1e-5);
    }

    /// What is removed is gone - the vector has nothing left along an axis -
    /// and what is left has unit length.
    #[test]
    fn a_vector_loses_its_language_and_keeps_its_length() {
        let axes = LanguageAxes {
            set: String::new(),
            recipe: String::new(),
            axes: vec![vec![1.0, 0.0, 0.0], vec![0.0, 0.6, 0.8]],
        };
        let mut v = vec![0.5, 0.5, 0.5];
        normalize(&mut v);
        axes.remove(&mut v);
        for axis in &axes.axes {
            let along: f32 = v.iter().zip(axis).map(|(a, b)| a * b).sum();
            assert!(along.abs() < 1e-6, "{along}");
        }
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-6);
        assert_ne!(
            axes.space(),
            LanguageAxes {
                axes: vec![vec![1.0, 0.0, 0.0]],
                ..axes.clone()
            }
            .space()
        );
    }
}
