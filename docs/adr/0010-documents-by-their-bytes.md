# 0010 — Word documents through office_oxide, books read here, and a file is what its bytes say

Date: 2026-10-21
Status: accepted

## Context

v0.11 adds the last two formats of the plan's first block, Word documents and EPUB books, and two things about every file: how its kind is told, and how its text is decoded. Five questions came with them.

1. **What reads a Word document.** A `.docx` is a zip of XML, but the text a person sees is not the text in the XML: Word stores a tracked deletion beside what replaced it, a text box twice (as a drawing and as VML), the contents as a field with a code and a result, and names its heading styles in the language of its interface - a Russian Word writes `1` and `2` where an English one writes `Heading1`.
2. **What reads a book.** An EPUB is a zip of XHTML pages, a package naming them in reading order, and a table of contents - an EPUB 3 navigation document or an EPUB 2 NCX - pointing at pages and at places inside them.
3. **Where a result is.** Neither has a page or a line a reader opens it at.
4. **What a file is.** Until v0.10 the extension decided, and a PDF a browser saved as `invoice` was not looked at, while a picture named `.pdf` was handed to the PDF reader.
5. **What a text file's encoding is.** Until v0.10 anything not UTF-8, or UTF-16 with a byte order mark, was skipped: a note in Windows-1251 was not found at all.

## Decision

**`office_oxide` reads Word documents, through its document model.** It is by `pdf_oxide`'s author and was already built as its dependency. It was measured on Word's own files: Word itself, driven through COM, wrote the fixtures - headings in built-in styles under a Russian interface, a table, a list, a footnote, a text box, a comment, a header, a tracked deletion and insertion, the contents field, numbered headings - and turned five pages of these docs into documents, saving each again as the plain text Word reads out of it. Against Word's own text `office_oxide` returned 0.989-0.995 of the words with precision 0.998-1.000; what it did not return were the bullets Word prints in a text export. It read every style id as its heading, a revision as it stands, a text box once, and each document in 2-4 ms. Reading the XML here would mean writing what it does - style inheritance, outline levels, numbering, fields, revisions, the alternate content of drawings - and none of it would be measured better. Around it, nooma decides what a search needs: a comment and a header are not the document's text; a footnote is read after the paragraph that points at it, under that paragraph's headings, rather than at the end; the lines of the contents - a heading again, maybe after its number, and a page - are not read, or every heading's words would sit in one chunk that answers every question with the contents. A document's title is its file name, as a PDF's is; the title in its properties is an alias.

**Books are read here, on the zip and XML readers the Word reader brings.** No EPUB crate reads a page's text; the package and the contents are the small part, and the pages are XHTML read event by event, with the HTML entities books use. A page of the reading order ends a chunk, as a PDF page does, and the contents stand in for bookmarks: a chunk is under the path of the last entry pointing at its page or at an element in it before the chunk, so chapters and the sections inside them both head chunks. A book without contents is headed by its `h1`..`h6`. The marks that point at a note - a link whose whole text is a note's number or sign, or one marked `noteref` - are not text; a note listed in the contents by its mark, as books converted from FB2 list every note, heads nothing. A book's title is its `dc:title`: a book's file name is as often a number as a name, and its title is what every reading app shows. Its authors and file name are aliases, since a book is asked for by its author. A book whose pages are enciphered - `META-INF/encryption.xml` naming anything but font obfuscation - is skipped as protected by DRM. On 88 real books in Russian and English every one was read, every chunk but the title pages under an entry of the contents, in 12 ms each at the median and 52 ms at most.

**Neither has a line or a page to show.** The line a chunk keeps is counted through the text read out of the document, and keys the chunk for the vectors and the index; the headings above it are where it is, to a person. The window and the shell show the file and its headings.

**The bytes decide what a file is.** A file is offered by its name - a document's extension, or none - and its first bytes decide: `%PDF-` is a PDF, a zip is a Word document if its content types name a Word main part and a book if its `mimetype` says so, a compound file is an old Office document or a new one under a password, a known signature is a picture or an archive, and the rest may be text. A Word document renamed to `.pdf` is read as one. A file whose name claims a document it is not is skipped with what it is; one with no extension that is no document is passed without a word, and the manifest remembers it, so it is not opened again until it changes. The size limit belongs to the kind the bytes say.

**Encodings are told by the bytes, and a guess is said.** A byte order mark says the encoding. Zero bytes mean UTF-16, if the bytes read as text in it, or no text at all - and come before UTF-8, since UTF-16 of Cyrillic or Latin text is valid UTF-8. Valid UTF-8 is UTF-8, and so is UTF-8 with a character cut off at the end or one damaged sequence in a hundred, which no legacy encoding produces by chance. Control bytes text has no use for are a binary file. The rest is guessed by `chardetng`, the detector Firefox uses for pages that name no encoding; it told Windows-1251, KOI8, code page 866, Windows-1252 and Shift_JIS apart on the fixtures, and Word's Russian plain-text exports on the machine as Windows-1251. The update lists every file whose encoding it guessed, with the guess, so a wrong one is seen.

**What read a file is recorded, for every kind.** The manifest keeps the reader of each file: `pdf_oxide`'s version, `office_oxide`'s, the number of the rules books are read by, and the detector for a text whose encoding was guessed. A different reader now reads those files again, and only those. A file's kind is kept with it too, so nothing asks its name again.

## Consequences

Positive:

- Word documents and books are found under their headings and contents; a book by its author as well.
- A PDF, a Word document or a book is found whatever it is named, and a note in a legacy encoding is found at all.
- A guessed encoding, a file that is not what its name says, a locked document - each is said in the report, not left to look like a search that misses.

Negative:

- A Word document's location is its headings only; one without headings is found as a whole.
- A guess can be wrong, mostly on short texts; it is listed, not corrected. A text in a legacy encoding a few words long gives the detector little to go on.
- Old binary Office files (`.doc`) are not read, though `office_oxide` reads them; they are a format the plan does not name, and a later version can take them.
- A file with no extension is opened once to be told; a folder of many such files pays for that on its first update.
