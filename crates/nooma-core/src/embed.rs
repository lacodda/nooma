//! Turning text into vectors.
//!
//! [`Embedder`] is what the vector store and the evaluation ask for, so both
//! can be tested with a stand-in that needs no model on disk. [`OnnxEmbedder`]
//! is the real one: a model from the catalogue, read from its directory and
//! run through ONNX Runtime. It is behind the `semantic` feature; the rest of
//! this module is not.
//!
//! Loading reads files and nothing else. A model that is not on disk is an
//! error naming the command that fetches it, never a download: the one
//! network call the product makes is one the person asked for.

use crate::error::Result;

/// What a text is to the model.
///
/// Some models were trained with a different prefix on each side, and read a
/// query marked as a passage as something else. The caller says which it is;
/// the model's entry in the catalogue says what that means for its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// What a person typed.
    Query,
    /// A piece of a document.
    Passage,
}

/// Something that turns text into vectors.
///
/// Every vector has [`Embedder::dimensions`] components and unit length, so
/// the dot product of two is their cosine.
pub trait Embedder {
    /// The model's id: what its vectors are stored and compared under.
    fn model_id(&self) -> &str;

    /// Everything that decides what vector a text gets: the weights, the
    /// longest input read, the pooling, the prefixes. Two embedders with the
    /// same recipe give the same vector for the same text, and a store of
    /// vectors records the recipe it was filled by - so changing any part of
    /// it starts the store again rather than mixing two kinds of vector.
    fn recipe(&self) -> String {
        self.model_id().to_string()
    }

    /// The length of every vector it returns.
    fn dimensions(&self) -> usize;

    /// One vector per text, in order.
    fn embed(&mut self, texts: &[&str], role: Role) -> Result<Vec<Vec<f32>>>;
}

/// Scale a vector to unit length; a zero vector is left as it is.
pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vector.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(feature = "semantic")]
pub use onnx::OnnxEmbedder;

#[cfg(feature = "semantic")]
mod onnx {
    use std::path::Path;

    use fastembed::{InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel};

    use super::{Embedder, Role, normalize};
    use crate::error::{Error, Result};
    use crate::model::{self, ModelSpec, Pooling};

    /// Passages embedded in one call to the runtime. Large enough to keep the
    /// matrix work efficient, small enough that a batch of long passages does
    /// not ask for gigabytes.
    const BATCH: usize = 32;

    /// A model from the catalogue, loaded and ready to run.
    pub struct OnnxEmbedder {
        spec: &'static ModelSpec,
        model: TextEmbedding,
    }

    impl std::fmt::Debug for OnnxEmbedder {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("OnnxEmbedder").field("model", &self.spec.id).finish_non_exhaustive()
        }
    }

    impl OnnxEmbedder {
        /// Load a model from a models directory.
        ///
        /// `threads` caps the runtime's threads. Embedding shares the machine
        /// with whatever the person is doing, as indexing does.
        pub fn load(spec: &'static ModelSpec, models: &Path, threads: usize) -> Result<Self> {
            let missing = spec.missing(models);
            if !missing.is_empty() {
                return Err(Error::ModelMissing {
                    id: spec.id.to_string(),
                    dir: spec.dir(models),
                    files: missing.iter().map(|file| file.path.to_string()).collect(),
                });
            }
            let read = |path: &str| {
                let full = spec.dir(models).join(path);
                std::fs::read(&full).map_err(|e| Error::io(&full, e))
            };
            let tokenizer = TokenizerFiles {
                tokenizer_file: read(model::TOKENIZER)?,
                config_file: read(model::CONFIG)?,
                special_tokens_map_file: read(model::SPECIAL_TOKENS)?,
                tokenizer_config_file: read(model::TOKENIZER_CONFIG)?,
            };
            let pooling = match spec.pooling {
                Pooling::Mean => fastembed::Pooling::Mean,
                Pooling::Cls => fastembed::Pooling::Cls,
            };
            let mut user = UserDefinedEmbeddingModel::new(read(spec.graph)?, tokenizer).with_pooling(pooling);
            for weights in spec.weights {
                // The graph refers to its weight files by bare name.
                let name = weights.rsplit('/').next().unwrap_or(weights).to_string();
                user = user.with_external_initializer(name, read(weights)?);
            }
            let options = InitOptionsUserDefined::new()
                .with_max_length(spec.max_tokens)
                .with_intra_threads(threads.max(1));
            let model = TextEmbedding::try_new_from_user_defined(user, options).map_err(|e| Error::Embedding(e.to_string()))?;
            Ok(Self { spec, model })
        }

        /// The model's entry in the catalogue.
        pub fn spec(&self) -> &'static ModelSpec {
            self.spec
        }
    }

    impl Embedder for OnnxEmbedder {
        fn model_id(&self) -> &str {
            self.spec.id
        }

        fn recipe(&self) -> String {
            format!(
                "{}@{} max_tokens={} pooling={:?} query={:?} passage={:?}",
                self.spec.repository, self.spec.revision, self.spec.max_tokens, self.spec.pooling, self.spec.query_prefix, self.spec.passage_prefix
            )
        }

        fn dimensions(&self) -> usize {
            self.spec.dimensions
        }

        fn embed(&mut self, texts: &[&str], role: Role) -> Result<Vec<Vec<f32>>> {
            let texts: Vec<String> = texts
                .iter()
                .map(|text| match role {
                    Role::Query => self.spec.query_text(text),
                    Role::Passage => self.spec.passage_text(text),
                })
                .collect();
            let mut vectors = self.model.embed(&texts, Some(BATCH)).map_err(|e| Error::Embedding(e.to_string()))?;
            for vector in &mut vectors {
                if vector.len() != self.spec.dimensions {
                    return Err(Error::Embedding(format!(
                        "{} returned a vector of {} components, not {}",
                        self.spec.id,
                        vector.len(),
                        self.spec.dimensions
                    )));
                }
                // The runtime normalizes already; the store's arithmetic
                // depends on it, so it is not left to a library's default.
                normalize(vector);
            }
            Ok(vectors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normalized_vector_has_unit_length_and_zero_stays_zero() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        let mut zero = vec![0.0; 3];
        normalize(&mut zero);
        assert_eq!(zero, vec![0.0; 3]);
    }
}
