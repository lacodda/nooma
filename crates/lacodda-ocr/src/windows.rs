//! The reading itself: Windows.Media.Ocr, with Windows.Graphics.Imaging to
//! decode pictures and Windows.Data.Pdf to draw PDF pages.
//!
//! Every call here is a WinRT call made from whatever thread the caller is
//! on, and every asynchronous one is waited for where it is made: the crate
//! is used from worker threads, never from a window's own.

use std::cell::RefCell;

use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Globalization::Language;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode, SoftwareBitmap,
};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::{DataWriter, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::System::UserProfile::GlobalizationPreferences;
use windows::UI::Color;
use windows::core::{HSTRING, Interface as _};

use crate::pixels::Grey;
use crate::speller::Spellers;
use crate::{Error, Line, Ocr, Rect, Text, Word};

/// How finely a PDF page is drawn for reading, in dots an inch: what a
/// scanner is set to for documents, at which body text stands some thirty
/// pixels tall.
const PDF_DPI: f32 = 200.0;

fn failed(what: &str) -> impl Fn(windows::core::Error) -> Error + '_ {
    move |error| Error::Failed(format!("{what}: {}", error.message()))
}

fn unreadable(what: &str) -> impl Fn(windows::core::Error) -> Error + '_ {
    move |error| Error::Unreadable(format!("{what}: {}", error.message()))
}

pub(crate) fn installed() -> Result<Vec<String>, Error> {
    let languages = OcrEngine::AvailableRecognizerLanguages().map_err(failed("the recognizers cannot be listed"))?;
    let mut tags = Vec::new();
    for index in 0..languages.Size().map_err(failed("the recognizers cannot be listed"))? {
        let language = languages.GetAt(index).map_err(failed("the recognizers cannot be listed"))?;
        tags.push(language.LanguageTag().map_err(failed("the recognizers cannot be listed"))?.to_string());
    }
    Ok(tags)
}

fn none_installed() -> Error {
    Error::Unavailable(
        "no OCR language is installed in Windows: add one in Settings > Time & language > Language & region, \
         with its optical character recognition"
            .to_string(),
    )
}

/// The engine for a language, if Windows has a recognizer for it.
fn engine(tag: &str) -> Result<Option<OcrEngine>, Error> {
    let language = Language::CreateLanguage(&HSTRING::from(tag)).map_err(|_| Error::Unavailable(format!("{tag} is not a language tag")))?;
    if !OcrEngine::IsLanguageSupported(&language).map_err(failed("the recognizers cannot be asked"))? {
        return Ok(None);
    }
    OcrEngine::TryCreateFromLanguage(&language)
        .map(Some)
        .map_err(failed("the recognizer cannot be started"))
}

pub(crate) fn for_the_user() -> Result<Ocr, Error> {
    let chosen = GlobalizationPreferences::Languages().map_err(failed("the person's languages cannot be read"))?;
    let mut languages = Vec::new();
    for index in 0..chosen.Size().map_err(failed("the person's languages cannot be read"))? {
        let tag = chosen.GetAt(index).map_err(failed("the person's languages cannot be read"))?.to_string();
        add(&mut languages, &tag)?;
    }
    for tag in installed()? {
        add(&mut languages, &tag)?;
    }
    if languages.is_empty() {
        return Err(none_installed());
    }
    Ok(Ocr { languages })
}

/// Add the language a tag's recognizer reads, unless it has none or the
/// language is already there: `en-GB` after `en-US` is the same recognizer.
fn add(languages: &mut Vec<String>, tag: &str) -> Result<(), Error> {
    let Some(engine) = engine(tag)? else { return Ok(()) };
    let reads = engine
        .RecognizerLanguage()
        .and_then(|language| language.LanguageTag())
        .map_err(failed("the recognizer cannot be asked"))?
        .to_string();
    if !languages.contains(&reads) {
        languages.push(reads);
    }
    Ok(())
}

pub(crate) fn with_languages(tags: &[&str]) -> Result<Ocr, Error> {
    let mut languages = Vec::new();
    for tag in tags {
        if engine(tag)?.is_none() {
            return Err(Error::Unavailable(format!(
                "no OCR is installed in Windows for {tag}: add the language in Settings > Time & language > Language & region"
            )));
        }
        add(&mut languages, tag)?;
    }
    if languages.is_empty() {
        return Err(none_installed());
    }
    Ok(Ocr { languages })
}

/// The recognizers and spell checkers a thread reads with.
///
/// Each thread has its own: a recognizer reads one picture at a time and
/// refuses a second while the first is read ("Another RecognizeAsync
/// operation is already running"), and letting spell checkers go unloads
/// their dictionaries, which took 200 ms - two thirds of reading a page. A
/// thread that reads many pictures makes them once.
struct Readers {
    languages: Vec<String>,
    engines: Vec<OcrEngine>,
    spellers: Spellers,
}

thread_local! {
    static READERS: RefCell<Option<Readers>> = const { RefCell::new(None) };
}

/// Run `with` on this thread's readers for these languages.
fn with_readers<R>(languages: &[String], with: impl FnOnce(&Readers) -> Result<R, Error>) -> Result<R, Error> {
    READERS.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.as_ref().is_none_or(|readers| readers.languages != languages) {
            let mut engines = Vec::with_capacity(languages.len());
            for tag in languages {
                engines.push(engine(tag)?.ok_or_else(|| Error::Unavailable(format!("the recognizer for {tag} is no longer installed")))?);
            }
            *kept = Some(Readers {
                languages: languages.to_vec(),
                engines,
                spellers: Spellers::for_languages(languages),
            });
        }
        with(kept.as_ref().expect("made above"))
    })
}

/// A stream over bytes, for the decoders that read only streams.
fn stream_of(bytes: &[u8]) -> Result<InMemoryRandomAccessStream, Error> {
    let fail = failed("the bytes cannot be handed to Windows");
    let stream = InMemoryRandomAccessStream::new().map_err(&fail)?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(&fail)?;
    writer.WriteBytes(bytes).map_err(&fail)?;
    writer.StoreAsync().and_then(|store| store.join()).map_err(&fail)?;
    writer.DetachStream().map_err(&fail)?;
    stream.Seek(0).map_err(&fail)?;
    Ok(stream)
}

/// Decode a picture, stood up the way its EXIF orientation says.
fn decode(stream: &IRandomAccessStream) -> Result<Grey, Error> {
    let not_a_picture = unreadable("not a picture Windows can decode");
    let decoder = BitmapDecoder::CreateAsync(stream)
        .and_then(|decoding| decoding.join())
        .map_err(&not_a_picture)?;
    let width = decoder.OrientedPixelWidth().map_err(&not_a_picture)?;
    let height = decoder.OrientedPixelHeight().map_err(&not_a_picture)?;
    let transform = BitmapTransform::new().map_err(failed("the picture cannot be decoded"))?;
    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Straight,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .and_then(|decoding| decoding.join())
        .map_err(&not_a_picture)?;
    let bgra = data.DetachPixelData().map_err(&not_a_picture)?;
    Ok(Grey::from_bgra(width, height, &bgra))
}

pub(crate) fn read(ocr: &Ocr, picture: &[u8]) -> Result<Text, Error> {
    let stream = stream_of(picture)?;
    let grey = decode(&stream.cast().map_err(failed("the bytes cannot be handed to Windows"))?)?;
    recognize(ocr, grey)
}

pub(crate) fn read_bgra(ocr: &Ocr, width: u32, height: u32, bgra: &[u8]) -> Result<Text, Error> {
    let expected = width as usize * height as usize * 4;
    if bgra.len() != expected {
        return Err(Error::Unreadable(format!(
            "{width} by {height} pixels are {expected} bytes, not {}",
            bgra.len()
        )));
    }
    recognize(ocr, Grey::from_bgra(width, height, bgra))
}

pub(crate) fn read_pdf_pages(ocr: &Ocr, pdf: &[u8], pages: &[u32]) -> Result<Vec<Text>, Error> {
    let not_a_pdf = unreadable("not a PDF Windows can draw");
    let stream = stream_of(pdf)?;
    let document = PdfDocument::LoadFromStreamAsync(&stream)
        .and_then(|loading| loading.join())
        .map_err(&not_a_pdf)?;
    let count = document.PageCount().map_err(&not_a_pdf)?;
    let max = max_dimension()?;
    let mut texts = Vec::with_capacity(pages.len());
    for &index in pages {
        if index >= count {
            return Err(Error::Unreadable(format!("the PDF has {count} pages, and no page {}", index + 1)));
        }
        let fail = failed("the page cannot be drawn");
        let page = document.GetPage(index).map_err(&fail)?;
        // The size is in device-independent pixels, 96 to the inch.
        let size = page.Size().map_err(&fail)?;
        let scale = (PDF_DPI / 96.0).min(max / size.Width.max(size.Height).max(1.0));
        let options = PdfPageRenderOptions::new().map_err(&fail)?;
        options.SetDestinationWidth((size.Width * scale).round().max(1.0) as u32).map_err(&fail)?;
        options.SetDestinationHeight((size.Height * scale).round().max(1.0) as u32).map_err(&fail)?;
        // Paper behind the page, which a PDF draws on nothing.
        options
            .SetBackgroundColor(Color {
                A: 255,
                R: 255,
                G: 255,
                B: 255,
            })
            .map_err(&fail)?;
        // An uncompressed bitmap: drawn to be read back at once, not kept.
        options.SetBitmapEncoderId(BitmapEncoder::BmpEncoderId().map_err(&fail)?).map_err(&fail)?;
        let drawn = InMemoryRandomAccessStream::new().map_err(&fail)?;
        page.RenderWithOptionsToStreamAsync(&drawn, &options)
            .and_then(|drawing| drawing.join())
            .map_err(&fail)?;
        drawn.Seek(0).map_err(&fail)?;
        let grey = decode(&drawn.cast().map_err(&fail)?)?;
        texts.push(recognize(ocr, grey)?);
    }
    Ok(texts)
}

/// The largest picture the recognizers read, in pixels on its longer side.
fn max_dimension() -> Result<f32, Error> {
    Ok(OcrEngine::MaxImageDimension().map_err(failed("the recognizer cannot be asked"))? as f32)
}

/// Lines shorter than this, in pixels from the top of their tallest letter
/// to the bottom of their lowest, are misread: a screen's 12-pixel type read
/// `inventory` as `irwentory` and `mode` as `rnode`, and the same picture
/// one and a half times larger read every word.
const SMALLEST_LINE: f32 = 20.0;

/// How tall a line too small to read is made.
const ENLARGED_LINE: f32 = 26.0;

/// How many times a picture is enlarged at most: beyond it, the strokes are
/// guessed rather than seen.
const MOST_ENLARGED: f32 = 3.0;

/// Read a picture with the recognizer for the language it is in.
///
/// The first recognizer reads it as it is, and the reading says how tall
/// its lines are: a picture whose lines are too small is enlarged and read
/// again. Then every other recognizer reads the same pixels, and the reading
/// with the most letters in words the person's languages know is the text -
/// the first, unless another knows more.
fn recognize(ocr: &Ocr, picture: Grey) -> Result<Text, Error> {
    with_readers(&ocr.languages, |readers| {
        let max = max_dimension()?;
        let longest = picture.width.max(picture.height) as f32;
        let mut scale = (max / longest).min(1.0);
        let mut grey = if scale < 1.0 { scaled(&picture, scale) } else { picture.clone() };
        let mut bitmap = bitmap_of(&grey)?;

        let mut readings = readers.engines.iter().zip(&readers.languages);
        let Some((engine, language)) = readings.next() else {
            return Ok(Text::default());
        };
        let mut first = read_with(engine, &bitmap, language)?;
        // Nothing on it for the first recognizer: no lines for any.
        if first.lines.is_empty() {
            return Ok(first);
        }
        let height = line_height(&first);
        if height < SMALLEST_LINE {
            let enlarged = (scale * ENLARGED_LINE / height).min(MOST_ENLARGED).min(max / longest);
            if enlarged > scale * 1.1 {
                scale = enlarged;
                grey = scaled(&picture, scale);
                bitmap = bitmap_of(&grey)?;
                first = read_with(engine, &bitmap, language)?;
            }
        }
        drop(grey);

        let judge = |text: &Text| {
            let plain = text.plain();
            let letters = plain.chars().filter(|c| c.is_alphabetic()).count();
            let known = if readers.spellers.is_empty() {
                crate::clean_letters(&plain)
            } else {
                readers.spellers.known_letters(&plain)
            };
            (known, letters)
        };
        let (mut best_known, letters) = judge(&first);
        let mut best = first;
        // Every word known: another language can tie, and the earlier wins.
        if best_known < letters {
            for (engine, language) in readings {
                let text = read_with(engine, &bitmap, language)?;
                let (known, _) = judge(&text);
                if known > best_known {
                    (best_known, best) = (known, text);
                }
            }
        }
        Ok(unscaled(best, scale))
    })
}

fn scaled(picture: &Grey, scale: f32) -> Grey {
    picture.resized((picture.width as f32 * scale).round() as u32, (picture.height as f32 * scale).round() as u32)
}

/// The usual height of a reading's lines: the median, so a heading or a
/// line of capitals does not decide it.
fn line_height(text: &Text) -> f32 {
    let mut heights: Vec<f32> = text.lines.iter().filter_map(|line| line.rect()).map(|rect| rect.height).collect();
    heights.sort_by(f32::total_cmp);
    heights.get(heights.len() / 2).copied().unwrap_or(f32::MAX)
}

/// A reading of a scaled picture, with every word where it stands in the
/// picture as it was given.
fn unscaled(mut text: Text, scale: f32) -> Text {
    if scale != 1.0 {
        for word in text.lines.iter_mut().flat_map(|line| line.words.iter_mut()) {
            word.rect = Rect {
                x: word.rect.x / scale,
                y: word.rect.y / scale,
                width: word.rect.width / scale,
                height: word.rect.height / scale,
            };
        }
    }
    text
}

fn read_with(engine: &OcrEngine, bitmap: &SoftwareBitmap, language: &str) -> Result<Text, Error> {
    let result = engine
        .RecognizeAsync(bitmap)
        .and_then(|reading| reading.join())
        .map_err(failed("the recognizer failed"))?;
    text_of(&result, language)
}

fn bitmap_of(grey: &Grey) -> Result<SoftwareBitmap, Error> {
    let fail = failed("the pixels cannot be handed to Windows");
    let writer = DataWriter::new().map_err(&fail)?;
    writer.WriteBytes(&grey.luma).map_err(&fail)?;
    let buffer = writer.DetachBuffer().map_err(&fail)?;
    let (width, height) = (
        i32::try_from(grey.width).map_err(|_| Error::Unreadable("the picture is too wide".to_string()))?,
        i32::try_from(grey.height).map_err(|_| Error::Unreadable("the picture is too tall".to_string()))?,
    );
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, width, height).map_err(&fail)
}

fn text_of(result: &OcrResult, language: &str) -> Result<Text, Error> {
    let fail = failed("the reading cannot be taken");
    let angle = result.TextAngle().ok().and_then(|angle| angle.Value().ok());
    let lines = result.Lines().map_err(&fail)?;
    let mut text = Text {
        language: language.to_string(),
        angle,
        lines: Vec::with_capacity(lines.Size().map_err(&fail)? as usize),
    };
    for index in 0..lines.Size().map_err(&fail)? {
        let line = lines.GetAt(index).map_err(&fail)?;
        let words = line.Words().map_err(&fail)?;
        let mut read = Line {
            text: line.Text().map_err(&fail)?.to_string(),
            words: Vec::with_capacity(words.Size().map_err(&fail)? as usize),
        };
        for index in 0..words.Size().map_err(&fail)? {
            let word = words.GetAt(index).map_err(&fail)?;
            let rect = word.BoundingRect().map_err(&fail)?;
            read.words.push(Word {
                text: word.Text().map_err(&fail)?.to_string(),
                rect: Rect {
                    x: rect.X,
                    y: rect.Y,
                    width: rect.Width,
                    height: rect.Height,
                },
            });
        }
        text.lines.push(read);
    }
    Ok(text)
}
