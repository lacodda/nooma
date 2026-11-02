---
title: Keeping up with the folders
description: How nooma keeps its index current as files change - the watcher that names what changed, the update that reads only that, what a watcher can miss, and how the work stays out of the way of yours.
---

A search that does not find the note saved a minute ago is a search that seems broken. Since v0.13.0 the window - and `nooma index --watch` - watches the folders it searches, and reads what changes as it changes: a note saved is found a moment later.

## Only what changed

Every update reads only what changed, and has since the first version: a file whose size and modification time are as they were is not opened, and one whose bytes hash as before is not parsed again. What watching adds is where to look. The system says which paths changed - ReadDirectoryChangesW on Windows, inotify on Linux, FSEvents on macOS - and the update walks down to those paths and into them, and leaves the rest of the index as it is.

It walks *down* to them, from the top of the folder: the rules that leave a file out - a `.gitignore`, a `.nooma-ignore` - may sit in any folder above the one that changed, and a walk that started at the changed path would not know them. So a change is read by the same rules, and the same code, as a whole update reads it. A change to a file of rules is a change to everything in its folder.

Changes come in bursts - an editor saves a note as a write, a rename and another write; a copy writes a book in a thousand pieces - so they are gathered until the folders have been quiet for half a second, or for five seconds at most under a steady stream, and read as one update. What no update would read is not even gathered: hidden folders - a vault's `.obsidian`, saved every few seconds, a repository's `.git` - and the library's own files. An update that finds nothing changed writes nothing.

## What a watcher can miss

The system's word is trusted for one thing only: where to look. What changed is read off the disk, as it always is, so a watcher can cost freshness and never correctness. It can miss a change: its buffer overflows in a burst of thousands, a network share does not report, a drive unplugged and plugged back in is no longer watched. So the window also reads the whole of every folder when it opens, when a folder is added, and when it comes back into focus two minutes or more after it last did - the moment a person is about to search.

## Out of the way

Indexing nobody is waiting for shares the machine with whatever its owner is doing, and must never be in the way of it. Three things keep it out:

- **Few threads.** Half the cores, at most four, for reading files and for the model alike.
- **Below the person's programs.** The threads that read files and compute vectors in the background run at a low priority: background mode on Windows, which lowers their claim on the disk as well as the processor; a niceness of 10 on Linux; the background quality of service on macOS. The system gives them what nothing else wants.
- **Not on battery.** While a laptop runs on its battery, the background work holds where it is - before the next file, before the next batch of passages - and goes on by itself when the power is back; the status line says it is paused. An hour of reading scans is an hour of battery a person would rather keep. A `nooma index` typed in a terminal is someone waiting, and it is never held; neither is a search.

There is no dial for any of it. A limit to tune is a question every person would have to answer, and these answers do not depend on the person.

## What could not be read

A file found and not read - too large, under a password or DRM, damaged, no document at all though its name says one - is kept in the index's records with why, as a file read is kept with what it holds. It is not tried again until it changes or a newer nooma reads it, so a password-locked PDF is not opened on every update - except a file another program holds, which may be let go of, and is tried every time. And the index can say at any moment what it lacks, not only in the report of the update that met it: `nooma health` lists it, and the window's status line counts it and opens the same list, grouped by why. So does what is held only in part: scans found by their names, pages of pictures that could not be read, encodings that were guessed, folders out of reach. See [`health`](/nooma/reference/find/#health).

## What it costs

Measured on 5,000 synthetic notes, on an i7-1360P laptop on mains power - 16 threads, of which nooma takes four:

| What | Took |
| --- | --- |
| The first update, reading every note | 26 s |
| A whole update that found nothing changed: a walk, and a look at each file's size and time | 73 ms, and nothing written |
| An update over one changed note, under `nooma index --watch` | 0.8 s - the median of seven, from 0.3 to 2.9 s |
| From the note written to the update done, the half second of quiet counted in | 1.4 s |

Most of an update over one note is the index committing to disk, at the low priority background work runs at: the same update in the foreground takes 0.4 s. The longest ones are the full-text index merging the small pieces earlier commits left - work any index that is written to does now and then, done here when nothing else wants the disk.
