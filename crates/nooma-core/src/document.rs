//! Text documents cut into the pieces a search answers with.
//!
//! A search result that says "this 40-page file mentions it somewhere" has
//! done half its job. What the index stores instead is a *chunk*: a run of
//! paragraphs under one heading, small enough that the fragment shown under a
//! result is the place the answer is, and carrying the headings above it so a
//! reader knows where in the file they have landed.
//!
//! # Where a chunk ends
//!
//! A heading always ends one: two sections are two subjects, and a chunk that
//! straddles them matches a query about either with the text of both. Inside a
//! section, paragraphs are packed together until the next one would push the
//! chunk past [`MAX_CHUNK_CHARS`]; a paragraph is never split unless it is too
//! long on its own, and then at line breaks before anywhere else. A fenced
//! code block is one paragraph however many blank lines it holds — cutting one
//! in half leaves two pieces of code that each mean nothing.
//!
//! # Obsidian
//!
//! A folder of markdown is very often an Obsidian vault, and a vault says more
//! about a note than its text does: frontmatter tags, `[[wikilinks]]` to other
//! notes, and — read from the other side — the notes linking to this one.
//! Those are fields of the [`Document`], not text mixed into the body, so a
//! query can match a note by what it is tagged or by what points at it without
//! the tag list turning up as the fragment.
//!
//! # PDF
//!
//! A PDF has pages where a note has headings, and a reader is told "page 12"
//! rather than "line 340". A page always ends a chunk, so every chunk is on
//! one page and the page a result names is the page the answer is on. Its
//! bookmarks, where it has them, stand in for headings: a chunk carries the
//! path of the last bookmark at or before its page. What is read out of a page
//! is cleaned of what typesetting leaves in it - soft hyphens, ligatures, runs
//! of spaces - before it is cut. The text itself comes from [`crate::pdf`].

use std::path::Path;

use unicode_normalization::UnicodeNormalization as _;

/// How long a chunk may grow, in characters, before the next paragraph starts
/// a new one.
///
/// Sized for the fragment a reader sees and for the vector half that arrives
/// later: a few paragraphs, well under what a small embedding model reads at
/// once.
pub const MAX_CHUNK_CHARS: usize = 1500;

/// What kind of document a file holds, which decides how it is read and cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// Markdown: headings, fences, frontmatter and links mean something.
    Markdown,
    /// Plain text: only blank lines do.
    Text,
    /// PDF: pages, and bookmarks where it has them.
    Pdf,
}

impl DocKind {
    /// The kind a file is by its extension, or `None` for a file this build
    /// does not read.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::Text),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    /// The name this kind goes by in `--json`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Text => "text",
            Self::Pdf => "pdf",
        }
    }

    /// The largest file of this kind that is read.
    ///
    /// A text file of 16 MB is a log or a dump, and reading it would hold up
    /// every file behind it. A PDF that size is an ordinary book: its bytes
    /// are mostly pictures, which are not read, and its text is a fraction of
    /// them.
    pub const fn max_bytes(self) -> u64 {
        match self {
            Self::Markdown | Self::Text => 16 * 1024 * 1024,
            Self::Pdf => 256 * 1024 * 1024,
        }
    }
}

/// A file, read as a document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    /// What the document is called: the frontmatter title, the first
    /// top-level heading, or the file name, whichever comes first.
    pub title: String,
    /// Other names the note answers to, from frontmatter `aliases`.
    pub aliases: Vec<String>,
    /// Tags, lowercased and without the `#`, from frontmatter and from the
    /// text. Each appears once.
    pub tags: Vec<String>,
    /// The notes this one links to, as written inside `[[...]]` — without the
    /// heading and the display text. Each appears once.
    pub links: Vec<String>,
    /// The text, cut into chunks.
    pub chunks: Vec<Chunk>,
}

/// A run of paragraphs under one heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The headings above the chunk, outermost first. Empty before the first
    /// heading and in plain text.
    pub headings: Vec<String>,
    /// The text of the chunk, paragraphs separated by a blank line.
    pub text: String,
    /// The 1-based line the chunk starts on, as an editor counts lines. In a
    /// PDF, the line of the text read out of it, counted through the pages:
    /// where the chunk is, not what a reader is told.
    pub line: u32,
    /// The 1-based page the chunk is on: in a PDF, and nowhere else.
    pub page: Option<u32>,
}

/// A paged document's text, as read out of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paged {
    /// The text of each page, in order; an empty string for a page with no
    /// text on it.
    pub pages: Vec<String>,
    /// The bookmarks, in the order the document lists them.
    pub bookmarks: Vec<Bookmark>,
    /// The title its metadata gives, if any.
    pub title: Option<String>,
}

/// A bookmark: where it points, and the titles from the outermost bookmark
/// down to this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// The 1-based page it points at.
    pub page: u32,
    /// Its title and the titles of the bookmarks it is nested in, outermost
    /// first.
    pub path: Vec<String>,
}

impl Paged {
    /// A paged document from plain text, its pages separated by form feeds -
    /// the way a paged document is written out as text.
    pub fn from_text(text: &str) -> Self {
        Self {
            pages: text.split('\u{c}').map(str::to_string).collect(),
            ..Self::default()
        }
    }

    /// Whether no page holds any text: a scan, until it is read by OCR.
    pub fn is_textless(&self) -> bool {
        self.pages.iter().all(|page| page.trim().is_empty())
    }
}

/// Read a file's text as a document of the given kind.
///
/// `stem` is the file name without its extension, the title of last resort.
/// A PDF's text is its pages separated by form feeds; [`read_paged`] reads
/// one with its bookmarks and title.
pub fn read(kind: DocKind, stem: &str, text: &str) -> Document {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    match kind {
        DocKind::Markdown => read_markdown(stem, text),
        DocKind::Text => read_plain(stem, text),
        DocKind::Pdf => read_paged(stem, &Paged::from_text(text)),
    }
}

/// Read a paged document: each page cut on its own, under the bookmarks that
/// lead to it.
///
/// The title is the file name. A PDF's metadata title is as often the name
/// of the program that wrote it, or of the file it was printed from, as the
/// name of the document; the file name is the one the reader sees in their
/// folder and opens it by. A metadata title that names the document is kept
/// as an alias, so the document is found by it too.
pub fn read_paged(stem: &str, paged: &Paged) -> Document {
    let mut bookmarks: Vec<&Bookmark> = paged.bookmarks.iter().filter(|b| !b.path.is_empty()).collect();
    // By page, keeping the document's order among bookmarks on one page: the
    // last one listed is the deepest, or the one that starts lowest.
    bookmarks.sort_by_key(|bookmark| bookmark.page);

    let mut builder = ChunkBuilder::default();
    let mut line = 0;
    let mut next_bookmark = 0;
    let mut headings: &[String] = &[];
    for (index, page) in paged.pages.iter().enumerate() {
        let number = line_number(index);
        while let Some(bookmark) = bookmarks.get(next_bookmark).filter(|b| b.page <= number) {
            headings = &bookmark.path;
            next_bookmark += 1;
        }
        builder.start_page(number, headings);
        for text in clean_page(page).lines() {
            if text.trim().is_empty() {
                builder.end_paragraph();
            } else {
                builder.push_line(text, line_number(line));
            }
            line += 1;
        }
    }

    let mut document = Document {
        title: stem.to_string(),
        chunks: non_empty(builder.finish(), Some(1)),
        ..Document::default()
    };
    if let Some(title) = paged.title.as_deref().map(str::trim).filter(|title| names_a_document(title, stem)) {
        document.aliases.push(title.to_string());
    }
    document
}

/// Whether a metadata title names the document rather than a file: not
/// empty, not the file name again, and not a file name of its own - what a
/// printer puts there, `Microsoft Word - report.docx`, `page.html`.
fn names_a_document(title: &str, stem: &str) -> bool {
    let looks_like_a_file = title
        .rsplit_once('.')
        .is_some_and(|(name, extension)| !name.is_empty() && (1..=5).contains(&extension.len()) && extension.chars().all(|c| c.is_ascii_alphanumeric()));
    title.chars().any(char::is_alphabetic) && !title.eq_ignore_ascii_case(stem) && !looks_like_a_file
}

/// A page's text as typesetting left it, made into text a search can match.
///
/// A soft hyphen marks where a word was broken across lines; it goes with the
/// line break and the spaces around it, and the word is whole again. What the
/// fonts map to compatibility characters - ligatures, non-breaking spaces,
/// full-width letters - becomes the ordinary characters (NFKC). A run of
/// spaces becomes one: a PDF places words, and the gaps between them come out
/// as spaces, as many as the gap was wide.
pub fn clean_page(page: &str) -> String {
    let mut joined = String::with_capacity(page.len());
    let mut chars = page.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{ad}' {
            while joined.ends_with(char::is_whitespace) {
                joined.pop();
            }
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            continue;
        }
        joined.push(c);
    }
    let normalized: String = joined.nfkc().collect();
    let mut out = String::with_capacity(normalized.len());
    for (index, line) in normalized.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mut words = line.split([' ', '\t']).filter(|word| !word.is_empty());
        if let Some(first) = words.next() {
            out.push_str(first);
            for word in words {
                out.push(' ');
                out.push_str(word);
            }
        }
    }
    out
}

fn read_plain(stem: &str, text: &str) -> Document {
    let mut builder = ChunkBuilder::default();
    for (index, line) in text.lines().enumerate() {
        let number = line_number(index);
        if line.trim().is_empty() {
            builder.end_paragraph();
        } else {
            builder.push_line(line, number);
        }
    }
    let chunks = builder.finish();
    Document {
        title: stem.to_string(),
        chunks: non_empty(chunks, None),
        ..Document::default()
    }
}

fn read_markdown(stem: &str, text: &str) -> Document {
    let lines: Vec<&str> = text.lines().collect();
    let (front, body_start) = frontmatter(&lines);

    let mut document = Document::default();
    // Frontmatter tags are the ones the author meant as the note's
    // classification, so they lead; inline ones found along the way follow.
    let mut tags = front.tags.clone();
    let mut links = Vec::new();
    let mut first_h1 = None;

    let mut builder = ChunkBuilder::default();
    let mut fence: Option<Fence> = None;
    for (index, line) in lines.iter().enumerate().skip(body_start) {
        let number = line_number(index);
        if let Some(open) = fence {
            builder.push_line(line, number);
            if open.closed_by(line) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = Fence::opened_by(line) {
            fence = Some(open);
            builder.push_line(line, number);
            continue;
        }
        if let Some((level, heading)) = heading(line) {
            if level == 1 && first_h1.is_none() && !heading.is_empty() {
                first_h1 = Some(heading.clone());
            }
            collect_links(&heading, &mut links);
            builder.start_section(level, heading);
            continue;
        }
        if line.trim().is_empty() {
            builder.end_paragraph();
            continue;
        }
        let prose = without_code_spans(line);
        collect_tags(&prose, &mut tags);
        collect_links(&prose, &mut links);
        builder.push_line(line, number);
    }

    document.title = front.title.or(first_h1).filter(|title| !title.is_empty()).unwrap_or_else(|| stem.to_string());
    document.aliases = front.aliases;
    document.tags = tags;
    document.links = links;
    document.chunks = non_empty(builder.finish(), None);
    document
}

/// A document with no text at all still gets one chunk, so a note that is
/// only a title and tags - or a scan - can be found by them. `page` is the
/// page it says it is on, in a paged document.
fn non_empty(chunks: Vec<Chunk>, page: Option<u32>) -> Vec<Chunk> {
    if chunks.is_empty() {
        vec![Chunk {
            headings: Vec::new(),
            text: String::new(),
            line: 1,
            page,
        }]
    } else {
        chunks
    }
}

fn line_number(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// Accumulates lines into paragraphs and paragraphs into chunks.
#[derive(Default)]
struct ChunkBuilder {
    headings: Vec<(usize, String)>,
    chunks: Vec<Chunk>,
    /// The chunk being filled: its paragraphs and the line it starts on.
    current: Vec<String>,
    current_line: u32,
    current_len: usize,
    /// The paragraph being filled.
    paragraph: Vec<String>,
    paragraph_line: u32,
    /// The page being read, in a paged document.
    page: Option<u32>,
}

impl ChunkBuilder {
    fn push_line(&mut self, line: &str, number: u32) {
        if self.paragraph.is_empty() {
            self.paragraph_line = number;
        }
        self.paragraph.push(line.trim_end().to_string());
    }

    fn end_paragraph(&mut self) {
        if self.paragraph.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.paragraph);
        let paragraph = lines.join("\n");
        if paragraph.chars().count() <= MAX_CHUNK_CHARS {
            self.add_paragraph(paragraph, self.paragraph_line);
            return;
        }
        // Too long to be a paragraph of a chunk: cut it at line breaks, and a
        // line that is itself too long at spaces.
        let mut line = self.paragraph_line;
        let mut piece = String::new();
        let mut piece_line = line;
        for text in lines {
            for part in split_long(&text) {
                if !piece.is_empty() && piece.chars().count() + part.chars().count() + 1 > MAX_CHUNK_CHARS {
                    self.add_paragraph(std::mem::take(&mut piece), piece_line);
                }
                if piece.is_empty() {
                    piece_line = line;
                } else {
                    piece.push('\n');
                }
                piece.push_str(&part);
            }
            line = line.saturating_add(1);
        }
        if !piece.is_empty() {
            self.add_paragraph(piece, piece_line);
        }
    }

    fn add_paragraph(&mut self, paragraph: String, line: u32) {
        let length = paragraph.chars().count();
        if !self.current.is_empty() && self.current_len + length + 2 > MAX_CHUNK_CHARS {
            self.end_chunk();
        }
        if self.current.is_empty() {
            self.current_line = line;
            self.current_len = 0;
        } else {
            self.current_len += 2;
        }
        self.current_len += length;
        self.current.push(paragraph);
    }

    fn end_chunk(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.current).join("\n\n");
        self.current_len = 0;
        self.chunks.push(Chunk {
            headings: self.headings.iter().map(|(_, text)| text.clone()).collect(),
            text,
            line: self.current_line,
            page: self.page,
        });
    }

    /// A new page ends the chunk before it, as a heading does, and puts the
    /// chunks after it under these headings.
    fn start_page(&mut self, page: u32, headings: &[String]) {
        self.end_paragraph();
        self.end_chunk();
        self.page = Some(page);
        self.headings = headings.iter().enumerate().map(|(level, text)| (level + 1, text.clone())).collect();
    }

    fn start_section(&mut self, level: usize, heading: String) {
        self.end_paragraph();
        self.end_chunk();
        while self.headings.last().is_some_and(|(open, _)| *open >= level) {
            self.headings.pop();
        }
        self.headings.push((level, heading));
    }

    fn finish(mut self) -> Vec<Chunk> {
        self.end_paragraph();
        self.end_chunk();
        self.chunks
    }
}

/// Cut a line longer than a chunk at spaces; a line with no space to cut at is
/// cut at the limit, on a character boundary.
fn split_long(line: &str) -> Vec<String> {
    if line.chars().count() <= MAX_CHUNK_CHARS {
        return vec![line.to_string()];
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut part_len = 0;
    for word in line.split_inclusive(' ') {
        let word_len = word.chars().count();
        if part_len + word_len > MAX_CHUNK_CHARS && !part.is_empty() {
            parts.push(std::mem::take(&mut part).trim_end().to_string());
            part_len = 0;
        }
        if word_len > MAX_CHUNK_CHARS {
            let chars: Vec<char> = word.chars().collect();
            for piece in chars.chunks(MAX_CHUNK_CHARS) {
                parts.push(piece.iter().collect());
            }
            continue;
        }
        part.push_str(word);
        part_len += word_len;
    }
    if !part.trim().is_empty() {
        parts.push(part.trim_end().to_string());
    }
    parts
}

/// A fenced code block's opening: which character and how many of it, since
/// only a fence of the same kind and at least the same length closes it.
#[derive(Debug, Clone, Copy)]
struct Fence {
    marker: char,
    length: usize,
}

impl Fence {
    fn opened_by(line: &str) -> Option<Self> {
        let trimmed = indent_up_to_three(line)?;
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
        let length = trimmed.chars().take_while(|c| *c == marker).count();
        // A backtick fence's info string cannot hold a backtick, or it would
        // be an inline code span on one line.
        let info = &trimmed[length..];
        (length >= 3 && !(marker == '`' && info.contains('`'))).then_some(Self { marker, length })
    }

    fn closed_by(self, line: &str) -> bool {
        let Some(trimmed) = indent_up_to_three(line) else {
            return false;
        };
        let length = trimmed.chars().take_while(|c| *c == self.marker).count();
        length >= self.length && trimmed[length..].trim().is_empty()
    }
}

/// The line without up to three spaces of indentation, or `None` if it is
/// indented further — four spaces make an indented code block, not a fence or
/// a heading.
fn indent_up_to_three(line: &str) -> Option<&str> {
    let spaces = line.chars().take_while(|c| *c == ' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}

/// An ATX heading: its level and its text, without the closing hashes.
fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = indent_up_to_three(line)?;
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let text = rest.trim();
    // "## Title ##" closes with hashes; "## C#" does not, because the closing
    // run has to stand apart from the text.
    let text = match text.trim_end_matches('#') {
        stripped if stripped.len() < text.len() && (stripped.is_empty() || stripped.ends_with(' ')) => stripped.trim_end(),
        _ => text,
    };
    Some((level, text.to_string()))
}

/// What the frontmatter says, and the index of the first line after it.
#[derive(Debug, Default)]
struct Frontmatter {
    title: Option<String>,
    tags: Vec<String>,
    aliases: Vec<String>,
}

/// Read the YAML block at the very top of a note, if there is one.
///
/// Only three keys matter here and they come in three shapes — a scalar, a
/// flow list `[a, b]` and a block list of `- a` lines — so this reads those
/// shapes and nothing else, rather than taking a YAML parser for a handful of
/// lines. A document that opens with `---` and never closes it has no
/// frontmatter; its first line is a thematic break.
fn frontmatter(lines: &[&str]) -> (Frontmatter, usize) {
    let mut front = Frontmatter::default();
    if lines.first().map(|line| line.trim_end()) != Some("---") {
        return (front, 0);
    }
    let Some(end) = lines.iter().skip(1).position(|line| matches!(line.trim_end(), "---" | "...")) else {
        return (front, 0);
    };
    let block = &lines[1..=end];

    let mut key: Option<String> = None;
    for line in block {
        // A block list item, "  - value", belongs to the key above it.
        if let Some(item) = line.trim_start().strip_prefix("- ").or_else(|| (line.trim() == "-").then_some("")) {
            if let Some(key) = &key {
                front.add(key, unquote(item));
            }
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.starts_with([' ', '\t']) {
            continue;
        }
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        key = Some(name.clone());
        if value.is_empty() {
            continue;
        }
        if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            for item in list.split(',') {
                front.add(&name, unquote(item));
            }
        } else if name == "tags" || name == "tag" {
            // Obsidian accepts "tags: a, b" and "tags: a b" alike.
            for item in value.split([',', ' ']) {
                front.add(&name, unquote(item));
            }
        } else {
            front.add(&name, unquote(value));
        }
    }
    (front, end + 2)
}

impl Frontmatter {
    fn add(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        match key {
            "title" => self.title = Some(value.to_string()),
            "tags" | "tag" => {
                let tag = normalize_tag(value);
                if !tag.is_empty() {
                    push_unique(&mut self.tags, tag);
                }
            }
            "aliases" | "alias" => push_unique(&mut self.aliases, value.to_string()),
            _ => {}
        }
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
        .trim()
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('#').to_lowercase()
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// The line with inline code spans blanked out, so `#include` in backticks is
/// not a tag and a `[[` in code is not a link.
fn without_code_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut inside = false;
    for c in line.chars() {
        if c == '`' {
            inside = !inside;
            out.push(' ');
        } else if inside {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Inline tags: `#word`, starting a word, holding at least one character that
/// is not a digit — `#1` is a number, not a tag, as Obsidian reads it.
fn collect_tags(line: &str, tags: &mut Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts_word = i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '#' | '&' | '/' | '_'));
        if chars[i] == '#' && starts_word {
            let body: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                .collect();
            let body = body.trim_end_matches(['/', '-']);
            if body.chars().any(|c| !c.is_ascii_digit()) {
                push_unique(tags, body.to_lowercase());
            }
            i += 1 + body.chars().count();
        } else {
            i += 1;
        }
    }
}

/// Link targets: `[[target]]`, `[[target|shown]]`, `[[target#heading]]` and
/// `![[embed]]` all name `target`; a link to a heading of the same note names
/// nothing.
fn collect_links(line: &str, links: &mut Vec<String>) {
    let mut rest = line;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            break;
        };
        let inner = &after[..end];
        let target = inner.split('|').next().unwrap_or("");
        let target = target.split('#').next().unwrap_or("").trim();
        if !target.is_empty() && !target.contains('[') {
            push_unique(links, target.to_string());
        }
        rest = &after[end + 2..];
    }
}

/// The key a wikilink resolves by: the last path segment, without a `.md`
/// extension, compared without case. `[[Projects/Plan]]` and `[[plan]]` name
/// the same file, `plan.md`.
pub fn link_key(target: &str) -> String {
    let last = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let last = last.strip_suffix(".md").or_else(|| last.strip_suffix(".MD")).unwrap_or(last);
    last.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(text: &str) -> Document {
        read(DocKind::Markdown, "note", text)
    }

    #[test]
    fn the_kind_comes_from_the_extension_without_case() {
        assert_eq!(DocKind::of(Path::new("a/b.MD")), Some(DocKind::Markdown));
        assert_eq!(DocKind::of(Path::new("a/b.txt")), Some(DocKind::Text));
        assert_eq!(DocKind::of(Path::new("a/b.Pdf")), Some(DocKind::Pdf));
        assert_eq!(DocKind::of(Path::new("a/b.docx")), None);
        assert_eq!(DocKind::of(Path::new("a/README")), None);
    }

    #[test]
    fn a_heading_ends_a_chunk_and_names_the_next() {
        let doc = md("intro text\n\n# One\n\nfirst\n\n## Two\n\nsecond\n\n# Three\n\nthird\n");
        let chunks: Vec<(Vec<&str>, &str, u32)> = doc
            .chunks
            .iter()
            .map(|c| (c.headings.iter().map(String::as_str).collect(), c.text.as_str(), c.line))
            .collect();
        assert_eq!(
            chunks,
            vec![
                (vec![], "intro text", 1),
                (vec!["One"], "first", 5),
                (vec!["One", "Two"], "second", 9),
                (vec!["Three"], "third", 13),
            ]
        );
    }

    #[test]
    fn paragraphs_share_a_chunk_until_it_is_full() {
        let paragraph = "word ".repeat(100);
        let text = std::iter::repeat_n(paragraph.trim(), 6).collect::<Vec<_>>().join("\n\n");
        let doc = read(DocKind::Text, "t", &text);
        assert!(doc.chunks.len() > 1, "six 500-character paragraphs cannot fit one chunk");
        for chunk in &doc.chunks {
            assert!(
                chunk.text.chars().count() <= MAX_CHUNK_CHARS,
                "chunk of {} characters",
                chunk.text.chars().count()
            );
        }
        // Nothing lost or duplicated in the packing.
        let joined: Vec<&str> = doc.chunks.iter().flat_map(|c| c.text.split("\n\n")).collect();
        assert_eq!(joined.len(), 6);
    }

    #[test]
    fn a_paragraph_too_long_on_its_own_is_cut_without_losing_words() {
        let long = "слово ".repeat(1000);
        let doc = read(DocKind::Text, "t", long.trim());
        assert!(doc.chunks.len() >= 4);
        let words: usize = doc.chunks.iter().map(|c| c.text.split_whitespace().count()).sum();
        assert_eq!(words, 1000);
        assert!(doc.chunks.iter().all(|c| c.text.chars().count() <= MAX_CHUNK_CHARS));
    }

    #[test]
    fn a_fence_is_one_paragraph_and_hides_headings_and_tags() {
        let doc = md("# Real\n\n```sh\n# not a heading\n\necho #nottag [[notlink]]\n```\n\nafter #real-tag\n");
        assert_eq!(doc.chunks.len(), 1);
        assert_eq!(doc.chunks[0].headings, vec!["Real"]);
        assert!(doc.chunks[0].text.contains("# not a heading"));
        assert_eq!(doc.tags, vec!["real-tag"]);
        assert!(doc.links.is_empty());
    }

    #[test]
    fn a_longer_fence_is_not_closed_by_a_shorter_one() {
        let doc = md("````\n```\n# inside\n```\n````\n\n# Outside\n\ntext\n");
        let headings: Vec<_> = doc.chunks.iter().map(|c| c.headings.clone()).collect();
        assert_eq!(headings, vec![vec![], vec!["Outside".to_string()]]);
    }

    #[test]
    fn the_title_prefers_frontmatter_then_first_h1_then_the_file_name() {
        assert_eq!(md("---\ntitle: From front\n---\n# From heading\n").title, "From front");
        assert_eq!(md("## Second level\n# From heading\n").title, "From heading");
        assert_eq!(md("no heading\n").title, "note");
        assert_eq!(read(DocKind::Text, "plain", "# not markdown\n").title, "plain");
    }

    #[test]
    fn frontmatter_tags_come_in_three_shapes() {
        assert_eq!(md("---\ntags: [Alpha, \"#beta\"]\n---\n").tags, vec!["alpha", "beta"]);
        assert_eq!(md("---\ntags:\n  - one\n  - Two\n---\n").tags, vec!["one", "two"]);
        assert_eq!(md("---\ntags: x, y z\n---\n").tags, vec!["x", "y", "z"]);
    }

    #[test]
    fn frontmatter_is_not_body_text() {
        let doc = md("---\ntitle: T\naliases: [Other name]\n---\nbody\n");
        assert_eq!(doc.aliases, vec!["Other name"]);
        assert_eq!(doc.chunks.len(), 1);
        assert_eq!(doc.chunks[0].text, "body");
        assert_eq!(doc.chunks[0].line, 5);
    }

    #[test]
    fn an_unclosed_frontmatter_is_a_thematic_break() {
        let doc = md("---\ntitle: nope\nbody\n");
        assert_eq!(doc.title, "note");
        assert!(doc.chunks[0].text.contains("title: nope"));
    }

    #[test]
    fn inline_tags_start_a_word_and_are_not_numbers() {
        let doc = md("see #Project/alpha and #rust_lang, not #123 or a#b or `#code` or url#anchor\n");
        assert_eq!(doc.tags, vec!["project/alpha", "rust_lang"]);
    }

    #[test]
    fn a_tag_in_frontmatter_and_text_appears_once() {
        let doc = md("---\ntags: [dup]\n---\ntext #dup #other\n");
        assert_eq!(doc.tags.iter().filter(|t| *t == "dup").count(), 1);
        assert!(doc.tags.contains(&"other".to_string()));
    }

    #[test]
    fn wikilinks_name_their_target_only() {
        let doc = md("[[Plan]] and [[Folder/Other note#Section|shown]] and ![[image.png]] and [[#local]] and [[Plan]]\n");
        assert_eq!(doc.links, vec!["Plan", "Folder/Other note", "image.png"]);
    }

    #[test]
    fn a_link_key_is_the_file_stem_without_case() {
        assert_eq!(link_key("Folder/Other Note"), "other note");
        assert_eq!(link_key("plan.md"), "plan");
        assert_eq!(link_key("Plan"), "plan");
    }

    #[test]
    fn a_closing_hash_run_must_stand_apart() {
        assert_eq!(heading("## Title ##"), Some((2, "Title".to_string())));
        assert_eq!(heading("## C#"), Some((2, "C#".to_string())));
        assert_eq!(heading("#hashtag"), None);
        assert_eq!(heading("    # indented code"), None);
    }

    #[test]
    fn an_empty_note_still_has_one_chunk_to_be_found_by() {
        let doc = md("---\ntitle: Only a title\ntags: [t]\n---\n");
        assert_eq!(doc.chunks.len(), 1);
        assert!(doc.chunks[0].text.is_empty());
    }

    #[test]
    fn a_byte_order_mark_is_not_text() {
        let doc = md("\u{feff}---\ntitle: T\n---\nbody\n");
        assert_eq!(doc.title, "T");
    }

    fn paged(pages: &[&str]) -> Paged {
        Paged {
            pages: pages.iter().map(|page| page.to_string()).collect(),
            ..Paged::default()
        }
    }

    fn bookmark(page: u32, path: &[&str]) -> Bookmark {
        Bookmark {
            page,
            path: path.iter().map(|title| title.to_string()).collect(),
        }
    }

    #[test]
    fn a_page_ends_a_chunk_and_every_chunk_names_its_page() {
        // Two short pages would fit one chunk; they must not share one, or
        // the second page's answer would be said to be on the first.
        let doc = read_paged("book", &paged(&["first page", "second page\n\nits second paragraph", "", "fourth"]));
        let chunks: Vec<(&str, u32, Option<u32>)> = doc.chunks.iter().map(|c| (c.text.as_str(), c.line, c.page)).collect();
        assert_eq!(
            chunks,
            vec![
                ("first page", 1, Some(1)),
                ("second page\n\nits second paragraph", 2, Some(2)),
                ("fourth", 5, Some(4)),
            ],
            "lines are counted through the pages, and a page with no text has no chunk"
        );
    }

    #[test]
    fn a_long_page_is_cut_and_each_piece_keeps_the_page() {
        let paragraph = "word ".repeat(200);
        let page = [paragraph.trim(), paragraph.trim(), paragraph.trim()].join("\n\n");
        let doc = read_paged("book", &paged(&["intro", &page]));
        assert!(doc.chunks.len() >= 3);
        assert!(doc.chunks[1..].iter().all(|c| c.page == Some(2)));
    }

    #[test]
    fn bookmarks_head_the_pages_from_theirs_to_the_next() {
        let mut book = paged(&["preface", "one", "one more", "two", "two more"]);
        book.bookmarks = vec![
            bookmark(2, &["Part I"]),
            bookmark(2, &["Part I", "Chapter 1"]),
            bookmark(4, &["Part I", "Chapter 2"]),
            // A bookmark whose page could not be told has no path to give.
            bookmark(5, &[]),
        ];
        let doc = read_paged("book", &book);
        let headings: Vec<Vec<&str>> = doc.chunks.iter().map(|c| c.headings.iter().map(String::as_str).collect()).collect();
        assert_eq!(
            headings,
            vec![
                vec![],
                vec!["Part I", "Chapter 1"],
                vec!["Part I", "Chapter 1"],
                vec!["Part I", "Chapter 2"],
                vec!["Part I", "Chapter 2"],
            ]
        );
    }

    #[test]
    fn bookmarks_listed_out_of_page_order_still_head_their_pages() {
        let mut book = paged(&["a", "b", "c"]);
        book.bookmarks = vec![bookmark(3, &["Late"]), bookmark(1, &["Early"])];
        let doc = read_paged("book", &book);
        assert_eq!(doc.chunks[1].headings, vec!["Early"]);
        assert_eq!(doc.chunks[2].headings, vec!["Late"]);
    }

    #[test]
    fn a_soft_hyphen_joins_the_word_it_broke() {
        // As a typesetter leaves it: the hyphen at the end of the line, or -
        // read out in another order - at the start of the next.
        assert_eq!(clean_page("определяе\u{ad}\nмая структура"), "определяемая структура");
        assert_eq!(clean_page("определяе\n\u{ad} мая"), "определяемая");
        assert_eq!(clean_page("по\u{ad}верить"), "поверить");
    }

    #[test]
    fn ligatures_and_wide_gaps_become_plain_text() {
        assert_eq!(clean_page("a \u{fb01}le  with\u{a0}\u{a0}gaps\t\tand\u{fb02}ow  "), "a file with gaps andflow");
        assert_eq!(clean_page("  first\n\n  second  "), "first\n\nsecond");
    }

    #[test]
    fn the_file_name_is_the_title_and_a_real_metadata_title_an_alias() {
        let mut book = paged(&["text"]);
        book.title = Some(" Coffee Machine Handbook ".to_string());
        let doc = read_paged("handbook", &book);
        assert_eq!(doc.title, "handbook");
        assert_eq!(doc.aliases, vec!["Coffee Machine Handbook"]);

        for junk in ["Microsoft Word - report.docx", "page.html", "", "  ", "HANDBOOK", "2025-03"] {
            book.title = Some(junk.to_string());
            assert!(read_paged("handbook", &book).aliases.is_empty(), "{junk:?} names a file, not the document");
        }
    }

    #[test]
    fn a_pdf_with_no_text_is_still_a_document_on_its_first_page() {
        let scan = paged(&["", "  \n "]);
        assert!(scan.is_textless());
        let doc = read_paged("lease", &scan);
        assert_eq!(doc.chunks.len(), 1);
        assert_eq!((doc.chunks[0].text.as_str(), doc.chunks[0].page), ("", Some(1)));
    }

    #[test]
    fn a_pdf_as_text_is_pages_between_form_feeds() {
        let doc = read(DocKind::Pdf, "book", "one\u{c}two");
        let pages: Vec<Option<u32>> = doc.chunks.iter().map(|c| c.page).collect();
        assert_eq!(pages, vec![Some(1), Some(2)]);
        assert!(md("text").chunks.iter().all(|c| c.page.is_none()), "only a paged document has pages");
    }
}
