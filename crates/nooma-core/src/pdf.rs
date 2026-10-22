//! Text out of a PDF, page by page.
//!
//! The reading itself is `pdf_oxide`'s, chosen by measuring it against the
//! alternatives on PDFs whose text was known (ADR 0009). What is done around
//! it here is what a search needs and a PDF library does not decide: a file
//! locked by a password is said to be, rather than indexed as empty; the
//! bookmarks are flattened into the paths [`crate::document::read_paged`]
//! heads pages with; and a panic inside the library is a file that could not
//! be read rather than an update that died. PDF has a thousand producers, and
//! one malformed file in a folder of thousands must not cost the other ones.

use pdf_oxide::PdfDocument;
use pdf_oxide::outline::{Destination, OutlineItem};

use crate::document::{Bookmark, Paged};

/// What read a PDF's text: the library and its version.
///
/// Kept with every PDF in the index. A file's bytes do not change when the
/// reader does, and an update that judged by the bytes alone would keep text
/// an older reader got wrong for as long as the file sits unchanged; a PDF
/// read by another reader is read again.
pub fn reader() -> String {
    format!("pdf_oxide {}", pdf_oxide::VERSION)
}

/// The text of a PDF, or why it could not be read, said for a person.
///
/// A page the library fails on reads as a page with no text, so one bad page
/// does not lose a book; a document whose every page fails is not read.
pub fn read(bytes: Vec<u8>) -> Result<Paged, String> {
    crate::reading::guarded("the PDF reader", || read_unguarded(bytes))
}

fn read_unguarded(bytes: Vec<u8>) -> Result<Paged, String> {
    let doc = PdfDocument::from_bytes(bytes).map_err(|e| format!("not a PDF that can be read: {e}"))?;
    // Most locked PDFs are locked against printing or copying, with an empty
    // password to open them; the library opens those itself. One that needs a
    // real password reads as pages of nothing, which is not what it holds.
    if doc.is_encrypted() && !doc.authenticate(b"").unwrap_or(false) {
        return Err("protected by a password".to_string());
    }
    let count = doc.page_count().map_err(|e| format!("its pages cannot be read: {e}"))?;
    let mut pages = Vec::with_capacity(count);
    let mut failure = None;
    for index in 0..count {
        match doc.extract_text(index) {
            Ok(text) => pages.push(text),
            Err(error) => {
                failure.get_or_insert(error);
                pages.push(String::new());
            }
        }
    }
    if let Some(error) = failure.filter(|_| pages.iter().all(String::is_empty)) {
        return Err(format!("no page of it can be read: {error}"));
    }
    Ok(Paged {
        pages,
        bookmarks: bookmarks(&doc),
        title: title(&doc),
    })
}

/// Every bookmark that points at a page, with the titles leading to it. One
/// whose page cannot be told is left out, and those nested in it keep their
/// own.
fn bookmarks(doc: &PdfDocument) -> Vec<Bookmark> {
    let Ok(Some(items)) = doc.get_outline() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    flatten(&items, &mut Vec::new(), &mut out);
    out
}

fn flatten(items: &[OutlineItem], path: &mut Vec<String>, out: &mut Vec<Bookmark>) {
    for item in items {
        let title = item.title.split_whitespace().collect::<Vec<_>>().join(" ");
        // The library's word for a bookmark that has no title.
        let titled = !title.is_empty() && title != "(No Title)";
        if titled {
            path.push(title);
            if let Some(Destination::PageIndex(index)) = item.dest {
                out.push(Bookmark {
                    page: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    path: path.clone(),
                });
            }
        }
        flatten(&item.children, path, out);
        if titled {
            path.pop();
        }
    }
}

/// The title in the document's metadata, decoded as a PDF text string.
fn title(doc: &PdfDocument) -> Option<String> {
    let info = doc.trailer().as_dict()?.get("Info")?;
    let info = doc.resolve_object(info).ok()?;
    let title = doc.resolve_object(info.as_dict()?.get("Title")?).ok()?;
    Some(pdf_oxide::optional_content::decode_pdf_text_string(title.as_string()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdf").join(name)).unwrap()
    }

    #[test]
    fn each_page_is_read_on_its_own() {
        let paged = read(fixture("handbook.pdf")).unwrap();
        assert_eq!(paged.pages.len(), 3);
        assert!(paged.pages[0].contains("Espresso machine warranty"));
        assert!(paged.pages[1].contains("Гарантийный ремонт"));
        assert!(paged.pages[2].contains("steam wand gasket"));
        assert!(!paged.pages[0].contains("Гарантийный"), "a page holds its own text only");
    }

    #[test]
    fn a_word_hyphenated_across_lines_is_whole() {
        // The Russian page is set in a narrow, hyphenated column: these words
        // are broken at the end of a line on the page.
        let paged = read(fixture("handbook.pdf")).unwrap();
        let words: Vec<String> = crate::document::clean_page(&paged.pages[1])
            .split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .collect();
        for word in ["гарантийного", "обслуживания", "производитель", "рекомендует", "ежемесячную"] {
            assert!(words.iter().any(|w| w == word), "{word} is not whole in {:?}", paged.pages[1]);
        }
    }

    #[test]
    fn bookmarks_point_at_their_pages_with_their_parents() {
        let paged = read(fixture("handbook.pdf")).unwrap();
        let bookmarks: Vec<(u32, Vec<&str>)> = paged.bookmarks.iter().map(|b| (b.page, b.path.iter().map(String::as_str).collect())).collect();
        assert_eq!(
            bookmarks,
            vec![
                (1, vec!["Warranty"]),
                (2, vec!["Гарантия"]),
                (2, vec!["Гарантия", "Сроки ремонта"]),
                (3, vec!["Repairs"]),
            ]
        );
        assert_eq!(paged.title.as_deref(), Some("Coffee Machine Handbook"));
    }

    #[test]
    fn a_pdf_locked_only_against_changes_opens_with_the_empty_password() {
        let paged = read(fixture("owner-locked.pdf")).unwrap();
        assert!(paged.pages[1].contains("Гарантийный ремонт"));
        assert_eq!(paged.title.as_deref(), Some("Coffee Machine Handbook"), "the metadata is decrypted too");
    }

    #[test]
    fn a_pdf_that_needs_a_password_says_so() {
        assert_eq!(read(fixture("password.pdf")), Err("protected by a password".to_string()));
    }

    #[test]
    fn a_scan_has_pages_and_no_text() {
        let paged = read(fixture("scan.pdf")).unwrap();
        assert_eq!(paged.pages.len(), 2);
        assert!(paged.is_textless());
    }

    #[test]
    fn a_broken_file_is_a_reason_not_a_crash() {
        assert!(read(b"%PDF-1.7\nnot really".to_vec()).is_err());
        assert!(read(Vec::new()).is_err());
        // A cut-off file may still give some of its pages; what matters is
        // that it returns at all.
        let mut bytes = fixture("handbook.pdf");
        bytes.truncate(bytes.len() / 3);
        let _ = read(bytes);
    }
}
