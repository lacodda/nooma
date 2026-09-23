//! The exact half of the hybrid: a `tantivy` index over document chunks.
//!
//! "Exact" means the words of the query occur in the text — not that they
//! occur letter for letter. A search for *договоры* has to find *договор*, and
//! *warranties* has to find *warranty*, or the half that exists to be reliable
//! misses the document a person knows is there. So every word is stemmed, in
//! Russian or in English, before it is stored and before it is looked up.
//!
//! # Which language a word is in
//!
//! Decided per word, by its alphabet: a word with a Cyrillic letter is
//! Russian, a word with a Latin letter is English, anything else — numbers,
//! codes, other scripts — is kept as written. Deciding per chunk would be the
//! usual shortcut and the wrong one here: a Russian note is full of English
//! names and terms, and stemming those with the Russian rules leaves them
//! unfindable by their own spelling. Per word, both halves of a mixed sentence
//! are right. `ё` is folded to `е` first, since people type either and mean
//! the same letter.
//!
//! This is the lesson `rigger` paid for with SQLite FTS5, which has no Russian
//! stemmer at all: a bare Russian word there finds only its own form.

use std::borrow::Cow;

use tantivy::schema::{FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions};
use tantivy::tokenizer::{LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer, Token, TokenFilter, TokenStream, Tokenizer};

/// The name the analyzer is registered under, and stored in the schema by.
pub const ANALYZER: &str = "nooma";

/// Words longer than this are dropped rather than indexed: a base64 blob or a
/// minified line is not something anyone types into a search field.
const LONGEST_WORD: usize = 48;

/// The analyzer every text field is indexed and queried with.
pub fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(LONGEST_WORD))
        .filter(LowerCaser)
        .filter(ScriptStemmer)
        .build()
}

/// The fields of a chunk in the index.
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    /// The file's absolute path: stored, and the term a file's chunks are
    /// deleted by.
    pub path: Field,
    /// `markdown` or `text`.
    pub kind: Field,
    /// The document's title.
    pub title: Field,
    /// Other names from frontmatter.
    pub aliases: Field,
    /// The headings above the chunk, one value each.
    pub headings: Field,
    /// The chunk's text.
    pub body: Field,
    /// The document's tags, one value each.
    pub tags: Field,
    /// The notes the document links to.
    pub links: Field,
    /// The titles of the notes that link to this document.
    pub backlinks: Field,
    /// How many notes link to this document: a signal, not text.
    pub backlink_count: Field,
    /// The 1-based line the chunk starts on.
    pub line: Field,
}

impl Fields {
    /// The schema, and the fields in it.
    pub fn schema() -> (Schema, Self) {
        let mut builder = Schema::builder();
        let indexed = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(ANALYZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        let stored = indexed.clone().set_stored();
        let fields = Self {
            path: builder.add_text_field("path", STRING | STORED),
            kind: builder.add_text_field("kind", STRING | STORED),
            title: builder.add_text_field("title", stored.clone()),
            aliases: builder.add_text_field("aliases", indexed.clone()),
            headings: builder.add_text_field("headings", stored.clone()),
            body: builder.add_text_field("body", stored.clone()),
            tags: builder.add_text_field("tags", stored),
            links: builder.add_text_field("links", indexed.clone()),
            backlinks: builder.add_text_field("backlinks", indexed),
            backlink_count: builder.add_u64_field("backlink_count", FAST),
            line: builder.add_u64_field("line", STORED | INDEXED),
        };
        (builder.build(), fields)
    }

    /// Look the fields up by name in a schema read back from disk.
    pub fn from_schema(schema: &Schema) -> tantivy::Result<Self> {
        let get = |name: &str| schema.get_field(name);
        Ok(Self {
            path: get("path")?,
            kind: get("kind")?,
            title: get("title")?,
            aliases: get("aliases")?,
            headings: get("headings")?,
            body: get("body")?,
            tags: get("tags")?,
            links: get("links")?,
            backlinks: get("backlinks")?,
            backlink_count: get("backlink_count")?,
            line: get("line")?,
        })
    }

    /// The fields a query is matched against, and how much a match in each
    /// weighs. A word in the title says more about what a document is than
    /// the same word somewhere in its body; a word in a linking note's title
    /// says a little.
    pub fn searched(&self) -> [(Field, f32); 7] {
        [
            (self.title, 3.0),
            (self.aliases, 3.0),
            (self.headings, 2.0),
            (self.tags, 2.0),
            (self.body, 1.0),
            (self.links, 1.0),
            (self.backlinks, 0.5),
        ]
    }
}

/// The alphabet a word is written in, for the purpose of stemming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    Cyrillic,
    Latin,
    Other,
}

fn script_of(word: &str) -> Script {
    let mut latin = false;
    for c in word.chars() {
        if matches!(c, '\u{0400}'..='\u{04FF}') {
            return Script::Cyrillic;
        }
        latin |= c.is_ascii_alphabetic();
    }
    if latin { Script::Latin } else { Script::Other }
}

/// Stems each word by the rules of its own alphabet. Words are expected to be
/// lowercased already.
#[derive(Debug, Clone, Copy)]
pub struct ScriptStemmer;

impl TokenFilter for ScriptStemmer {
    type Tokenizer<T: Tokenizer> = ScriptStemmerFilter<T>;

    fn transform<T: Tokenizer>(self, tokenizer: T) -> ScriptStemmerFilter<T> {
        ScriptStemmerFilter { inner: tokenizer }
    }
}

/// The tokenizer [`ScriptStemmer`] wraps around another.
#[derive(Clone)]
pub struct ScriptStemmerFilter<T> {
    inner: T,
}

impl<T: Tokenizer> Tokenizer for ScriptStemmerFilter<T> {
    type TokenStream<'a> = ScriptStemmerStream<T::TokenStream<'a>>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        ScriptStemmerStream {
            tail: self.inner.token_stream(text),
            russian: rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::Russian),
            english: rust_stemmers::Stemmer::create(rust_stemmers::Algorithm::English),
        }
    }
}

/// The stream [`ScriptStemmerFilter`] produces.
pub struct ScriptStemmerStream<T> {
    tail: T,
    russian: rust_stemmers::Stemmer,
    english: rust_stemmers::Stemmer,
}

impl<T: TokenStream> TokenStream for ScriptStemmerStream<T> {
    fn advance(&mut self) -> bool {
        if !self.tail.advance() {
            return false;
        }
        let token = self.tail.token_mut();
        let stemmed = match script_of(&token.text) {
            Script::Cyrillic => {
                let folded = token.text.replace('ё', "е");
                match self.russian.stem(&folded) {
                    Cow::Borrowed(stem) => stem.to_string(),
                    Cow::Owned(stem) => stem,
                }
            }
            Script::Latin => self.english.stem(&token.text).into_owned(),
            Script::Other => return true,
        };
        token.text = stemmed;
        true
    }

    fn token(&self) -> &Token {
        self.tail.token()
    }

    fn token_mut(&mut self) -> &mut Token {
        self.tail.token_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(text: &str) -> Vec<String> {
        let mut analyzer = analyzer();
        let mut stream = analyzer.token_stream(text);
        let mut out = Vec::new();
        while stream.advance() {
            out.push(stream.token().text.clone());
        }
        out
    }

    #[test]
    fn russian_forms_of_a_word_share_a_stem() {
        assert_eq!(terms("договор"), terms("договоры"));
        assert_eq!(terms("договор"), terms("договоров"));
        assert_eq!(terms("гарантия"), terms("гарантии"));
    }

    #[test]
    fn english_forms_of_a_word_share_a_stem() {
        assert_eq!(terms("warranty"), terms("warranties"));
        assert_eq!(terms("claim"), terms("claimed"));
    }

    /// The reason for deciding per word: an English term inside Russian text
    /// is stemmed as English, and a Russian word beside it as Russian.
    #[test]
    fn a_mixed_sentence_stems_each_word_by_its_alphabet() {
        assert_eq!(terms("гарантии warranties"), [terms("гарантия"), terms("warranty")].concat());
    }

    #[test]
    fn yo_and_ye_are_one_letter() {
        assert_eq!(terms("ёлка"), terms("елка"));
        assert_eq!(terms("Ёжик"), terms("ежик"));
    }

    #[test]
    fn case_does_not_matter() {
        assert_eq!(terms("Договор"), terms("ДОГОВОР"));
        assert_eq!(terms("Espresso"), terms("espresso"));
    }

    #[test]
    fn numbers_and_codes_are_kept_as_written() {
        assert_eq!(terms("INV-2025-0114"), vec!["inv", "2025", "0114"]);
    }

    #[test]
    fn a_blob_is_not_a_word() {
        assert!(terms(&"a".repeat(LONGEST_WORD + 1)).is_empty());
    }
}
