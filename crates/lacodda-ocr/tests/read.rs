//! Reading the pictures in tests/images, made by images/source/make_images.py.
//!
//! They need the recognizers for English, Russian and Spanish installed in
//! Windows, and do not skip themselves without them: a test that passes
//! because nothing could be read proves nothing.

use lacodda_ocr::{Error, Ocr};

#[cfg(not(windows))]
#[test]
fn there_is_no_ocr_but_on_windows() {
    assert!(matches!(Ocr::new(), Err(Error::Unavailable(_))));
    assert!(matches!(lacodda_ocr::installed(), Err(Error::Unavailable(_))));
}

#[cfg(windows)]
mod on_windows {
    use std::path::Path;

    use lacodda_ocr::Text;

    use super::*;

    fn picture(name: &str) -> Vec<u8> {
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/images").join(name)).unwrap()
    }

    /// The three languages the pictures are in, Russian first, as the
    /// person who wrote these tests has them: an English page has to win
    /// against the language put before it.
    fn ocr() -> Ocr {
        Ocr::with_languages(&["ru", "en-US", "es-ES"]).expect("the recognizers for Russian, English and Spanish are installed")
    }

    fn read(name: &str) -> Text {
        ocr().read(&picture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    /// The words of a reading, lowercased, without punctuation.
    fn words(text: &Text) -> Vec<String> {
        text.plain()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    }

    fn assert_has(text: &Text, expected: &[&str]) {
        let words = words(text);
        for word in expected {
            assert!(words.iter().any(|w| w == word), "{word} is not in {:?}", text.plain());
        }
    }

    #[test]
    fn each_letter_is_read_in_its_own_language() {
        let english = read("letter-en.png");
        assert_eq!(english.language, "en-US");
        assert_has(&english, &["warehouse", "renewal", "deposit", "fifteenth", "on", "june"]);

        let russian = read("letter-ru.png");
        assert_eq!(russian.language, "ru");
        assert_has(&russian, &["гарантийный", "уплотнителя", "кофемашина", "накипи"]);

        let spanish = read("letter-es.png");
        assert_eq!(spanish.language, "es-ES");
        assert_has(&spanish, &["préstamo", "mañanas", "amplía", "podrá", "climatización"]);
    }

    #[test]
    fn the_language_whose_words_they_are_wins_where_the_alphabet_cannot_tell() {
        // English asked first. Its reading - `La mafiana del jueves el sefior
        // Mufioz` - is as many Latin letters as the Spanish one, with no
        // digit among them: by alphabet the two tie, and the first would win.
        // Only the dictionaries know `mañana` is a word and `mafiana` none.
        let text = Ocr::with_languages(&["en-US", "es-ES"]).unwrap().read(&picture("accents.png")).unwrap();
        assert_eq!(text.language, "es-ES", "{}", text.plain());
        assert_has(&text, &["mañana", "señor", "muñoz", "estación", "compañía"]);
    }

    #[test]
    fn a_recognizer_for_another_alphabet_does_not_win() {
        // The Russian recognizer reads `on` and `June` as the Cyrillic
        // `оп` and `Јипе`: words in an alphabet, and no word of English.
        let english = read("letter-en.png");
        assert!(!english.plain().chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c)), "{}", english.plain());
    }

    #[test]
    fn paragraphs_are_apart_and_lines_of_one_are_not() {
        let text = read("letter-en.png").plain();
        let paragraphs: Vec<&str> = text.split("\n\n").collect();
        assert_eq!(paragraphs.len(), 6, "{text}");
        assert!(paragraphs[0].starts_with("Northfield Storage"));
        assert_eq!(paragraphs[1], "Notice of the warehouse lease renewal");
        assert_eq!(paragraphs[2].lines().count(), 3, "the first paragraph is three lines: {}", paragraphs[2]);
        assert!(paragraphs[5].starts_with("Kind regards,\n"));
    }

    #[test]
    fn small_type_is_enlarged_until_it_reads() {
        // Twelve-pixel type, as a screen at 100% sets it: read as it is, the
        // recognizer gave `irwentory`, `rnode` and `remcn.'e`.
        let screen = read("screen.png");
        assert_has(&screen, &["inventory", "mode", "remove", "archive", "configuration", "fourteen"]);
    }

    #[test]
    fn words_stand_where_they_are_in_the_picture_as_given() {
        // The screen was read enlarged; its words are in its own pixels.
        let screen = read("screen.png");
        let all: Vec<_> = screen.lines.iter().flat_map(|line| &line.words).collect();
        assert!(
            all.iter()
                .all(|word| word.rect.x >= 0.0 && word.rect.right() <= 1280.0 && word.rect.bottom() <= 720.0)
        );
        let heading = all.iter().find(|word| word.text == "Backup").expect("the light panel's heading");
        assert!(
            (15.0..30.0).contains(&heading.rect.x) && (15.0..40.0).contains(&heading.rect.y),
            "{:?}",
            heading.rect
        );
        let dark = all.iter().find(|word| word.text == "Build").expect("the dark panel's heading");
        assert!(dark.rect.x > 640.0, "the dark panel is the right half: {:?}", dark.rect);
    }

    #[test]
    fn a_photo_is_stood_up_the_way_its_exif_says() {
        // Stored on its side, 1754 by 1240, and shown upright.
        let photo = read("photo-sideways.jpg");
        assert_has(&photo, &["warehouse", "renewal", "deposit"]);
        let all: Vec<_> = photo.lines.iter().flat_map(|line| &line.words).collect();
        assert!(all.iter().all(|word| word.rect.right() <= 1240.0 && word.rect.bottom() <= 1754.0));
    }

    #[test]
    fn a_transparent_background_is_paper() {
        assert_has(&read("transparent.png"), &["signed", "notary", "march"]);
    }

    #[test]
    fn a_picture_with_no_text_reads_as_none() {
        let text = read("no-text.jpg");
        assert!(text.is_empty() && text.plain().is_empty());
    }

    #[test]
    fn a_scanned_pdf_is_read_page_by_page() {
        let pages = ocr().read_pdf_pages(&picture("scan.pdf"), &[1, 0]).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].language, "ru", "in the order asked");
        assert_has(&pages[0], &["гарантийный", "ремонт"]);
        assert_has(&pages[1], &["warehouse", "lease"]);
        assert!(matches!(ocr().read_pdf_pages(&picture("scan.pdf"), &[2]), Err(Error::Unreadable(_))));
    }

    #[test]
    fn what_is_not_a_picture_is_said_to_be_none() {
        assert!(matches!(ocr().read(b"%PDF-1.7 or anything else"), Err(Error::Unreadable(_))));
        assert!(matches!(ocr().read(&[]), Err(Error::Unreadable(_))));
        assert!(matches!(ocr().read_pdf_pages(&picture("letter-en.png"), &[0]), Err(Error::Unreadable(_))));
        assert!(matches!(ocr().read_bgra(2, 2, &[0; 15]), Err(Error::Unreadable(_))));
    }

    #[test]
    fn decoded_pixels_read_as_the_picture_does() {
        // A white picture with nothing on it: the pixel path reaches the
        // recognizers and comes back empty.
        let text = ocr().read_bgra(64, 32, &[255; 64 * 32 * 4]).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn a_language_with_no_recognizer_is_said() {
        assert!(matches!(Ocr::with_languages(&["xx-unknown"]), Err(Error::Unavailable(_))));
        let mine = Ocr::new().unwrap();
        assert!(!mine.languages().is_empty());
        let installed = lacodda_ocr::installed().unwrap();
        assert!(
            mine.languages().iter().all(|language| installed.contains(language)),
            "{:?} of {installed:?}",
            mine.languages()
        );
        assert!(mine.reader().starts_with(&format!("lacodda-ocr {} (", lacodda_ocr::VERSION)));
    }

    #[test]
    fn one_reader_reads_on_many_threads_at_once() {
        let ocr = ocr();
        let alone = ocr.read(&picture("letter-ru.png")).unwrap();
        let bytes = picture("letter-ru.png");
        std::thread::scope(|scope| {
            let readings: Vec<_> = (0..6).map(|_| scope.spawn(|| ocr.read(&bytes).unwrap())).collect();
            for reading in readings {
                assert_eq!(reading.join().unwrap(), alone);
            }
        });
    }
}
