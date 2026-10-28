---
title: Pictures and scans
description: How nooma reads the text in screenshots, photos of pages and scanned PDFs with the OCR Windows ships with, how it picks the language a picture is in, and what it costs.
---

A screenshot of an error, a photo of a receipt, a contract that came back from the scanner: their text is pixels, and a search that reads files sees none of it. Since v0.12.0 nooma reads that text where Windows can - on the machine, with the recognizers Windows has had since Windows 10, one per language, installed with the language. A screenshot is found by the words on the screen, a scan by what the page says, and a scanned page inside a typed document is read with the others.

Nothing about this leaves the machine. The recognizers are part of Windows; nooma hands them pixels and gets text back. The reading is done by [`lacodda-ocr`](https://crates.io/crates/lacodda-ocr), a crate of the lacodda line that nitid reads screenshots with as well.

## What is read

- **Pictures** - PNG, JPEG, TIFF, BMP, GIF, and WebP, HEIC or AVIF where Windows has their extensions. A photo taken with the phone held sideways is stood up the way its EXIF says; a picture cut out with a transparent background is put on white paper first, since the recognizers do not see transparency and would read black text on nothing as black on black.
- **PDF pages with no text on them** - a whole scan, or one scanned page among typed ones. Windows draws the page at 200 dots an inch, as a scanner is set for documents, and the drawing is read. A page with text is read as text and never drawn.

A picture with no text in it - a photo of a sunset - is not a document. nooma passes over it without a word, as over any file that is no document, and does not look at it again until it changes or something that reads pictures does: a language added to Windows, a newer nooma.

Text read this way may hold a misread word, so it says where it came from: the window marks the row **OCR**, `find` writes `· OCR` after the place, and `--json` gives `recognized`.

## Which language

A recognizer reads one language - one alphabet and the words of that language - and reads every picture as if it were in it. The English recognizer reads a Russian page as `rapaHTHiiHb1ii`. The Russian one reads an English page almost right, but writes `on` and `June` with Cyrillic letters that look the same, so neither word is ever found. The English one reads a Spanish page in the right alphabet, but `mañanas` comes out `mafianas`.

So every recognizer installed reads the picture, the languages you have chosen in Windows first, and the reading kept is the one with the most letters in words your languages know - judged by the spell checkers Windows underlines typing with, one per language. On the test pages the English, Russian and Spanish letters each went to their own recognizer by a clear margin: 414 known letters of 414 for the Spanish reading of the Spanish letter, against 358 for the English reading of it. When the first reading's words are all known, the others are not asked; when it finds no lines at all, there is nothing in the picture for any of them.

## Small type

The recognizers misread text smaller than about twenty pixels a line: a screen at 100% sets its type twelve pixels tall, and read as it is, `inventory` came back `irwentory` and `mode` came back `rnode`. The first reading says how tall the lines are, and a picture whose lines are smaller is enlarged until they stand about twenty-six pixels, at most three times, and read again. The words' places are given in the picture's own pixels either way.

## What it costs

Measured on an i7-1360P with English, Russian and Spanish installed:

| Picture | Time |
| --- | --- |
| A page of a Russian letter, 1654 by 2339 | 75 ms |
| A page of an English letter, three recognizers asked | 160 ms |
| A screenshot at 100%, enlarged and read again | 160 ms |
| A photo with no text in it | 5 ms |

A scanned PDF costs that for each of its pages, plus drawing the page. A folder of photos with nothing written in them costs a few milliseconds each, once.

## Where there is no OCR

On macOS and Linux, or on a Windows with no recognizer installed, pictures are not looked at, and a scan is found by its name and its metadata, as before - the report names it and says why its text was not read. A language is added in Windows Settings > Time & language > Language & region, with its optical character recognition. nooma asks Windows which recognizers there are when it starts - the window when it opens, the shell on every command - and the first update after that reads the pictures and scans again.
