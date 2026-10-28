# 0012 — Pictures and scanned pages are read by Windows OCR, in the language whose words they hold

Date: 2026-10-28
Status: accepted

## Context

Until v0.11 a scan was a PDF found by its name, and a picture was not a document at all. v0.12 reads their text. Five questions came with it.

1. **What reads the text.** It has to stay on the machine (ADR 0002) and read Russian as well as English.
2. **Which language a picture is in.** A recognizer reads one language and reads every picture as if it were in it.
3. **How small text is read.** A screenshot sets its type twelve pixels tall.
4. **Which PDF pages are read, and how they become pictures.**
5. **What a picture with no text is,** and when a picture or a scan is read again.

## Decision

**Windows reads it, through `lacodda-ocr` (ADR 0011).** Windows has had a recognizer per language since Windows 10, installed with the language: nothing to download, nothing to bundle, local by construction. Tesseract would be a C++ library to build and a model per language to fetch and keep; the Rust OCR crates read Latin alone. The cost is that pictures are read on Windows only; elsewhere they are not looked at and a scan is found by its name, as before. macOS has a reader of its own (Vision), which the crate can grow into.

**Every installed recognizer reads, and the reading whose words are words wins.** The person's languages in Windows are asked first, then any other installed recognizer. Each reading is judged by how many of its letters are in words some language's spell checker knows - the dictionaries Windows underlines typing with - and the most wins; a tie goes to the language asked first. Measured on letters in three languages: the Russian recognizer read the English letter with `on` and `June` in Cyrillic letters (553 known letters to 579); the English one read the Spanish letter as `mafianas`, `abrirå` (358 to 414); the English one read the Russian letter as `rapaHTHiiHb1ii`. Judging by alphabet alone cannot tell English from Spanish; the person's first language alone gets every other one wrong. Where no spell checker is installed, a reading is judged by its letters in words of one alphabet with no digit inside. A reading whose words are all known ends the asking - a Russian page read by the Russian recognizer first costs one reading, 75 ms instead of 160 - and a first reading with no lines ends it too: there is nothing in the picture.

**Small type is enlarged.** Lines under twenty pixels were misread (`irwentory`, `rnode`); one and a half times larger, every word was read. The first reading measures the median line, and a picture whose lines are smaller is enlarged to twenty-six pixels a line, at most three times, and read again. Words keep the places they have in the picture as given.

**A PDF page with no text is drawn and read; a page with text never is.** Windows draws it at 200 dots an inch - a scanner's setting for documents, body text some thirty pixels tall. A typed document with one scanned page reads the typed pages as text and the scanned one by OCR. The file is read from disk again for the drawing rather than its bytes kept: they were given to the PDF reader, and keeping every PDF's bytes for the few that are scans would double what a folder of books holds in memory. A chunk read by OCR says so (`recognized`), since its words may be misread, and the full-text format goes to 5 to keep it.

**A picture with no text is no document,** passed over without a word, as a file with no extension that is no document is (ADR 0010). The manifest keeps what looked at it - the crate's version and the languages - and looks again only when the file or that changes. A picture with text records the same reader, and a PDF records its reader and, where there is OCR, the OCR's: a language added, or a newer crate, reads the pictures and every PDF again, and a scan read before there was OCR is read when there is.

## Consequences

Positive:

- A screenshot is found by the words on the screen, a scanned contract by its clauses, a scanned page in a typed document with the rest - in Russian, English, Spanish, or any language installed in Windows.
- A folder of photos costs a few milliseconds a photo, once.

Negative:

- Pictures are read on Windows only.
- A picture costs up to one reading per installed language when its words are not all known; a scanned book of three hundred pages takes a minute or more on the first update.
- A recognizer's language model is Windows' own: a word it misreads is misread the same way every time, and nothing here can correct it.
- A multi-page TIFF is read by its first page.
