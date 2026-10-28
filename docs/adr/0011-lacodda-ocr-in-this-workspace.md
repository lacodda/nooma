# 0011 — lacodda-ocr lives here, with a version of its own

Date: 2026-10-28
Status: accepted

## Context

v0.12 reads the text in pictures and scanned PDF pages with the OCR Windows ships with. nitid, the line's image viewer, plans the same reader for its screenshots: text copied out of a picture, a session's screenshots as one document, e-mail addresses and tokens found by their words and blurred. The line's rule is a shared crate once a second consumer is visible, and nitid is that consumer; the plan of 11.09 said whichever product got there first would write it, and nitid had not started.

What is shared is more than a call into Windows. A recognizer refuses a second picture while it reads the first, so readers are made per thread; the spell checkers that judge which language's reading to keep take 200 ms to let go, so they are kept per thread too; pictures are decoded, stood up by their EXIF orientation and put on white paper; small type is enlarged; a reading's lines are laid out as paragraphs. Two copies of that would drift apart.

The crate could live in a repository of its own or in this workspace.

## Decision

`lacodda-ocr` is a crate of this workspace, `crates/lacodda-ocr`, published to crates.io from here - the way `lacodda-seal` lives in efema's workspace and `furca-core` in furca's.

It carries **its own version**, not the workspace's. A nooma release that does not touch it must not make nitid move its dependency. Its version moves when its API does, or when the same picture would be read differently - and its version is part of what nooma records as a file's reader, so a picture read by an older crate is read again.

Everything that is Windows lives in the crate: the recognizers, the image decoders, the PDF renderer, the spell checkers. nooma-core asks it for text and never names a Windows type. On other systems it builds and says there is no OCR.

Its API gives words with their rectangles, in the picture's own pixels, for what nitid does with them; nooma reads only the plain text.

## Consequences

- A repository of its own would need its own mark, documentation site and CI for some twelve hundred lines; this one has all three. If the crate ever outgrows nooma, moving it is a new trusted publisher on crates.io, not a change for its users.
- How the choices were measured is on nooma's site ([Pictures and scans](https://lacodda.github.io/nooma/concepts/pictures/)), and ADR 0012 records them.
- Its tests need the recognizers for English, Russian and Spanish installed; CI installs them on the Windows runner.
- The first publication to crates.io is made by hand, as every first one in the line is: a trusted publisher cannot be attached to a crate that does not exist. Until then nitid takes it by path or from git.
