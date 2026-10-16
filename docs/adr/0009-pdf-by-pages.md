# 0009 — PDF is read by pdf_oxide, page by page, and a page ends a chunk

Date: 2026-10-16
Status: accepted

## Context

Until v0.10 a library held markdown and plain text. PDF is the next format the plan names, and the one a personal archive holds most of after notes: books, papers, manuals, statements, contracts. Four questions came with it.

1. **What reads the text.** PDF text is glyphs placed on a page, mapped to characters by each font's own tables. A reader decides what a word is, which line follows which, and what to do with hyphens, ligatures and two columns. What it gets wrong is not visible in a search - it is a document that is not found.
2. **Where a result is.** A note's place is a line, which an editor opens. A PDF's place is a page, which a reader opens; nobody opens a PDF at line 340.
3. **What the document is called.** A PDF carries a title in its metadata, as often the name of the program that printed it as the name of the document.
4. **What a library says about a PDF it cannot read.** A scan has no text at all; a PDF can need a password; a malformed one can make its reader fail.

## Decision

**`pdf_oxide` reads the text, through its `extract_text`.** It was measured against the alternatives on twenty-five real PDFs - arXiv papers, Wikipedia articles in Russian and English printed by its own renderer, a US tax form, the test files of pdf.js, seven Russian books with typeset and OCR text layers - and on three PDFs whose text was known: the synthetic bilingual corpus of `tests/meaning`, printed by Chrome in one column, two columns, and a narrow hyphenated column. On the known text it returned every word (recall 1.000) where `pdf-extract` left words broken by hyphenation (0.996); on Russian Wikipedia 0.996-1.000 against 0.983-0.986, with more word pairs in their order (0.970-0.989 against 0.934-0.956). It reads books two to ten times faster (354 pages in 1.1 s against 9.1 s), opens PDFs locked only by an owner password, drops running headers, and on a font with no character map it returns nothing rather than noise. `pdf-extract` broke the words of OCR text layers into letters and syllables. `pdfium-render` would ship a C++ library beside the binary. `pdf_oxide`'s other mode, `to_plain_text`, orders text by layout analysis and interleaved the two columns of a paper line by line; `extract_text` did not. It is built without its default features: colour management is for rendering, which nooma does not do; the RC4 cipher stays, for PDFs locked with the empty password by older producers.

**A page ends a chunk, as a heading does.** Every chunk is on one page, and the page a result names - `page` in the JSON, `p. 12` in the window and the shell - is the page the answer is on. A chunk keeps a line too: the line of the text read out of the PDF, counted through the pages, which is where the vectors and the full-text index find a chunk by. The PDF's bookmarks stand in for headings: a chunk is under the path of the last bookmark pointing at its page or an earlier one.

**Text is cleaned before it is cut.** A soft hyphen goes with the line break and the spaces around it, joining the word it broke; the text is put in NFKC, so ligatures and non-breaking spaces are ordinary characters; runs of spaces become one.

**A PDF's title is its file name.** The metadata title is searched as an alias when it names the document - not empty, not the file name again, and not a file name itself, such as `Microsoft Word - report.docx`.

**What read a file is part of the manifest.** Every PDF's entry carries `pdf_oxide`'s version. A file whose bytes did not change but whose reader did is read again, and only PDFs are: a newer reader is the most likely reason the text of an unchanged PDF would improve, and the stack is updated weekly.

**What cannot be read is said.** A PDF that needs a password to open is skipped with that reason. One with no text on any page is indexed by its name and metadata title, and the update names it (`textless`) - OCR is a later version. A page the reader fails on reads as empty; a document whose every page fails is skipped. A panic inside the reader is caught and becomes the reason the file was skipped.

**Size is a property of the kind.** Markdown and text larger than 16 MB are a log or a dump and are skipped; a PDF may be 256 MB, since its bytes are mostly pictures, which are not read.

## Consequences

Positive:

- A book or a paper is found at its page and its chapter, in Russian and English, with words broken across lines found whole.
- A scan is still found by its name, and the update says which files are scans, so the reason a search misses their text is visible.
- Updating `pdf_oxide` reads every PDF again by itself, and nothing else.

Negative:

- `pdf_oxide` is heavy: about 140 crates in its tree, among them an image decoder and a layout engine the reading does not use. The offline test checks none of them can reach the network.
- A paragraph that runs over a page break is two chunks, each matching half of it.
- In an OCR text layer, `extract_text` sometimes puts the lines of a page out of order; and it can run two paragraphs together without a space (`loop.Every`), which the tokenizer splits anyway.
- `pdf_oxide` 0.3.78 does not resolve named destinations whose names are not UTF-8 - the UTF-16 names InDesign writes - so such a book's bookmarks give no headings. Its pages and text are unaffected.
- Every new `pdf_oxide` version costs one reading of every PDF.

Rejected alternatives:

- **`pdf-extract`.** Smaller and older; broken words in OCR text and hyphenation, garbage on fonts without a character map, slow on books.
- **PDFium.** The best reader there is, and a native library to ship, version and load beside every build.
- **Chunks packed across pages.** Fewer, fuller chunks on slides and short pages, and a page number that can be wrong for every chunk but the first page's.
- **The metadata title as the title.** Right for a paper, wrong for most of what an archive holds, and a wrong title is shown on every result.
- **Bumping the chunker's version when the PDF reader changes.** One more thing to remember at every weekly stack update, and a reindex of every note for a change that only touched PDFs.
