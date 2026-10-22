//! A text file's bytes, as text.
//!
//! Most text is UTF-8 now, and most of what is not does not say what it is:
//! a note saved by an old editor in Windows-1251, a log in UTF-16 without a
//! byte order mark, a letter in KOI8-R. Skipping those loses documents a
//! person knows are there; reading them as UTF-8 indexes gibberish that no
//! query ever matches. So the encoding is told by the bytes, in this order:
//!
//! 1. A byte order mark says it: UTF-8, UTF-16LE or UTF-16BE.
//! 2. Bytes with zeros in them are UTF-16 if they read as text in it, and
//!    not text at all if they do not. No text has a zero byte in UTF-8 or in
//!    a legacy encoding - and UTF-16 of Cyrillic or Latin text, all bytes
//!    below 128, is valid UTF-8, so this comes first.
//! 3. Bytes that are valid UTF-8 are UTF-8 - and so are bytes that are
//!    UTF-8 but for a character cut off at the end or a rare damaged byte,
//!    which no legacy encoding produces by chance.
//! 4. Control characters that text does not use are a binary file.
//! 5. Anything else is a legacy encoding, guessed by the detector Firefox
//!    uses for pages that do not name theirs.
//!
//! A guess is said to be one: the library lists the files whose encoding it
//! guessed, so a wrong one can be seen rather than wondered at.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// What guesses an encoding, as the library records it with every file it
/// guessed for: a new detector may guess otherwise, and those files are read
/// again.
pub fn detector() -> String {
    "chardetng 1".to_string()
}

/// A text, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The text, without a byte order mark.
    pub text: String,
    /// The encoding it was in, by its WHATWG name: `UTF-8`, `UTF-16LE`,
    /// `windows-1251`.
    pub encoding: &'static str,
    /// Whether the encoding was guessed, rather than said by a byte order
    /// mark or proven by the bytes being UTF-8.
    pub guessed: bool,
}

/// Bytes that are not text: what they look like instead, said for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotText(pub &'static str);

/// Decode a file's bytes, or say they are not text.
pub fn decode(bytes: &[u8]) -> Result<Decoded, NotText> {
    if let Some((encoding, length)) = Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[length..]);
        return Ok(decoded(text.into_owned(), encoding, false));
    }
    if bytes.contains(&0) {
        return utf16_without_mark(bytes).ok_or(NotText("binary data, not text"));
    }
    if let Some(text) = as_utf8(bytes) {
        return Ok(text);
    }
    if controls_per_thousand(bytes) > 1 {
        return Err(NotText("binary data, not text"));
    }
    // UTF-8 is ruled out above; what is left is a legacy encoding.
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
    let (text, _) = encoding.decode_without_bom_handling(bytes);
    Ok(decoded(text.into_owned(), encoding, true))
}

fn decoded(text: String, encoding: &'static Encoding, guessed: bool) -> Decoded {
    Decoded {
        text,
        encoding: encoding.name(),
        guessed,
    }
}

/// UTF-8, or UTF-8 that a cut or a stray byte damaged: no more than one
/// broken sequence in a hundred well-formed ones beyond ASCII, which text in
/// a legacy encoding never comes close to - its letters are bytes UTF-8
/// cannot start a sequence with and continue one.
fn as_utf8(bytes: &[u8]) -> Option<Decoded> {
    let error = match std::str::from_utf8(bytes) {
        Ok(text) => return Some(decoded(text.to_string(), UTF_8, false)),
        Err(error) => error,
    };
    // A file cut off in the middle of a character - a log copied while it
    // was written: the text up to it, whole.
    if error.error_len().is_none() {
        let text = std::str::from_utf8(&bytes[..error.valid_up_to()]).ok()?;
        return Some(decoded(text.to_string(), UTF_8, false));
    }
    let (mut good, mut broken) = (0usize, 0usize);
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                good += text.chars().filter(|c| !c.is_ascii()).count();
                break;
            }
            Err(error) => {
                let valid = &rest[..error.valid_up_to()];
                good += std::str::from_utf8(valid).map_or(0, |text| text.chars().filter(|c| !c.is_ascii()).count());
                broken += 1;
                rest = &rest[error.valid_up_to() + error.error_len().unwrap_or(rest.len() - error.valid_up_to())..];
                if broken > good / 100 + 1 {
                    return None;
                }
            }
        }
    }
    (broken * 100 <= good).then(|| decoded(String::from_utf8_lossy(bytes).into_owned(), UTF_8, true))
}

/// UTF-16 with no mark to say so, in whichever byte order reads as text:
/// nearly every character a letter, a digit, a space or a mark of the
/// alphabets and punctuation text is written in. Random bytes read as
/// UTF-16 are mostly ideographs and unpaired surrogates, and fail.
fn utf16_without_mark(bytes: &[u8]) -> Option<Decoded> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    [UTF_16LE, UTF_16BE].into_iter().find_map(|encoding| {
        let (text, malformed) = encoding.decode_without_bom_handling(bytes);
        let total = text.chars().count();
        let textlike = text.chars().filter(|c| reads_as_text(*c)).count();
        (!malformed && total > 0 && textlike * 100 >= total * 98).then(|| decoded(text.into_owned(), encoding, true))
    })
}

fn reads_as_text(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{c}' | ' '..='~')
        || matches!(c as u32,
            0xA0..=0x2FF // Latin
            | 0x370..=0x58F // Greek, Cyrillic, Armenian
            | 0x590..=0x6FF // Hebrew, Arabic
            | 0x2000..=0x206F // punctuation
            | 0x20A0..=0x20CF // currency
            | 0x2100..=0x214F) // letterlike symbols
}

/// How many bytes in a thousand are control characters text has no use for.
/// Tabs, line ends, form feeds, the old end-of-file mark and the escape
/// that starts a terminal's colours are text's own; the rest are binary.
fn controls_per_thousand(bytes: &[u8]) -> usize {
    let controls = bytes.iter().filter(|b| matches!(b, 0x00..=0x08 | 0x0E..=0x19 | 0x1C..=0x1F | 0x7F)).count();
    controls * 1000 / bytes.len().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(text: &str, encoding: &'static Encoding) -> Vec<u8> {
        encoding.encode(text).0.into_owned()
    }

    #[test]
    fn a_byte_order_mark_says_the_encoding() {
        let mut le = vec![0xFF, 0xFE];
        for unit in "привет".encode_utf16() {
            le.extend(unit.to_le_bytes());
        }
        assert_eq!(decode(&le), Ok(decoded("привет".into(), UTF_16LE, false)));
        assert_eq!(decode(b"\xEF\xBB\xBFtea"), Ok(decoded("tea".into(), UTF_8, false)));
    }

    #[test]
    fn utf8_is_utf8_even_cut_off_or_slightly_damaged() {
        let text = "Гарантия на кофемашину - два года, ремонт бесплатный. ".repeat(4);
        assert_eq!(decode(text.as_bytes()), Ok(decoded(text.clone(), UTF_8, false)));
        let mut cut = text.as_bytes().to_vec();
        cut.extend("я".as_bytes().first());
        assert_eq!(
            decode(&cut).map(|d| (d.text, d.guessed)),
            Ok((text.clone(), false)),
            "the cut character is dropped"
        );
        let mut damaged = text.as_bytes().to_vec();
        damaged.insert(10, 0xFF);
        let read = decode(&damaged).unwrap();
        assert_eq!((read.encoding, read.guessed), ("UTF-8", true));
        assert!(read.text.contains("ремонт бесплатный"));
    }

    #[test]
    fn utf16_without_a_mark_is_found_by_reading_as_text() {
        let text = "Список покупок: молоко, хлеб, кофе.\r\nShopping list: milk, bread, coffee.\r\n";
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&le), Ok(decoded(text.into(), UTF_16LE, true)));
        assert_eq!(decode(&be), Ok(decoded(text.into(), UTF_16BE, true)));
    }

    #[test]
    fn binary_data_is_not_text() {
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x01\x00\x00\x00\x01\x00\x08\x06\x00\x00\x00";
        assert_eq!(decode(png), Err(NotText("binary data, not text")));
        let noise: Vec<u8> = (0u32..4096).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8 | 0x01).collect();
        assert_eq!(decode(&noise), Err(NotText("binary data, not text")));
    }

    #[test]
    fn a_legacy_encoding_is_guessed_and_said_to_be() {
        let russian = "Договор аренды склада на Портовой улице. Арендатор платит каждый месяц и хранит ключи у себя.";
        // KOI8-R and KOI8-U differ in Ukrainian letters only: Russian text is
        // the same in both, and the detector names them as one.
        for (encoding, named) in [
            (encoding_rs::WINDOWS_1251, ["windows-1251"; 2]),
            (encoding_rs::KOI8_R, ["KOI8-R", "KOI8-U"]),
            (encoding_rs::IBM866, ["IBM866"; 2]),
        ] {
            let read = decode(&encoded(russian, encoding)).unwrap();
            assert_eq!(read.text, russian, "{}", encoding.name());
            assert!(read.guessed && named.contains(&read.encoding), "{} read as {}", encoding.name(), read.encoding);
        }
        let german = "Die Kaffeemaschine hat zwei Jahre Garantie; für Reparaturen gilt die Größe der Rechnung.";
        assert_eq!(
            decode(&encoded(german, encoding_rs::WINDOWS_1252)),
            Ok(decoded(german.into(), encoding_rs::WINDOWS_1252, true))
        );
    }
}
