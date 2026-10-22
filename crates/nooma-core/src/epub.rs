//! Text out of an EPUB, in its reading order, under its contents.
//!
//! An EPUB is a zip of XHTML pages, with a package file naming them in the
//! order they are read - the spine - and a table of contents pointing into
//! them. A book is cut the way a PDF is: a page of the spine ends a chunk, as
//! a PDF page does, and the contents stand in for bookmarks - a chunk is
//! under the path of the last contents entry at or before it. An entry points
//! at a page or at an element inside one, so a chapter's sections head their
//! chunks too. The contents are what a reader sees of the book's structure in
//! any reading app, and they name chapters a page's markup often does not; a
//! book without usable contents falls back on its own `h1`..`h6`.
//!
//! What is not the book's text is left out: the page head, scripts and
//! styles, and the marks that point at a footnote. A book locked by DRM is
//! said to be, rather than read: its pages are ciphertext, and indexed they
//! would be noise that matches nothing.
//!
//! Both halves are small, so they are read here rather than through a
//! library: the container is the zip reader the Word reader already brings,
//! and the pages are XHTML, read event by event with the HTML entities a
//! book's pages use.

use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read as _};

use quick_xml::XmlVersion;
use quick_xml::escape::{resolve_html5_entity, resolve_predefined_entity};
use quick_xml::events::{BytesEnd, BytesRef, BytesStart, Event};
use quick_xml::reader::Reader;
use zip::ZipArchive;

use crate::document::{Block, Blocks};

/// The rules a book is read by, as a number. Raised when they change what a
/// book reads as: books are read again, and no other file is.
pub const READER_VERSION: u32 = 1;

/// The largest page of a book read, decompressed. A chapter is kilobytes; an
/// entry that claims hundreds of megabytes is a zip bomb, not a page.
const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;

/// The ciphers a book may apply to its fonts alone - the IDPF's and Adobe's
/// obfuscation, which keeps a font from being lifted out of the book. A book
/// that uses only these reads as any other.
const FONT_OBFUSCATION: [&str; 2] = ["http://www.idpf.org/2008/embedding", "http://ns.adobe.com/pdf/enc#RC"];

/// What read a book's text, as the library records it with every book.
pub fn reader() -> String {
    format!("nooma epub {READER_VERSION}")
}

/// Whether an archive is an EPUB: it says so in its `mimetype` entry, or, if
/// that entry is missing, it has the container file every EPUB has.
pub fn is_epub<R: std::io::Read + std::io::Seek>(zip: &mut ZipArchive<R>) -> bool {
    if let Ok(mut entry) = zip.by_name("mimetype") {
        let mut kind = String::new();
        if entry.size() < 256 && entry.read_to_string(&mut kind).is_ok() {
            return kind.trim() == "application/epub+zip";
        }
    }
    zip.index_for_name("META-INF/container.xml").is_some()
}

/// The text of a book, or why it could not be read, said for a person.
pub fn read(bytes: Vec<u8>) -> Result<Blocks, String> {
    crate::reading::guarded("the EPUB reader", || read_unguarded(bytes))
}

fn read_unguarded(bytes: Vec<u8>) -> Result<Blocks, String> {
    let mut book = Book::open(bytes)?;
    if book.locked()? {
        return Err("protected by DRM".to_string());
    }
    let package_path = book.package_path()?;
    let package = Package::parse(&book.text(&package_path)?, &package_path);
    let contents = book.contents(&package);

    let mut blocks = Vec::new();
    let mut read_any = false;
    let mut failure = None;
    let mut path: Vec<String> = Vec::new();
    for page in &package.spine {
        if !page.media_type.contains("html") {
            continue;
        }
        let text = match book.text(&page.path) {
            Ok(text) => text,
            Err(error) => {
                failure.get_or_insert(error);
                continue;
            }
        };
        let entries = contents.get(&page.path);
        // The last entry that names the page itself is the deepest one, or
        // the one that starts lowest: "Part I" and "Chapter 1" both open it.
        if let Some(entry) = entries.and_then(|entries| entries.iter().rev().find(|entry| entry.fragment.is_none())) {
            path.clone_from(&entry.path);
        }
        blocks.push(Block::Part(path.clone()));
        let anchors: HashMap<&str, &Vec<String>> = entries
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.fragment.as_deref().map(|fragment| (fragment, &entry.path)))
            .collect();
        match read_page(&text, &anchors, contents.is_empty(), &mut blocks, &mut path) {
            Ok(()) => read_any = true,
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(error) = failure.filter(|_| !read_any) {
        return Err(format!("no page of it can be read: {error}"));
    }
    Ok(Blocks {
        title: package.title,
        aliases: package.creators,
        blocks,
    })
}

/// A book's archive, with its entries found by name whatever their case: a
/// book written on Windows names a page `Chapter1.xhtml` and links to it as
/// `chapter1.xhtml`, and reading apps forgive it.
struct Book {
    zip: ZipArchive<Cursor<Vec<u8>>>,
    names: HashMap<String, String>,
}

impl Book {
    fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not an EPUB that can be read: {e}"))?;
        let names = zip.file_names().flatten().map(|name| (name.to_lowercase(), name.into_owned())).collect();
        Ok(Self { zip, names })
    }

    fn bytes(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let name = self.names.get(&path.to_lowercase()).ok_or_else(|| format!("{path} is missing from it"))?;
        let mut entry = self.zip.by_name(name).map_err(|e| format!("{path} cannot be read: {e}"))?;
        if entry.size() > MAX_PAGE_BYTES {
            return Err(format!("{path} is larger than {} MB", MAX_PAGE_BYTES / 1024 / 1024));
        }
        let mut bytes = Vec::new();
        // The size an entry claims is the archive's word; the reading is
        // capped whatever it says.
        entry
            .by_ref()
            .take(MAX_PAGE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{path} cannot be read: {e}"))?;
        if bytes.len() as u64 > MAX_PAGE_BYTES {
            return Err(format!("{path} is larger than {} MB", MAX_PAGE_BYTES / 1024 / 1024));
        }
        Ok(bytes)
    }

    /// An entry as text. A book's XML is UTF-8, or UTF-16 with a byte order
    /// mark; a stray invalid byte is not a reason to lose the chapter.
    fn text(&mut self, path: &str) -> Result<String, String> {
        let bytes = self.bytes(path)?;
        let (encoding, skip) = encoding_rs::Encoding::for_bom(&bytes).unwrap_or((encoding_rs::UTF_8, 0));
        Ok(encoding.decode_without_bom_handling(&bytes[skip..]).0.into_owned())
    }

    /// Whether the book's pages are enciphered: anything but a font, by
    /// anything but font obfuscation.
    fn locked(&mut self) -> Result<bool, String> {
        if !self.names.contains_key("meta-inf/encryption.xml") {
            return Ok(false);
        }
        let text = self.text("META-INF/encryption.xml")?;
        let mut reader = xml_reader(&text);
        loop {
            match reader.read_event() {
                Ok(Event::Start(e) | Event::Empty(e)) if local(&e) == "EncryptionMethod" => {
                    if attribute(&e, "Algorithm").is_some_and(|algorithm| !FONT_OBFUSCATION.contains(&algorithm.as_str())) {
                        return Ok(true);
                    }
                }
                Ok(Event::Eof) => return Ok(false),
                Ok(_) => {}
                // Unreadable, it cannot say the pages are locked; they are
                // read, and a locked page reads as nothing.
                Err(_) => return Ok(false),
            }
        }
    }

    /// Where the package file is: the container names it, and a book whose
    /// container is missing still has one.
    fn package_path(&mut self) -> Result<String, String> {
        if self.names.contains_key("meta-inf/container.xml") {
            let text = self.text("META-INF/container.xml")?;
            let mut reader = xml_reader(&text);
            loop {
                match reader.read_event() {
                    Ok(Event::Start(e) | Event::Empty(e)) if local(&e) == "rootfile" => {
                        if let Some(path) = attribute(&e, "full-path") {
                            return Ok(path);
                        }
                    }
                    Ok(Event::Eof) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        let mut packages: Vec<&String> = self.names.values().filter(|name| name.to_lowercase().ends_with(".opf")).collect();
        packages.sort();
        packages
            .first()
            .map(|name| (*name).clone())
            .ok_or_else(|| "not an EPUB: it has no package file".to_string())
    }

    /// The contents, by the page each entry points at, in the order the
    /// book lists them: from the EPUB 3 navigation document, or else the
    /// EPUB 2 NCX.
    fn contents(&mut self, package: &Package) -> BTreeMap<String, Vec<Entry>> {
        let entries = [package.nav.as_ref().map(|nav| (nav, true)), package.ncx.as_ref().map(|ncx| (ncx, false))]
            .into_iter()
            .flatten()
            .find_map(|(path, is_nav)| {
                let text = self.text(path).ok()?;
                let entries = if is_nav { nav_entries(&text, path) } else { ncx_entries(&text, path) };
                (!entries.is_empty()).then_some(entries)
            })
            .unwrap_or_default();
        let mut by_page: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
        // A book converted from FB2 lists every note in its contents, titled
        // by its mark: `[12]` heads nothing, and its note stays under the
        // notes' own title. A bare number is a chapter's title.
        let listed_note = |title: &String| (title.starts_with('[') && title.ends_with(']') || !title.contains(char::is_alphanumeric)) && is_a_note_mark(title);
        for entry in entries.into_iter().filter(|entry| !entry.path.last().is_some_and(listed_note)) {
            by_page.entry(entry.page.to_lowercase()).or_default().push(entry);
        }
        by_page
    }
}

/// What the package file says: the book's title and authors, the pages in
/// reading order, and where its contents are.
#[derive(Debug, Default)]
struct Package {
    title: Option<String>,
    creators: Vec<String>,
    spine: Vec<Page>,
    nav: Option<String>,
    ncx: Option<String>,
}

#[derive(Debug)]
struct Page {
    /// Its path in the archive, lowercased: what the contents are matched by.
    path: String,
    media_type: String,
}

impl Package {
    fn parse(text: &str, path: &str) -> Self {
        struct Item {
            href: String,
            media_type: String,
            properties: String,
        }
        let mut items: HashMap<String, Item> = HashMap::new();
        let mut spine_ids = Vec::new();
        let mut spine_toc = None;
        let mut package = Self::default();
        let mut capture: Option<&str> = None;
        let mut text_of = String::new();

        let mut reader = xml_reader(text);
        loop {
            let event = match reader.read_event() {
                Ok(Event::Eof) | Err(_) => break,
                Ok(event) => event,
            };
            match event {
                Event::Start(e) if matches!(local(&e).as_str(), "title" | "creator") && capture.is_none() => {
                    capture = Some(if local(&e) == "title" { "title" } else { "creator" });
                    text_of.clear();
                }
                Event::Text(t) if capture.is_some() => text_of.push_str(&t.xml10_content()),
                Event::GeneralRef(r) if capture.is_some() => push_reference(&mut text_of, &r),
                Event::End(e) if capture.is_some_and(|field| local_of_end(&e) == field) => {
                    let value = collapse(&text_of);
                    if !value.is_empty() {
                        match capture {
                            Some("title") if package.title.is_none() => package.title = Some(value),
                            Some("creator") if !package.creators.contains(&value) => package.creators.push(value),
                            _ => {}
                        }
                    }
                    capture = None;
                }
                Event::Start(e) | Event::Empty(e) => match local(&e).as_str() {
                    "item" => {
                        if let (Some(id), Some(href)) = (attribute(&e, "id"), attribute(&e, "href")) {
                            items.insert(
                                id,
                                Item {
                                    href,
                                    media_type: attribute(&e, "media-type").unwrap_or_default(),
                                    properties: attribute(&e, "properties").unwrap_or_default(),
                                },
                            );
                        }
                    }
                    "spine" => spine_toc = attribute(&e, "toc"),
                    "itemref" => {
                        if let Some(id) = attribute(&e, "idref") {
                            spine_ids.push(id);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let dir = parent(path);
        for id in spine_ids {
            if let Some(item) = items.get(&id) {
                package.spine.push(Page {
                    path: resolve(dir, &item.href).0.to_lowercase(),
                    media_type: item.media_type.to_lowercase(),
                });
            }
        }
        package.nav = items
            .values()
            .find(|item| item.properties.split_whitespace().any(|property| property == "nav"))
            .map(|item| resolve(dir, &item.href).0);
        package.ncx = spine_toc
            .and_then(|id| items.get(&id))
            .or_else(|| items.values().find(|item| item.media_type == "application/x-dtbncx+xml"))
            .map(|item| resolve(dir, &item.href).0);
        package
    }
}

/// An entry of the contents: the titles leading to it, outermost first, and
/// where it points.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: Vec<String>,
    page: String,
    fragment: Option<String>,
}

/// The entries of an EPUB 3 navigation document's `toc` list. An item with
/// no link - a part title over its chapters - is a title on the way to the
/// entries under it, and no entry itself.
fn nav_entries(text: &str, nav_path: &str) -> Vec<Entry> {
    let dir = parent(nav_path);
    let mut entries = Vec::new();
    let mut reader = xml_reader(text);
    // Inside the contents' `nav`, how many elements deep, so its end is known.
    let mut in_toc: Option<usize> = None;
    let mut depth = 0usize;
    // The open items: their titles so far, and the link of each.
    let mut items: Vec<(String, Option<String>)> = Vec::new();
    // How deep the element holding an item's title is, while inside it: a
    // title may have markup of its own, `<a><span>1</span> Winter</a>`.
    let mut label: Option<usize> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = local(&e);
                if in_toc.is_none() {
                    if name == "nav" && attribute(&e, "type").is_some_and(|kind| kind.split_whitespace().any(|kind| kind == "toc")) {
                        in_toc = Some(depth);
                    }
                    continue;
                }
                match name.as_str() {
                    "li" => items.push((String::new(), None)),
                    "a" | "span" if !items.is_empty() && label.is_none() => {
                        label = Some(depth);
                        if let (Some(item), Some(href)) = (items.last_mut(), attribute(&e, "href")) {
                            item.1 = Some(href);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = local_of_end(&e);
                if in_toc == Some(depth) {
                    in_toc = None;
                } else if in_toc.is_some() {
                    match name.as_str() {
                        _ if label == Some(depth) => {
                            label = None;
                            let path: Vec<String> = items.iter().map(|(title, _)| collapse(title)).collect();
                            if let Some(href) = items.last().and_then(|(_, href)| href.as_deref())
                                && path.iter().all(|title| !title.is_empty())
                            {
                                let (page, fragment) = resolve(dir, href);
                                entries.push(Entry { path, page, fragment });
                            }
                        }
                        "li" => {
                            items.pop();
                        }
                        _ => {}
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Text(t)) if label.is_some() => {
                if let Some(item) = items.last_mut() {
                    item.0.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) if label.is_some() => {
                if let Some(item) = items.last_mut() {
                    push_reference(&mut item.0, &r);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    entries
}

/// The entries of an EPUB 2 NCX: navigation points, nested.
fn ncx_entries(text: &str, ncx_path: &str) -> Vec<Entry> {
    let dir = parent(ncx_path);
    let mut entries = Vec::new();
    let mut reader = xml_reader(text);
    let mut points: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local(&e).as_str() {
                "navPoint" => points.push((String::new(), None, false)),
                "text" if !points.is_empty() => in_text = true,
                "content" => set_source(&mut points, &e),
                _ => {}
            },
            Ok(Event::Empty(e)) if local(&e) == "content" => set_source(&mut points, &e),
            Ok(Event::End(e)) => match local_of_end(&e).as_str() {
                "text" => in_text = false,
                "navPoint" => {
                    points.pop();
                }
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                if let Some(point) = points.last_mut() {
                    point.0.push_str(&t.xml10_content());
                }
            }
            Ok(Event::GeneralRef(r)) if in_text => {
                if let Some(point) = points.last_mut() {
                    push_reference(&mut point.0, &r);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
        // A point is an entry once it has both its title and its target;
        // the title comes first in an NCX.
        if let Some((_, Some(src), emitted @ false)) = points.last_mut() {
            let src = src.clone();
            *emitted = true;
            let path: Vec<String> = points.iter().map(|(title, _, _)| collapse(title)).collect();
            if path.iter().all(|title| !title.is_empty()) {
                let (page, fragment) = resolve(dir, &src);
                entries.push(Entry { path, page, fragment });
            }
        }
    }
    entries
}

fn set_source(points: &mut [(String, Option<String>, bool)], e: &BytesStart) {
    if let (Some(point), Some(src)) = (points.last_mut(), attribute(e, "src")) {
        point.1 = Some(src);
    }
}

/// Elements that hold no text of the book, skipped with everything in them.
const SKIPPED: [&str; 5] = ["head", "script", "style", "template", "svg"];

/// Elements a paragraph ends at - starting or ending one - in a page's
/// markup.
const BLOCKS: [&str; 30] = [
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "table",
    "tr",
    "thead",
    "tbody",
    "tfoot",
    "blockquote",
    "pre",
    "section",
    "article",
    "aside",
    "header",
    "footer",
    "nav",
    "figure",
    "figcaption",
    "hr",
];

/// Read one page of the spine into blocks. `anchors` are the contents
/// entries pointing inside the page, by the id they point at; `headings`
/// says whether the page's own `h1`..`h6` head its chunks, as they do in a
/// book with no contents. `path` is the contents path in force, carried from
/// page to page.
fn read_page(text: &str, anchors: &HashMap<&str, &Vec<String>>, headings: bool, blocks: &mut Vec<Block>, path: &mut Vec<String>) -> Result<(), String> {
    let mut reader = xml_reader(text);
    let mut page = PageText::default();
    let mut skipping: Option<usize> = None;
    let mut depth = 0usize;
    let mut heading: Option<usize> = None;
    let mut preformatted = 0usize;
    // A link to a place in the book, open: how deep it is, and where in the
    // paragraph its text begins.
    let mut link: Option<(usize, usize)> = None;
    let result = loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) => break Ok(()),
            Ok(event) => event,
            Err(error) => break Err(error.to_string()),
        };
        match event {
            Event::Start(e) => {
                depth += 1;
                if skipping.is_some() {
                    continue;
                }
                let name = local(&e).to_ascii_lowercase();
                if SKIPPED.contains(&name.as_str()) || points_at_a_note(&e) {
                    skipping = Some(depth);
                    continue;
                }
                page.element(&e, anchors, blocks, path);
                if BLOCKS.contains(&name.as_str()) {
                    page.end_paragraph(blocks);
                }
                if name == "pre" {
                    preformatted += 1;
                }
                if headings && let Some(level) = heading_level(&name) {
                    heading = Some(level);
                }
                if matches!(name.as_str(), "td" | "th") && !page.paragraph.trim().is_empty() {
                    page.paragraph.push_str(" · ");
                }
                if name == "br" {
                    page.paragraph.push('\n');
                }
                if name == "a" && link.is_none() && attribute(&e, "href").is_some_and(|href| href.contains('#')) {
                    link = Some((depth, page.paragraph.len()));
                }
            }
            Event::Empty(e) => {
                if skipping.is_some() {
                    continue;
                }
                page.element(&e, anchors, blocks, path);
                match local(&e).to_ascii_lowercase().as_str() {
                    "br" => page.paragraph.push('\n'),
                    "hr" => page.end_paragraph(blocks),
                    _ => {}
                }
            }
            Event::End(e) => {
                let ends = depth;
                depth = depth.saturating_sub(1);
                if let Some(start) = skipping {
                    if ends <= start {
                        skipping = None;
                    }
                    continue;
                }
                let name = local_of_end(&e).to_ascii_lowercase();
                if name == "pre" {
                    preformatted = preformatted.saturating_sub(1);
                }
                if let Some((_, start)) = link.filter(|(at, _)| *at == ends) {
                    link = None;
                    if page.paragraph.get(start..).is_some_and(is_a_note_mark) {
                        page.paragraph.truncate(start);
                    }
                }
                if heading.is_some() && heading_level(&name).is_some() {
                    let level = heading.take().unwrap_or(1);
                    let text = collapse(&std::mem::take(&mut page.paragraph));
                    if !text.is_empty() {
                        blocks.push(Block::Heading(level, text));
                    }
                } else if BLOCKS.contains(&name.as_str()) {
                    page.end_paragraph(blocks);
                }
            }
            Event::Text(t) if skipping.is_none() => page.push_text(&t.xml10_content(), preformatted > 0),
            Event::CData(t) if skipping.is_none() => page.push_text(&t.into_inner(), preformatted > 0),
            Event::GeneralRef(r) if skipping.is_none() => push_reference(&mut page.paragraph, &r),
            _ => {}
        }
    };
    page.end_paragraph(blocks);
    result
}

/// The text of the paragraph a page is in the middle of.
#[derive(Default)]
struct PageText {
    paragraph: String,
}

impl PageText {
    /// An element starting: if the contents point at it, it starts their
    /// entry's part of the book.
    fn element(&mut self, e: &BytesStart, anchors: &HashMap<&str, &Vec<String>>, blocks: &mut Vec<Block>, path: &mut Vec<String>) {
        if let Some(entry) = attribute(e, "id").and_then(|id| anchors.get(id.as_str()).copied()) {
            self.end_paragraph(blocks);
            path.clone_from(entry);
            blocks.push(Block::Part(entry.clone()));
        }
    }

    fn push_text(&mut self, text: &str, preformatted: bool) {
        if preformatted {
            self.paragraph.push_str(text);
            return;
        }
        // Markup's whitespace is layout: any run of it is one space.
        let mut last_space = self.paragraph.ends_with([' ', '\n']) || self.paragraph.is_empty();
        for c in text.chars() {
            if c.is_whitespace() {
                if !last_space {
                    self.paragraph.push(' ');
                    last_space = true;
                }
            } else {
                self.paragraph.push(c);
                last_space = false;
            }
        }
    }

    fn end_paragraph(&mut self, blocks: &mut Vec<Block>) {
        let text = std::mem::take(&mut self.paragraph);
        let lines: Vec<String> = text.lines().map(collapse).filter(|line| !line.is_empty()).collect();
        if !lines.is_empty() {
            blocks.push(Block::Paragraph(lines.join("\n")));
        }
    }
}

/// Whether a link's text is the mark of a note: a number of up to three
/// digits, or a sign - `1`, `[12]`, `*`, `†`. Books mark their notes so
/// whatever made them; this is what every maker's marks have in common.
fn is_a_note_mark(text: &str) -> bool {
    let mark = text.trim().trim_matches(|c: char| matches!(c, '[' | ']' | '(' | ')'));
    let digits = !mark.is_empty() && mark.len() <= 3 && mark.chars().all(|c| c.is_ascii_digit());
    let signs = !mark.is_empty() && mark.chars().all(|c| matches!(c, '*' | '†' | '‡' | '§'));
    digits || signs
}

/// Whether an element is the mark pointing at a footnote, as the book says
/// it is: its number is not the text, and glued to the word before it, it
/// would make a word that is not there.
fn points_at_a_note(e: &BytesStart) -> bool {
    let says = |name: &str, word: &str| attribute(e, name).is_some_and(|value| value.split_whitespace().any(|v| v == word));
    says("type", "noteref") || says("role", "doc-noteref")
}

fn heading_level(name: &str) -> Option<usize> {
    match name {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// A reader forgiving of what books get wrong: an end tag that does not
/// match, or one with no start.
fn xml_reader(text: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_reader(text.as_bytes());
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;
    reader
}

/// An element's name without its namespace prefix.
fn local(e: &BytesStart) -> String {
    local_end(e.local_name().as_ref()).to_string()
}

/// An end tag's name without its namespace prefix.
fn local_of_end(e: &BytesEnd) -> String {
    local_end(e.local_name().as_ref()).to_string()
}

fn local_end(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// An attribute's value by its name without a prefix - `epub:type` is
/// `type` - with its entities resolved.
fn attribute(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().with_checks(false).flatten().find_map(|attribute| {
        (local_end(attribute.key.as_ref()) == name).then(|| match attribute.normalized_value_with(XmlVersion::Implicit1_0, 16, resolve_entity) {
            Ok(value) => value.into_owned(),
            Err(_) => attribute.value.into_owned(),
        })
    })
}

fn resolve_entity(name: &str) -> Option<&'static str> {
    resolve_predefined_entity(name).or_else(|| resolve_html5_entity(name))
}

/// A character or entity reference, as the text it stands for; one no table
/// knows is left out.
fn push_reference(out: &mut String, reference: &BytesRef) {
    match reference.resolve_char_ref() {
        Ok(Some(c)) => out.push(c),
        _ => {
            if let Some(text) = resolve_entity(reference) {
                out.push_str(text);
            }
        }
    }
}

/// Runs of whitespace as one space, and none at either end.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The directory a path in the archive is in, with its trailing `/`.
fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |slash| &path[..=slash])
}

/// A link from a file in `dir` as a path in the archive and a fragment:
/// relative segments resolved and percent escapes decoded.
fn resolve(dir: &str, href: &str) -> (String, Option<String>) {
    let (file, fragment) = match href.split_once('#') {
        Some((file, fragment)) => (file, Some(percent_decoded(fragment)).filter(|f| !f.is_empty())),
        None => (href, None),
    };
    let mut segments: Vec<String> = dir.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
    for segment in percent_decoded(file).split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment.to_string()),
        }
    }
    (segments.join("/"), fragment)
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = bytes
                .get(i + 1..i + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_is_resolved_against_its_file() {
        assert_eq!(resolve("OEBPS/text/", "../images/a%20b.png"), ("OEBPS/images/a b.png".to_string(), None));
        assert_eq!(
            resolve("EPUB/", "text/ch001.xhtml#%D0%BB%D0%B5%D1%82"),
            ("EPUB/text/ch001.xhtml".to_string(), Some("лет".to_string()))
        );
        assert_eq!(resolve("", "./ch.xhtml#"), ("ch.xhtml".to_string(), None));
    }

    fn page(markup: &str, headings: bool) -> Vec<Block> {
        let mut blocks = Vec::new();
        read_page(markup, &HashMap::new(), headings, &mut blocks, &mut Vec::new()).unwrap();
        blocks
    }

    fn paragraph(text: &str) -> Block {
        Block::Paragraph(text.to_string())
    }

    #[test]
    fn markup_s_whitespace_is_layout_and_a_break_is_a_line() {
        let blocks = page("<html><body><p>one\n   two <b>three</b>four<br/>five</p><div>six</div></body></html>", false);
        assert_eq!(blocks, vec![paragraph("one two threefour\nfive"), paragraph("six")]);
    }

    #[test]
    fn the_head_scripts_and_note_marks_are_not_text() {
        let blocks = page(
            r##"<html><head><title>ch001.xhtml</title><style>p{}</style></head><body>
            <p>the log.<a href="#fn1" epub:type="noteref">1</a> More<a role="doc-noteref" href="#fn2"><sup>2</sup></a></p>
            <script>var x = 1;</script></body></html>"##,
            false,
        );
        assert_eq!(blocks, vec![paragraph("the log. More")]);
    }

    #[test]
    fn a_note_listed_in_the_contents_heads_nothing_and_a_numbered_chapter_does() {
        let ncx = r#"<ncx><navMap>
            <navPoint><navLabel><text>1</text></navLabel><content src="ch1.xhtml"/></navPoint>
            <navPoint><navLabel><text>Notes</text></navLabel><content src="notes.xhtml"/>
              <navPoint><navLabel><text>[1]</text></navLabel><content src="notes.xhtml#n1"/></navPoint>
              <navPoint><navLabel><text>*</text></navLabel><content src="notes.xhtml#n2"/></navPoint>
            </navPoint></navMap></ncx>"#;
        let package = Package {
            ncx: Some("toc.ncx".to_string()),
            ..Package::default()
        };
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("toc.ncx", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, ncx.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let contents = Book::open(bytes).unwrap().contents(&package);
        let titles: Vec<Vec<String>> = contents.values().flatten().map(|entry| entry.path.clone()).collect();
        assert_eq!(titles, vec![vec!["1".to_string()], vec!["Notes".to_string()]]);
    }

    #[test]
    fn a_link_that_is_only_a_note_s_number_is_not_text() {
        let blocks = page(
            r##"<body><p>the log.<a href="#fn1" class="footnote-ref"><sup>1</sup></a> Notes [<a href="notes.xhtml#n12">12</a>]
            and <a href="#chapter2">chapter 2</a> and <a href="other.xhtml">3</a><a href="#n">†</a>.</p></body>"##,
            false,
        );
        assert_eq!(blocks, vec![paragraph("the log. Notes [] and chapter 2 and 3.")]);
    }

    #[test]
    fn html_entities_are_the_characters_they_name() {
        let blocks = page(
            "<body><p>tea&nbsp;&mdash; &laquo;chai&raquo; &#1095;&#x430;&iacute; &amp; &unknown;</p></body>",
            false,
        );
        // A no-break space is a space, as any other run of whitespace is.
        assert_eq!(blocks, vec![paragraph("tea — «chai» ча\u{ed} &")]);
    }

    #[test]
    fn a_table_row_is_a_line_of_its_cells() {
        let blocks = page(
            "<body><table><tr><th>Item</th><th>Amount</th></tr><tr><td>Diesel</td><td>400 litres</td></tr></table></body>",
            false,
        );
        assert_eq!(blocks, vec![paragraph("Item · Amount"), paragraph("Diesel · 400 litres")]);
    }

    #[test]
    fn headings_head_chunks_only_in_a_book_without_contents() {
        let markup = "<body><h1>Winter</h1><p>lamp</p><h2>Fog</h2><p>horn</p></body>";
        assert_eq!(
            page(markup, true),
            vec![
                Block::Heading(1, "Winter".into()),
                paragraph("lamp"),
                Block::Heading(2, "Fog".into()),
                paragraph("horn")
            ]
        );
        assert_eq!(
            page(markup, false),
            vec![paragraph("Winter"), paragraph("lamp"), paragraph("Fog"), paragraph("horn")]
        );
    }

    #[test]
    fn an_element_the_contents_point_at_starts_its_part() {
        let fog = vec!["Winter".to_string(), "Fog".to_string()];
        let anchors = HashMap::from([("fog", &fog)]);
        let mut blocks = Vec::new();
        let mut path = vec!["Winter".to_string()];
        read_page(
            "<body><p>lamp</p><section id=\"fog\"><h2>Fog</h2><p>horn</p></section></body>",
            &anchors,
            false,
            &mut blocks,
            &mut path,
        )
        .unwrap();
        assert_eq!(blocks, vec![paragraph("lamp"), Block::Part(fog.clone()), paragraph("Fog"), paragraph("horn")]);
        assert_eq!(path, fog, "the next page goes on under it");
    }

    #[test]
    fn unmatched_markup_does_not_lose_the_page() {
        let blocks = page("<body><p>first<p>second</div></p><p>third</body>", false);
        assert_eq!(blocks, vec![paragraph("first"), paragraph("second"), paragraph("third")]);
    }

    #[test]
    fn the_contents_of_a_nav_and_of_an_ncx_are_the_same_entries() {
        let nav = r#"<html xmlns:epub="http://www.idpf.org/2007/ops"><body>
            <nav epub:type="landmarks"><ol><li><a href="text/cover.xhtml">Cover</a></li></ol></nav>
            <nav epub:type="toc"><h1>Contents</h1><ol>
              <li><span>Part I</span><ol>
                <li><a href="text/ch1.xhtml"><span>Chapter</span> &amp; one</a><ol><li><a href="text/ch1.xhtml#s1">Section</a></li></ol></li>
              </ol></li>
              <li><a href="text/ch2.xhtml">Chapter two</a></li>
            </ol></nav></body></html>"#;
        let ncx = r#"<ncx><navMap>
            <navPoint id="p"><navLabel><text>Part I</text></navLabel><content src="text/ch1.xhtml"/>
              <navPoint><navLabel><text>Chapter &amp; one</text></navLabel><content src="text/ch1.xhtml"/>
                <navPoint><navLabel><text>Section</text></navLabel><content src="text/ch1.xhtml#s1"/></navPoint>
              </navPoint>
            </navPoint>
            <navPoint><navLabel><text>Chapter two</text></navLabel><content src="text/ch2.xhtml"/></navPoint>
            </navMap></ncx>"#;
        let entry = |path: &[&str], page: &str, fragment: Option<&str>| Entry {
            path: path.iter().map(|title| title.to_string()).collect(),
            page: page.to_string(),
            fragment: fragment.map(str::to_string),
        };
        let from_nav = nav_entries(nav, "EPUB/nav.xhtml");
        assert_eq!(
            from_nav,
            vec![
                entry(&["Part I", "Chapter & one"], "EPUB/text/ch1.xhtml", None),
                entry(&["Part I", "Chapter & one", "Section"], "EPUB/text/ch1.xhtml", Some("s1")),
                entry(&["Chapter two"], "EPUB/text/ch2.xhtml", None),
            ],
            "the landmarks are not the contents, and an unlinked title only leads to its entries"
        );
        let mut from_ncx = ncx_entries(ncx, "EPUB/toc.ncx");
        // An NCX point links where a nav item may not; that is the only
        // difference.
        assert_eq!(from_ncx.remove(0), entry(&["Part I"], "EPUB/text/ch1.xhtml", None));
        assert_eq!(from_ncx, from_nav);
    }
}
