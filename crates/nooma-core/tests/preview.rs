//! A document read around the place a search found in it: the chunk found,
//! whole, with its neighbours in the document's order and the query's words
//! marked - what the window's preview shows.
//!
//! The notes are synthetic, written here; the PDF is the fixture of
//! `tests/pdf`.

use std::path::{Path, PathBuf};

use nooma_core::{Library, Preview};

struct Folder {
    _dir: tempfile::TempDir,
    notes: PathBuf,
    library: Library,
}

const GREENHOUSE: &str = "\
# Greenhouse

The greenhouse stands at the end of the garden.

## Vents

Open the vents at noon, when the sun is on the glass.

## Watering

Water the tomatoes in the evening, never at noon.
The cucumbers want watering every other day.

## Heating

The heater goes on when the night is below five degrees.

## Pests

Whitefly: hang the yellow cards by the door.
";

impl Folder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("greenhouse.md"), GREENHOUSE).unwrap();
        let mut library = Library::open_at(dir.path().join("store")).unwrap();
        library.add_source(&notes, Vec::new()).unwrap();
        library.update().unwrap();
        Self { _dir: dir, notes, library }
    }

    fn note(&self) -> PathBuf {
        self.notes.join("greenhouse.md")
    }

    /// The line the chunk holding these words starts on, as a search says.
    fn line_of(&self, words: &str) -> u32 {
        let hit = &self.library.search(words, 1).unwrap().hits[0].hit;
        assert_eq!(hit.path, self.note());
        hit.line
    }

    fn preview(&self, path: &Path, line: u32, query: Option<&str>, before: usize, after: usize) -> Preview {
        let finder = self.library.finder().unwrap().unwrap();
        finder.preview(path, line, query, before, after).unwrap().unwrap()
    }
}

fn marked(preview: &Preview, at: usize) -> Vec<&str> {
    let passage = &preview.passages[at];
    passage.highlights.iter().map(|range| &passage.text[range.clone()]).collect()
}

#[test]
fn the_chunk_found_is_shown_whole_among_its_neighbours() {
    let folder = Folder::new();
    let line = folder.line_of("tomatoes");
    let preview = folder.preview(&folder.note(), line, Some("tomato"), 1, 1);

    assert_eq!(preview.kind, "markdown");
    assert_eq!(preview.title, "Greenhouse");
    let headings: Vec<&str> = preview.passages.iter().map(|passage| passage.headings.last().unwrap().as_str()).collect();
    assert_eq!(headings, ["Vents", "Watering", "Heating"], "in the document's order");
    assert_eq!(preview.found, 1);
    assert_eq!(preview.passages[1].line, line);
    assert_eq!(
        preview.passages[1].text, "Water the tomatoes in the evening, never at noon.\nThe cucumbers want watering every other day.",
        "whole, as the note writes it"
    );
    assert_eq!((preview.before, preview.after), (1, 1), "the opening and the pests are left out");
}

#[test]
fn the_neighbours_stop_at_the_ends_of_the_document() {
    let folder = Folder::new();
    let first = folder.preview(&folder.note(), folder.line_of("garden"), None, 3, 1);
    assert_eq!((first.found, first.before, first.passages.len(), first.after), (0, 0, 2, 3));

    let last = folder.preview(&folder.note(), folder.line_of("whitefly"), None, 2, 3);
    assert_eq!((last.found, last.before, last.passages.len(), last.after), (2, 2, 3, 0));

    let whole = folder.preview(&folder.note(), folder.line_of("heater"), None, 10, 10);
    assert_eq!((whole.found, whole.before, whole.passages.len(), whole.after), (3, 0, 5, 0));
}

#[test]
fn the_words_of_the_query_are_marked_in_every_passage_in_any_form() {
    let folder = Folder::new();
    let preview = folder.preview(&folder.note(), folder.line_of("tomatoes"), Some("the noon watering"), 1, 1);
    // `the` is in nearly every chunk: it says nothing about the query, and
    // marking it would mark the whole note.
    assert_eq!(marked(&preview, 0), ["noon"]);
    assert_eq!(marked(&preview, 1), ["Water", "noon", "watering"]);
    assert!(marked(&preview, 2).is_empty());

    let unmarked = folder.preview(&folder.note(), folder.line_of("tomatoes"), None, 1, 1);
    assert!(
        unmarked.passages.iter().all(|passage| passage.highlights.is_empty()),
        "a pasted passage marks nothing"
    );
}

#[test]
fn a_note_changed_since_the_search_shows_the_chunk_at_or_before_the_line() {
    let folder = Folder::new();
    let line = folder.line_of("tomatoes");
    let preview = folder.preview(&folder.note(), line + 1, None, 0, 0);
    assert_eq!(preview.passages[0].line, line, "a line inside the chunk is the chunk");

    // Written again: the chunks are in another segment of the index, and
    // still in the document's order.
    std::fs::write(folder.note(), GREENHOUSE.replace("## Heating", "## Heating\n\nA new first line.\n\n## Frost")).unwrap();
    folder.library.update().unwrap();
    let preview = folder.preview(&folder.note(), folder.line_of("tomatoes"), None, 0, 10);
    let headings: Vec<&str> = preview.passages.iter().map(|passage| passage.headings.last().unwrap().as_str()).collect();
    assert_eq!(headings, ["Watering", "Heating", "Frost", "Pests"]);
    assert!(preview.passages.windows(2).all(|pair| pair[0].line < pair[1].line));
}

#[test]
fn a_file_the_index_does_not_hold_has_no_preview() {
    let folder = Folder::new();
    let finder = folder.library.finder().unwrap().unwrap();
    assert_eq!(finder.preview(&folder.notes.join("absent.md"), 1, None, 1, 1).unwrap(), None);
}

#[test]
fn a_pdf_is_shown_by_its_pages() {
    let folder = Folder::new();
    let pdf = folder.notes.join("handbook.pdf");
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdf/handbook.pdf"), &pdf).unwrap();
    folder.library.update().unwrap();
    let finder = folder.library.finder().unwrap().unwrap();
    let preview = finder.preview(&pdf, 1, None, 0, 100).unwrap().unwrap();
    assert_eq!(preview.kind, "pdf");
    let pages: Vec<u32> = preview.passages.iter().map(|passage| passage.page.unwrap()).collect();
    assert!(pages.len() > 1 && pages[0] == 1, "{pages:?}");
    assert!(pages.windows(2).all(|pair| pair[0] <= pair[1]), "{pages:?}");
}
