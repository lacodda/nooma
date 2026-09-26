//! The embedding models nooma knows, pinned to the byte.
//!
//! A model is not a name. `intfloat/multilingual-e5-small` on the Hub is a
//! repository whose files can be replaced by a new commit at any time, and a
//! vector computed by one set of weights means nothing next to a vector
//! computed by another. So each model here is a commit of its repository and
//! the size and SHA-256 of every file taken from it: what was measured is what
//! is fetched, and a download that differs by one byte is refused.
//!
//! The catalogue is data and opens no connection. Fetching lives in its own
//! crate, `nooma-fetch`, the one place in the product that reaches the
//! network; loading a model reads files from a directory and nothing else.
//!
//! # Only multilingual models
//!
//! A meaningful share of what nooma is pointed at is Russian, and much of the
//! rest mixes Russian and English in one note. An English-only model places a
//! Russian sentence by its punctuation. Every model here was trained on both,
//! and on queries in one language finding documents in the other.

use std::path::{Path, PathBuf};

/// How a model turns the vectors of a passage's tokens into one vector.
///
/// A property of how the model was trained, not a choice: pooling a
/// mean-pooled model by its first token gives a vector that is well-formed,
/// normalized, and wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pooling {
    /// The mean of every token's vector, padding left out.
    Mean,
    /// The vector of the first token.
    Cls,
}

/// One file of a model, as it was pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ModelFile {
    /// The path inside the model's repository, and under its directory here.
    pub path: &'static str,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its SHA-256, in lowercase hex.
    pub sha256: &'static str,
}

/// An embedding model nooma can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ModelSpec {
    /// The name nooma calls it by, and the name of its directory — for the
    /// files and for the vectors computed with it. Never reused for other
    /// weights: a new revision is a new id.
    pub id: &'static str,
    /// The repository on the Hugging Face Hub the files come from.
    pub repository: &'static str,
    /// The commit of that repository the files were taken at.
    pub revision: &'static str,
    /// The license the weights are published under.
    pub license: &'static str,
    /// Every file the model needs, with its size and hash.
    #[serde(skip)]
    pub files: &'static [ModelFile],
    /// Which of the files is the ONNX graph.
    #[serde(skip)]
    pub graph: &'static str,
    /// Files holding weights the graph refers to by name, beside it.
    #[serde(skip)]
    pub weights: &'static [&'static str],
    /// The length of its vectors.
    pub dimensions: usize,
    /// How its token vectors are pooled.
    pub pooling: Pooling,
    /// Put before a query. Models trained with asymmetric prefixes rank
    /// noticeably worse without them, and give no sign of it.
    pub query_prefix: &'static str,
    /// Put before a passage.
    pub passage_prefix: &'static str,
    /// The most tokens of a passage it reads; the rest is cut off.
    pub max_tokens: usize,
}

/// The tokenizer's files, by the names every model here uses for them.
pub const TOKENIZER: &str = "tokenizer.json";
/// The model's configuration, read for the padding token.
pub const CONFIG: &str = "config.json";
/// The special tokens, read for the padding token's text.
pub const SPECIAL_TOKENS: &str = "special_tokens_map.json";
/// The tokenizer's configuration, read for its longest input.
pub const TOKENIZER_CONFIG: &str = "tokenizer_config.json";

/// The model nooma uses.
///
/// Chosen by measurement on a bilingual corpus against the other candidates
/// below; see `docs/adr/0005-embedding-model.md` for the numbers.
pub const DEFAULT_MODEL: &str = "multilingual-e5-small";

/// Every model nooma can run.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "multilingual-e5-small",
        repository: "intfloat/multilingual-e5-small",
        revision: "614241f622f53c4eeff9890bdc4f31cfecc418b3",
        license: "MIT",
        files: &[
            ModelFile {
                path: "onnx/model.onnx",
                bytes: 470_268_510,
                sha256: "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665",
            },
            ModelFile {
                path: TOKENIZER,
                bytes: 17_082_730,
                sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
            },
            ModelFile {
                path: CONFIG,
                bytes: 655,
                sha256: "69137736cab8b8903a07fe8afaafdda25aac55415a12a55d1bffa9f581abf959",
            },
            ModelFile {
                path: SPECIAL_TOKENS,
                bytes: 167,
                sha256: "d05497f1da52c5e09554c0cd874037a083e1dc1b9cfd48034d1c717f1afc07a7",
            },
            ModelFile {
                path: TOKENIZER_CONFIG,
                bytes: 443,
                sha256: "a1d6bc8734a6f635dc158508bef000f8e2e5a759c7d92f984b2c86e5ff53425b",
            },
        ],
        graph: "onnx/model.onnx",
        weights: &[],
        dimensions: 384,
        pooling: Pooling::Mean,
        query_prefix: "query: ",
        passage_prefix: "passage: ",
        max_tokens: 512,
    },
    ModelSpec {
        id: "paraphrase-multilingual-minilm-l12-v2",
        repository: "Xenova/paraphrase-multilingual-MiniLM-L12-v2",
        revision: "2c4055b12046f11709e9df2c122e59ffbdc2f900",
        license: "Apache-2.0",
        files: &[
            ModelFile {
                path: "onnx/model.onnx",
                bytes: 470_268_510,
                sha256: "185ae63f47e17a7e8d30d0e6a3cde6a6e4b79bc5b81666ecffc279a6856ca113",
            },
            ModelFile {
                path: TOKENIZER,
                bytes: 17_082_913,
                sha256: "b60b6b43406a48bf3638526314f3d232d97058bc93472ff2de930d43686fa441",
            },
            ModelFile {
                path: CONFIG,
                bytes: 673,
                sha256: "05b570bff786faa5c4604152aa16f19f77ed6dfc31e47dd0f3dd987078693ac7",
            },
            ModelFile {
                path: SPECIAL_TOKENS,
                bytes: 280,
                sha256: "06e405a36dfe4b9604f484f6a1e619af1a7f7d09e34a8555eb0b77b66318067f",
            },
            ModelFile {
                path: TOKENIZER_CONFIG,
                bytes: 496,
                sha256: "3f5961b9ac86288cccdb97f32fb848d6187c78e1603958c53f3ea1f296b7d8a2",
            },
        ],
        graph: "onnx/model.onnx",
        weights: &[],
        dimensions: 384,
        pooling: Pooling::Mean,
        query_prefix: "",
        passage_prefix: "",
        // The tokenizer would accept 512; the model was trained on 128 and
        // reads past that with the positions it never learned.
        max_tokens: 128,
    },
    ModelSpec {
        id: "bge-m3",
        repository: "BAAI/bge-m3",
        revision: "5617a9f61b028005a4858fdac845db406aefb181",
        license: "MIT",
        files: &[
            ModelFile {
                path: "onnx/model.onnx",
                bytes: 724_923,
                sha256: "f84251230831afb359ab26d9fd37d5936d4d9bb5d1d5410e66442f630f24435b",
            },
            ModelFile {
                path: "onnx/model.onnx_data",
                bytes: 2_266_820_608,
                sha256: "1eebfb28493f67bba03ce0ef64bfdc7fc5a3bd9d7493f818bb1d78cd798416b4",
            },
            ModelFile {
                path: "onnx/Constant_7_attr__value",
                bytes: 65_552,
                sha256: "cdf16f72c5d07b36484056e601ed9687f78477e5d85cee85a34f2406b7fb5906",
            },
            ModelFile {
                path: TOKENIZER,
                bytes: 17_098_108,
                sha256: "21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08",
            },
            ModelFile {
                path: CONFIG,
                bytes: 687,
                sha256: "26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa",
            },
            ModelFile {
                path: SPECIAL_TOKENS,
                bytes: 964,
                sha256: "8c785abebea9ae3257b61681b4e6fd8365ceafde980c21970d001e834cf10835",
            },
            ModelFile {
                path: TOKENIZER_CONFIG,
                bytes: 444,
                sha256: "a62b2b6784f990259fddef5f16388693a8043be4f69179e6a5257eeb3f9abac4",
            },
        ],
        graph: "onnx/model.onnx",
        weights: &["onnx/model.onnx_data", "onnx/Constant_7_attr__value"],
        dimensions: 1024,
        pooling: Pooling::Cls,
        query_prefix: "",
        passage_prefix: "",
        // It reads 8192, and attention costs the square of the length. A
        // chunk is at most 1500 characters, which is well inside this.
        max_tokens: 1024,
    },
];

impl ModelSpec {
    /// The model with this id.
    pub fn find(id: &str) -> Option<&'static ModelSpec> {
        MODELS.iter().find(|spec| spec.id == id)
    }

    /// The model nooma uses.
    pub fn default_model() -> &'static ModelSpec {
        Self::find(DEFAULT_MODEL).expect("the default model is in the catalogue")
    }

    /// Where its files live under a models directory.
    pub fn dir(&self, models: &Path) -> PathBuf {
        models.join(self.id)
    }

    /// Where one of its files lives under a models directory.
    pub fn file_path(&self, models: &Path, file: &ModelFile) -> PathBuf {
        self.dir(models).join(file.path)
    }

    /// Its size on disk, in bytes.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }

    /// The files that are not there, or not the size they were pinned at.
    ///
    /// Sizes rather than hashes: hashing two gigabytes on every start would
    /// cost seconds, and a file the right size but wrong content is what a
    /// fetch checks for. A truncated download - the case that happens - is
    /// caught here.
    pub fn missing(&self, models: &Path) -> Vec<&'static ModelFile> {
        self.files
            .iter()
            .filter(|file| std::fs::metadata(self.file_path(models, file)).map_or(true, |meta| meta.len() != file.bytes))
            .collect()
    }

    /// Delete its files. Returns whether there were any.
    pub fn remove(&self, models: &Path) -> std::io::Result<bool> {
        match std::fs::remove_dir_all(self.dir(models)) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// The text a model reads for a query.
    pub fn query_text(&self, query: &str) -> String {
        format!("{}{query}", self.query_prefix)
    }

    /// The text a model reads for a passage.
    pub fn passage_text(&self, passage: &str) -> String {
        format!("{}{passage}", self.passage_prefix)
    }
}

/// The directory models are kept in, shared by the command line and the
/// window.
pub fn default_models_dir() -> crate::Result<PathBuf> {
    Ok(crate::data_dir()?.join("models"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_whole_and_named_once() {
        let mut ids = std::collections::BTreeSet::new();
        for spec in MODELS {
            assert!(ids.insert(spec.id), "{} appears twice", spec.id);
            let paths: Vec<&str> = spec.files.iter().map(|file| file.path).collect();
            for required in [spec.graph, TOKENIZER, CONFIG, SPECIAL_TOKENS, TOKENIZER_CONFIG]
                .into_iter()
                .chain(spec.weights.iter().copied())
            {
                assert!(paths.contains(&required), "{} does not pin {required}", spec.id);
            }
            for file in spec.files {
                assert_eq!(file.sha256.len(), 64, "{} {}: not a SHA-256", spec.id, file.path);
                assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
                assert!(file.bytes > 0);
            }
            assert!(spec.revision.len() == 40, "{}: a revision is a full commit id", spec.id);
        }
        assert!(ModelSpec::find(DEFAULT_MODEL).is_some());
    }

    #[test]
    fn a_prefix_goes_in_front_of_the_text_it_marks() {
        let e5 = ModelSpec::find("multilingual-e5-small").unwrap();
        assert_eq!(e5.query_text("чайник"), "query: чайник");
        assert_eq!(e5.passage_text("kettle"), "passage: kettle");
        let plain = ModelSpec::find("bge-m3").unwrap();
        assert_eq!(plain.query_text("kettle"), "kettle");
    }

    #[test]
    fn a_file_the_wrong_size_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ModelSpec::find("multilingual-e5-small").unwrap();
        assert_eq!(spec.missing(dir.path()).len(), spec.files.len());
        let config = spec.files.iter().find(|file| file.path == CONFIG).unwrap();
        let path = spec.file_path(dir.path(), config);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b' '; usize::try_from(config.bytes).unwrap() - 1]).unwrap();
        assert!(spec.missing(dir.path()).contains(&config), "one byte short is not the file");
        std::fs::write(&path, vec![b' '; usize::try_from(config.bytes).unwrap()]).unwrap();
        assert!(!spec.missing(dir.path()).contains(&config));
    }
}
