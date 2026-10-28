//! Text in pictures: a screenshot, a photo of a page, the pages of a scan.
//!
//! Read by the OCR that ships with Windows, through `lacodda-ocr` - the
//! line's crate, which nitid reads screenshots with too (ADR 0011, ADR 0012).
//! Nothing here leaves the machine: Windows reads with the recognizers that
//! are installed, one per language.
//!
//! Where there is no OCR - not Windows, or no recognizer installed - a
//! picture is not offered as a document at all, and a scan is read as it
//! always was, by its name. The reader of a file records whether OCR was
//! there, so the day it is, those files are read again.

use std::sync::OnceLock;

use lacodda_ocr::{Ocr, Text};

static OCR: OnceLock<Result<Ocr, String>> = OnceLock::new();

/// The reader of text in pictures on this machine, or why there is none.
///
/// Asked once a process: a language added in Windows while nooma runs is
/// read in from its next start.
pub fn ocr() -> Result<&'static Ocr, &'static str> {
    OCR.get_or_init(|| Ocr::new().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(String::as_str)
}

/// Whether text in pictures can be read here.
pub fn available() -> bool {
    ocr().is_ok()
}

/// What reads text in pictures here, as the library records it with every
/// file read through it: the crate's version and the languages. Empty where
/// there is no OCR.
pub fn reader() -> String {
    ocr().map(Ocr::reader).unwrap_or_default()
}

/// The text in a picture, as a page: `None` when there is none in it.
pub fn read_picture(bytes: &[u8]) -> Result<Option<String>, String> {
    let ocr = ocr().map_err(str::to_string)?;
    let text = crate::reading::guarded("the OCR", || ocr.read(bytes).map_err(|error| error.to_string()))?;
    Ok((!text.is_empty()).then(|| as_page(&text)))
}

/// The text on pages of a PDF, read as pictures: the pages are 0-based, and
/// a page with nothing read on it is an empty string.
pub fn read_pages(pdf: &[u8], pages: &[u32]) -> Result<Vec<String>, String> {
    let ocr = ocr().map_err(str::to_string)?;
    let texts = crate::reading::guarded("the OCR", || ocr.read_pdf_pages(pdf, pages).map_err(|error| error.to_string()))?;
    Ok(texts.iter().map(as_page).collect())
}

/// Text read out of a picture, as a page a search reads: paragraphs apart,
/// and a word broken across lines whole again.
///
/// A printed page breaks words at the end of a line with a hyphen the
/// typesetter put there, and OCR reads it as written: `обслужи-` and
/// `вания` would be two words no query has. A hyphen ending a line after a
/// letter, before a line starting with a small letter, goes with the break.
fn as_page(text: &Text) -> String {
    let plain = text.plain();
    let mut out = String::with_capacity(plain.len());
    let mut lines = plain.split('\n').peekable();
    while let Some(line) = lines.next() {
        out.push_str(line);
        let Some(next) = lines.peek() else { break };
        let broken = line
            .strip_suffix('-')
            .is_some_and(|before| before.chars().next_back().is_some_and(char::is_alphabetic))
            && next.chars().next().is_some_and(char::is_lowercase);
        if broken {
            out.pop();
        } else {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lacodda_ocr::{Line, Rect, Word};

    fn text(lines: &[&str]) -> Text {
        Text {
            lines: lines
                .iter()
                .enumerate()
                .map(|(index, line)| Line {
                    text: (*line).to_string(),
                    words: vec![Word {
                        text: (*line).to_string(),
                        rect: Rect {
                            x: 0.0,
                            y: index as f32 * 40.0,
                            width: 500.0,
                            height: 30.0,
                        },
                    }],
                })
                .collect(),
            ..Text::default()
        }
    }

    #[test]
    fn a_word_broken_at_the_end_of_a_line_is_whole() {
        assert_eq!(
            as_page(&text(&["условия гарантийного обслужи-", "вания и ремонта"])),
            "условия гарантийного обслуживания и ремонта"
        );
        assert_eq!(as_page(&text(&["the warranty cov-", "ers the gasket"])), "the warranty covers the gasket");
    }

    #[test]
    fn a_dash_that_breaks_no_word_stays() {
        // A dash before a capital, a number, or after a space is no hyphen
        // the typesetter put in.
        assert_eq!(as_page(&text(&["from Monday -", "Friday too"])), "from Monday -\nFriday too");
        assert_eq!(as_page(&text(&["unit 7-", "B"])), "unit 7-\nB");
        assert_eq!(as_page(&text(&["page 12-", "14"])), "page 12-\n14");
        assert_eq!(as_page(&text(&["first line", "second line"])), "first line\nsecond line");
    }
}
