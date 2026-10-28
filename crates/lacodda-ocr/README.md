# lacodda-ocr

> Text in pictures and in scanned PDF pages, read on the machine by the OCR that ships with Windows.

[![crates.io](https://img.shields.io/crates/v/lacodda-ocr?style=flat-square)](https://crates.io/crates/lacodda-ocr)
[![docs.rs](https://img.shields.io/docsrs/lacodda-ocr?style=flat-square&label=docs.rs)](https://docs.rs/lacodda-ocr)

The OCR of the [lacodda line](https://github.com/lacodda): nooma reads
screenshots, photos of pages and scanned PDFs with it to find them by their
text, and nitid copies the text out of a screenshot. The recognizers are the
ones Windows has had since Windows 10 - local, one per language, installed
with the language. What the crate adds is what using them takes:

- **any picture Windows decodes** - PNG, JPEG, TIFF, BMP, GIF, and HEIC or
  WebP where their extensions are installed - stood up the way its EXIF
  orientation says, or pixels you have already decoded;
- **a PDF page drawn as a scanner would see it**, for the pages that are
  pictures of text;
- **the language chosen by the text**: the recognizers for every language the
  person reads in try the picture, and the reading that is text in its own
  alphabet wins;
- **paragraphs**, from where the lines stand: the recognizer gives lines and
  nothing else.

```rust
let ocr = lacodda_ocr::Ocr::new()?;          // the person's languages
let text = ocr.read(&std::fs::read("scan.png")?)?;
println!("{}", text.plain());                // lines, paragraphs apart
for line in &text.lines {
    for word in &line.words {
        println!("{} at {:?}", word.text, word.rect);
    }
}
```

Nothing leaves the machine: the crate has no network code, and Windows reads
the text with what is installed. On other systems it builds, and
`Ocr::new()` returns `Error::Unavailable`. A language is added in Windows
Settings > Time & language > Language & region, with its optical character
recognition.

How the choices were made and measured:
[Pictures and scans](https://lacodda.github.io/nooma/concepts/pictures/) on the
nooma documentation site.

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
