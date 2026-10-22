//! Text out of a Word document, under its headings.
//!
//! The reading is office_oxide's, measured against Word's own files (ADR
//! 0010): it tells a heading by its outline level, its style or its style's
//! name, whatever language Word's interface wrote the style ids in; it reads
//! a revision as it stands, without the deleted text; it reads a text box
//! once, though Word stores it twice. What is done around it here is what a
//! search needs and a Word library does not decide:
//!
//! - A comment is a remark about the document, and a header or a footer is
//!   repeated on every page: neither is the document's text. Nor is the
//!   table of contents Word builds: its lines are the headings again, each
//!   with a page number, and read they would put every heading's words in
//!   one chunk - one that would then answer every question about the
//!   document with its contents.
//! - A footnote is read where it is referenced - after the paragraph that
//!   points at it, under that paragraph's headings - rather than at the end,
//!   where it would sit under the last heading of the document.
//! - A table row is one line, its cells side by side; a list item is a
//!   paragraph of its own.
//! - The title is the file name, as a PDF's is: the title in the document's
//!   properties is as often a template's as the document's own, and it is
//!   kept as another name the document is found by.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use office_oxide::DocumentFormat;
use office_oxide::ir::{DocumentIR, Element, InlineContent, Note};

use crate::document::{Block, Blocks};

/// What read a Word document's text, as the library records it with every
/// one: a new version of the reader reads them all again.
pub fn reader() -> String {
    format!("office_oxide {}", office_oxide::VERSION)
}

/// The text of a Word document, or why it could not be read, said for a
/// person.
pub fn read(bytes: Vec<u8>) -> Result<Blocks, String> {
    crate::reading::guarded("the Word reader", || {
        let doc =
            office_oxide::Document::from_reader(Cursor::new(bytes), DocumentFormat::Docx).map_err(|e| format!("not a Word document that can be read: {e}"))?;
        let title = doc
            .as_docx()
            .and_then(|docx| docx.core_properties.as_ref())
            .and_then(|properties| properties.title.clone());
        let mut blocks = blocks_of(&doc.to_ir());
        blocks.aliases.extend(title);
        Ok(blocks)
    })
}

/// A Word document's elements as blocks, its notes where they are pointed
/// at.
fn blocks_of(ir: &DocumentIR) -> Blocks {
    let mut notes = BTreeMap::new();
    for element in ir.sections.iter().flat_map(|section| &section.elements) {
        match element {
            Element::Footnote(note) => {
                notes.insert((Mark::Footnote, note.id), note);
            }
            Element::Endnote(note) if !is_comment(note) => {
                notes.insert((Mark::Endnote, note.id), note);
            }
            _ => {}
        }
    }
    let mut headings = BTreeSet::new();
    collect_headings(ir.sections.iter().flat_map(|section| &section.elements), &mut headings);
    let mut writer = Writer {
        blocks: Vec::new(),
        notes,
        placed: BTreeSet::new(),
        headings,
    };
    for element in ir.sections.iter().flat_map(|section| &section.elements) {
        if !matches!(element, Element::Footnote(_) | Element::Endnote(_)) {
            writer.element(element);
        }
    }
    // A note nothing points at still holds text: at the end, under no
    // heading, since no heading is its.
    let unplaced: Vec<&Note> = writer
        .notes
        .iter()
        .filter(|(key, _)| !writer.placed.contains(key))
        .map(|(_, note)| *note)
        .collect();
    if !unplaced.is_empty() {
        writer.blocks.push(Block::Part(Vec::new()));
        for note in unplaced {
            writer.elements(&note.content);
        }
    }
    Blocks {
        title: None,
        aliases: Vec::new(),
        blocks: writer.blocks,
    }
}

/// How office_oxide carries a comment: as an endnote labelled `Comment`, or
/// `Comment (author)`.
fn is_comment(note: &Note) -> bool {
    note.marker
        .as_deref()
        .is_some_and(|marker| marker == "Comment" || marker.starts_with("Comment ("))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mark {
    Footnote,
    Endnote,
}

struct Writer<'a> {
    blocks: Vec<Block>,
    notes: BTreeMap<(Mark, u32), &'a Note>,
    /// The notes already read where they were pointed at.
    placed: BTreeSet<(Mark, u32)>,
    /// The document's headings, as one line each: what a line of its
    /// contents repeats.
    headings: BTreeSet<String>,
}

impl<'a> Writer<'a> {
    fn elements(&mut self, elements: &'a [Element]) {
        for element in elements {
            self.element(element);
        }
    }

    fn element(&mut self, element: &'a Element) {
        match element {
            Element::Heading(heading) => {
                let text = line_of(&heading.content);
                if !text.is_empty() {
                    self.blocks.push(Block::Heading(usize::from(heading.level.clamp(1, 6)), text));
                }
                self.notes_of(&heading.content);
            }
            Element::Paragraph(paragraph) => {
                if is_contents_line(&office_oxide::ir::inline_to_text(&paragraph.content), &self.headings) {
                    return;
                }
                self.paragraph(text_of(&paragraph.content));
                self.notes_of(&paragraph.content);
            }
            Element::Table(table) => {
                let mut pointed = Vec::new();
                for row in &table.rows {
                    let cells: Vec<String> = row
                        .cells
                        .iter()
                        .map(|cell| {
                            collect_inline(&cell.content, &mut pointed);
                            flat_text(&cell.content)
                        })
                        .filter(|cell| !cell.is_empty())
                        .collect();
                    self.paragraph(cells.join(" · "));
                }
                for content in pointed {
                    self.notes_of(content);
                }
            }
            Element::List(list) => {
                for item in &list.items {
                    self.elements(&item.content);
                    if let Some(nested) = &item.nested {
                        for item in &nested.items {
                            self.elements(&item.content);
                        }
                    }
                }
            }
            Element::TextBox(text_box) => self.elements(&text_box.content),
            Element::CodeBlock(code) => self.paragraph(code.content.clone()),
            // Notes are read where they are pointed at; the rest - images,
            // breaks, rules, shapes - holds no text.
            _ => {}
        }
    }

    fn paragraph(&mut self, text: String) {
        if !text.is_empty() {
            self.blocks.push(Block::Paragraph(text));
        }
    }

    /// The notes a paragraph points at, after it, each once.
    fn notes_of(&mut self, content: &[InlineContent]) {
        for item in content {
            let key = match item {
                InlineContent::FootnoteRef(mark) => (Mark::Footnote, mark.note_id),
                InlineContent::EndnoteRef(mark) => (Mark::Endnote, mark.note_id),
                _ => continue,
            };
            if let Some(note) = self.notes.get(&key).copied()
                && self.placed.insert(key)
            {
                self.elements(&note.content);
            }
        }
    }
}

/// Every heading among some elements, as one line, to know the contents by.
fn collect_headings<'a>(elements: impl Iterator<Item = &'a Element>, out: &mut BTreeSet<String>) {
    for element in elements {
        match element {
            Element::Heading(heading) => {
                out.insert(line_of(&heading.content));
            }
            Element::TextBox(text_box) => collect_headings(text_box.content.iter(), out),
            _ => {}
        }
    }
}

/// Whether a paragraph is a line of the document's own table of contents:
/// one of its headings, maybe after the heading's number, then a tab and a
/// page number - `Rent\t3`, `1.1\tRent\t3`, `Preface\tiv`.
fn is_contents_line(raw: &str, headings: &BTreeSet<String>) -> bool {
    let Some((entry, page)) = raw.trim_end().rsplit_once('\t') else {
        return false;
    };
    let page = page.trim();
    let roman = |c: char| "ivxlcdmIVXLCDM".contains(c);
    if page.is_empty() || !(page.chars().all(|c| c.is_ascii_digit()) || page.chars().all(roman)) {
        return false;
    }
    let entry = entry.trim();
    let entry = match entry.split_once('\t') {
        // A heading's number: a short token with no space in it.
        Some((number, heading)) if number.chars().count() <= 12 && !number.contains(' ') => heading,
        _ => entry,
    };
    headings.contains(&entry.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The inline content of every paragraph among some elements, to find the
/// notes a table cell points at.
fn collect_inline<'a>(elements: &'a [Element], out: &mut Vec<&'a [InlineContent]>) {
    for element in elements {
        match element {
            Element::Paragraph(paragraph) => out.push(&paragraph.content),
            Element::Heading(heading) => out.push(&heading.content),
            Element::TextBox(text_box) => collect_inline(&text_box.content, out),
            _ => {}
        }
    }
}

/// A paragraph's text, a line break kept as a line, runs of spaces and tabs
/// as one space.
fn text_of(content: &[InlineContent]) -> String {
    office_oxide::ir::inline_to_text(content)
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text on one line: a heading's, or a cell's.
fn line_of(content: &[InlineContent]) -> String {
    office_oxide::ir::inline_to_text(content).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of some elements on one line - what a table cell holds.
fn flat_text(elements: &[Element]) -> String {
    let mut parts = Vec::new();
    for element in elements {
        match element {
            Element::Paragraph(paragraph) => parts.push(line_of(&paragraph.content)),
            Element::Heading(heading) => parts.push(line_of(&heading.content)),
            Element::List(list) => parts.extend(list.items.iter().map(|item| flat_text(&item.content))),
            Element::Table(table) => {
                parts.extend(table.rows.iter().flat_map(|row| row.cells.iter().map(|cell| flat_text(&cell.content))));
            }
            Element::TextBox(text_box) => parts.push(flat_text(&text_box.content)),
            _ => {}
        }
    }
    parts.retain(|part| !part.is_empty());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contents_line_is_a_heading_and_its_page() {
        let headings: BTreeSet<String> = ["Terms", "Rent", "Условия аренды"].into_iter().map(String::from).collect();
        for line in ["Terms\t1", "1.1\tRent\t12", "2\tУсловия аренды\t3", "Rent\tiv", "Rent \t 3 "] {
            assert!(is_contents_line(line, &headings), "{line:?}");
        }
        for line in ["Terms", "Coffee\t3", "Rent\tpaid", "Rent\t", "the Rent\t3", "1250 euros\t3"] {
            assert!(!is_contents_line(line, &headings), "{line:?}");
        }
    }
}
