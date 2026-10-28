//! Text in pictures, read on the machine by the OCR that ships with Windows.
//!
//! A screenshot, a photo of a page, a scanned PDF: their text is pixels, and
//! a program that searches or copies text sees none. Windows has read text in
//! pictures since Windows 10, locally and in every language whose recognizer
//! is installed; this crate is that reader with what using it takes around
//! it - the picture decoded and stood up the way its EXIF says, a transparent
//! background put on paper, a PDF page drawn as a picture, the recognizer
//! chosen for the language the text is in, and the lines it returns laid out
//! as paragraphs.
//!
//! Nothing leaves the machine: the recognizers are part of Windows, and the
//! crate has no network code. Elsewhere than Windows it builds, and
//! [`Ocr::new`] says there is no OCR here.
//!
//! ```no_run
//! let ocr = lacodda_ocr::Ocr::new()?;
//! let text = ocr.read(&std::fs::read("receipt.jpg")?)?;
//! println!("{}", text.plain());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#[cfg_attr(not(windows), allow(dead_code))]
mod pixels;
#[cfg(windows)]
mod speller;
#[cfg(windows)]
mod windows;

/// The crate's version, which says what reads a picture as much as the
/// recognizers do: a program that keeps text read out of pictures keeps this
/// beside it, and reads them again when it changes.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why a picture was not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// There is no OCR on this machine: not Windows, or no recognizer
    /// installed for any language. Said for a person, with what to do.
    #[error("{0}")]
    Unavailable(String),
    /// The bytes are not a picture or a PDF that Windows can decode.
    #[error("{0}")]
    Unreadable(String),
    /// Windows failed while reading it.
    #[error("{0}")]
    Failed(String),
}

/// The text read out of one picture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Text {
    /// The language of the recognizer that read it, as a BCP-47 tag: `ru`,
    /// `en-US`. Empty when no recognizer was asked.
    pub language: String,
    /// How far the text is turned from the horizontal, in degrees, when
    /// Windows could tell.
    pub angle: Option<f64>,
    /// The lines, in the order they are read.
    pub lines: Vec<Line>,
}

/// One line of text, as the recognizer found it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    /// The line's text, written the way its language writes it: words
    /// apart by spaces, or not, as in Chinese.
    pub text: String,
    /// Its words, each where it stands in the picture.
    pub words: Vec<Word>,
}

/// One word and where it stands.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Word {
    /// The word.
    pub text: String,
    /// Where it is, in the pixels of the picture as it was given - after an
    /// EXIF orientation stood it up, before any scaling the reading did.
    pub rect: Rect,
}

/// A rectangle in a picture, in pixels from its top left corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Where it ends downwards.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Where it ends rightwards.
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The smallest rectangle holding both.
    fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }
}

impl Line {
    /// Where the whole line is: the rectangle around its words.
    pub fn rect(&self) -> Option<Rect> {
        self.words.iter().map(|word| word.rect).reduce(Rect::union)
    }
}

impl Text {
    /// Whether nothing was read: a photo with no words in it, a blank page.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.text.trim().is_empty())
    }

    /// The text as plain text: a line per line, and a blank line where one
    /// paragraph ends and the next begins.
    ///
    /// The recognizer returns lines and nothing about paragraphs. A
    /// paragraph ends where the next line stands further below than lines of
    /// this size stand apart, or does not stand below at all - the top of
    /// the next column, a panel beside this one.
    pub fn plain(&self) -> String {
        let lines: Vec<(&str, Option<Rect>)> = self
            .lines
            .iter()
            .filter(|line| !line.text.trim().is_empty())
            .map(|line| (line.text.trim(), line.rect()))
            .collect();
        let height = median(lines.iter().filter_map(|(_, rect)| rect.map(|rect| rect.height)));
        let mut out = String::new();
        let mut previous: Option<Rect> = None;
        for (text, rect) in lines {
            if !out.is_empty() {
                let apart = match (previous, rect) {
                    (Some(above), Some(below)) => starts_a_paragraph(above, below, height),
                    _ => false,
                };
                out.push_str(if apart { "\n\n" } else { "\n" });
            }
            out.push_str(text);
            previous = rect.or(previous);
        }
        out
    }
}

/// Whether a line below another starts a new paragraph.
///
/// Lines of a paragraph stand under each other with a gap smaller than their
/// height; a paragraph's spacing, or a heading's, is a gap of about a line or
/// more. The height is the page's usual line, not either line's own: a line
/// of capitals and a line with descenders differ by a third.
fn starts_a_paragraph(above: Rect, below: Rect, height: f32) -> bool {
    let went_up = below.y < above.y + above.height / 2.0;
    let beside = below.x >= above.right() || below.right() <= above.x;
    let gap = below.y - above.bottom();
    went_up || beside || gap > PARAGRAPH_GAP * height
}

/// How far below the line above, in lines, a line starts a paragraph.
const PARAGRAPH_GAP: f32 = 0.8;

fn median(values: impl Iterator<Item = f32>) -> f32 {
    let mut values: Vec<f32> = values.filter(|value| value.is_finite()).collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

/// The letters of a reading in words that look like words: no digit inside
/// them, and every letter in the alphabet most of the reading's letters are
/// in. What a reading is judged by where no spell checker can judge it: a
/// recognizer for another alphabet reads `Гарантийный` as `rapaHTHiiHb1ii`.
#[cfg_attr(not(windows), allow(dead_code))]
fn clean_letters(text: &str) -> usize {
    let mut by_alphabet = std::collections::BTreeMap::<Alphabet, usize>::new();
    let mut clean = Vec::new();
    for token in text.split_whitespace().map(|token| token.trim_matches(|c: char| !c.is_alphanumeric())) {
        if token.is_empty() || token.chars().any(char::is_numeric) {
            continue;
        }
        let mut alphabets = token.chars().filter(|c| c.is_alphabetic()).map(alphabet);
        let Some(first) = alphabets.next() else { continue };
        if alphabets.all(|other| other == first) {
            let letters = token.chars().filter(|c| c.is_alphabetic()).count();
            *by_alphabet.entry(first).or_default() += letters;
            clean.push((first, letters));
        }
    }
    let Some((main, _)) = by_alphabet.iter().max_by_key(|(_, letters)| **letters) else {
        return 0;
    };
    clean.iter().filter(|(of, _)| of == main).map(|(_, letters)| letters).sum()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Alphabet {
    Latin,
    Greek,
    Cyrillic,
    /// Any other, by the block of 256 code points it is in.
    Other(u32),
}

fn alphabet(c: char) -> Alphabet {
    match c {
        'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{24f}' | '\u{1e00}'..='\u{1eff}' => Alphabet::Latin,
        '\u{370}'..='\u{3ff}' | '\u{1f00}'..='\u{1fff}' => Alphabet::Greek,
        '\u{400}'..='\u{52f}' => Alphabet::Cyrillic,
        other => Alphabet::Other(u32::from(other) >> 8),
    }
}

/// What reads text: the languages it reads in, each by its recognizer.
///
/// Made once and shared: every thread that reads through it reads with
/// recognizers of its own, made the first time it reads, so readings on
/// many threads go on at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ocr {
    languages: Vec<String>,
}

/// The languages this machine has a recognizer for, as BCP-47 tags.
pub fn installed() -> Result<Vec<String>, Error> {
    #[cfg(windows)]
    return windows::installed();
    #[cfg(not(windows))]
    return Err(not_windows());
}

#[cfg(not(windows))]
fn not_windows() -> Error {
    Error::Unavailable("text in pictures is read by Windows, and this is not Windows".to_string())
}

impl Ocr {
    /// Every recognizer installed, those for the languages the person has
    /// chosen in Windows first, in their order: a reading that ties with
    /// another is the earlier one's. A recognizer is installed with its
    /// language, so the two lists are mostly one.
    pub fn new() -> Result<Self, Error> {
        #[cfg(windows)]
        return windows::for_the_user();
        #[cfg(not(windows))]
        return Err(not_windows());
    }

    /// The recognizers for these languages, given as BCP-47 tags, in this
    /// order; a language with no recognizer installed is an error.
    pub fn with_languages(languages: &[&str]) -> Result<Self, Error> {
        #[cfg(windows)]
        return windows::with_languages(languages);
        #[cfg(not(windows))]
        {
            let _ = languages;
            Err(not_windows())
        }
    }

    /// The languages it reads in, in the order it tries them.
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// What reads a picture through this, said in one line: the crate's
    /// version and the languages. Two readings that say the same read the
    /// same picture the same way.
    pub fn reader(&self) -> String {
        format!("lacodda-ocr {VERSION} ({})", self.languages.join(", "))
    }

    /// Read the text in a picture: PNG, JPEG, GIF, BMP, TIFF, ICO, and
    /// whatever else Windows has a decoder for - HEIC and WebP where their
    /// extensions are installed.
    pub fn read(&self, picture: &[u8]) -> Result<Text, Error> {
        #[cfg(windows)]
        return windows::read(self, picture);
        #[cfg(not(windows))]
        {
            let _ = picture;
            Err(not_windows())
        }
    }

    /// Read the text in pixels already decoded: `width` by `height`, four
    /// bytes each in the order blue, green, red, alpha, the alpha not
    /// multiplied in, rows top to bottom with no padding.
    pub fn read_bgra(&self, width: u32, height: u32, bgra: &[u8]) -> Result<Text, Error> {
        #[cfg(windows)]
        return windows::read_bgra(self, width, height, bgra);
        #[cfg(not(windows))]
        {
            let _ = (width, height, bgra);
            Err(not_windows())
        }
    }

    /// Read the text on pages of a PDF, each drawn as a scanner would have
    /// seen it: the pages are 0-based, and the texts come back in their
    /// order. A page that is not in the document is an error.
    pub fn read_pdf_pages(&self, pdf: &[u8], pages: &[u32]) -> Result<Vec<Text>, Error> {
        #[cfg(windows)]
        return windows::read_pdf_pages(self, pdf, pages);
        #[cfg(not(windows))]
        {
            let _ = (pdf, pages);
            Err(not_windows())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: f32, y: f32, width: f32, height: f32) -> Line {
        Line {
            text: text.to_string(),
            words: vec![Word {
                text: text.to_string(),
                rect: Rect { x, y, width, height },
            }],
        }
    }

    #[test]
    fn lines_close_together_are_one_paragraph() {
        let text = Text {
            lines: vec![line("first line", 100.0, 100.0, 800.0, 30.0), line("second line", 100.0, 142.0, 600.0, 26.0)],
            ..Text::default()
        };
        assert_eq!(text.plain(), "first line\nsecond line");
    }

    #[test]
    fn a_gap_of_a_line_starts_a_paragraph() {
        let text = Text {
            lines: vec![
                line("Heading", 100.0, 40.0, 300.0, 40.0),
                line("one", 100.0, 120.0, 800.0, 30.0),
                line("two", 100.0, 162.0, 800.0, 30.0),
                line("three", 100.0, 240.0, 800.0, 30.0),
            ],
            ..Text::default()
        };
        assert_eq!(text.plain(), "Heading\n\none\ntwo\n\nthree");
    }

    #[test]
    fn the_top_of_the_next_column_starts_a_paragraph() {
        let text = Text {
            lines: vec![
                line("left top", 20.0, 20.0, 400.0, 14.0),
                line("left bottom", 20.0, 40.0, 400.0, 14.0),
                line("right top", 660.0, 20.0, 400.0, 14.0),
            ],
            ..Text::default()
        };
        assert_eq!(text.plain(), "left top\nleft bottom\n\nright top");
    }

    #[test]
    fn a_line_with_nothing_on_it_is_not_text() {
        let text = Text {
            lines: vec![line("  ", 0.0, 0.0, 10.0, 10.0)],
            ..Text::default()
        };
        assert!(text.is_empty());
        assert_eq!(text.plain(), "");
        assert!(Text::default().is_empty());
    }
}
