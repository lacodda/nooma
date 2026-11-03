---
title: Reading what was found
description: The preview beside the list - the passage found among its neighbours, a note drawn as a note - opening a document at the place it was found, in the program that opens it, and taking a passage away as a quote.
---

A list of results says where the answer is. Since v0.14.0 the window also shows it: the row selected is read beside the list, in its document, and opened from there at the place it was found.

## The preview

Where the window is wide enough - 860 pixels and more, as it opens - the row selected is read on the right of the list. The passage found is there whole, not its fragment, with the passages before and after it in the document; the words of the query are marked in all of them, in any of their forms, as the list marks them. **Show earlier** and **Show later** read on through the document, and the passage found stays where the eye is while they do. The handle between the list and the preview moves the boundary, and the window remembers it.

A note is drawn as a note: headings where its sections start, lists and tasks, quotes, tables, code, and what Obsidian adds to markdown - links between notes shown by their names, embeds, tags, `==highlights==`, callouts in the colour of their kind; `%%comments%%` are not shown, as Obsidian does not show them. A page of a PDF, a Word document, a book or a picture is shown as the text read out of it, a PDF under the number of each page.

Nothing in the preview acts. A link is drawn as a link and goes nowhere, and a picture in a note is named where it stands, not loaded: the preview is a reading of the file, and a click that took the window to a web page would be a click that sent something out.

A row found by meaning alone, with no word of the query in it, is asked which of its sentences answered - once it has been selected a moment - and that sentence is marked in the passage, as the list marks the one found in another language. See [One list from two halves](/nooma/concepts/hybrid/).

At its narrowest the window is the list alone, and a row's fragment is the preview there is room for. Everything below works there as well, by its keys.

## Opening at the place

`Enter`, a double click, or **Open** in the preview opens the document in the program that would open it anyway - the one the system has for files like it: the shell's association on Windows, `xdg-mime`'s default on Linux, Launch Services on macOS. When that program is one whose way of being told a place is known, it is told the place: an editor the line the passage starts on, a PDF reader the page. The button says what it will do - **Open at line 12**, **Open at page 4** - and its tooltip says in which program.

| Program | Told |
| --- | --- |
| Visual Studio Code, VSCodium, Cursor | `-g file:line` |
| Sublime Text, Zed | `file:line` |
| Notepad++, Kate, gedit | the line, by their own flags |
| Edge, Chrome, Brave, Vivaldi, Firefox | the PDF as a link to its page, `file:///…#page=4` |
| Adobe Acrobat and Reader, SumatraPDF, Okular, Evince, zathura | the page, by their own flags |

Any other program is handed the file, as a double click in the file manager would hand it, and the button says **Open**. A Word document or a book has no line or page anyone is told, and opens at its start.

A note in an Obsidian vault opens in Obsidian, whatever opens markdown elsewhere: the vault is Obsidian's. Obsidian is told the note by its own link, `obsidian://open`, with the heading the passage is under - as near a line as its links go. Which folders are vaults Obsidian says itself, in the list of vaults it keeps in its settings; a folder it does not list is not one it would open. Should the link find no Obsidian to take it, the note is handed to the system as any other file.

`Ctrl+Enter` - or **Show in folder** - shows the file in the file manager instead.

## A passage as a quote

`Ctrl+Shift+C` - or **Copy as quote** - puts the passage found on the clipboard as a markdown quote, with where it came from under it: for a chat, or for a note.

```markdown
> Open the vents at noon, when the sun is on the glass, and close them
> before the evening damp comes in.
>
> `/home/you/Notes/garden/greenhouse.md:10`
```

The file is said with its line where it has lines, with its page for a PDF, and with the headings the passage is under for a Word document or a book. The path is code, in backticks, so the backslashes of a Windows path stay as they are written. A note's passage is quoted in the markdown it is written in. Text selected in the preview is quoted instead of the whole passage.
