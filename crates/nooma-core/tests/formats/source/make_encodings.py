"""Build the text fixtures in tests/formats/encodings and tests/formats/kinds.

The files are committed; this script is how they were made, so they can be
made again:

    python make_encodings.py

encodings/ holds one short text in the encodings text files are found in -
encoded by Python's codecs, not by the decoder they test. kinds/ holds files
whose names say one thing and whose bytes another, made from the other
fixtures. Every word in them is synthetic.
"""

import pathlib
import shutil
import struct
import zipfile
import zlib

HERE = pathlib.Path(__file__).resolve().parent
FORMATS = HERE.parent
ENCODINGS = FORMATS / "encodings"
KINDS = FORMATS / "kinds"

RUSSIAN = (
    "Договор аренды склада на Портовой улице.\n"
    "Арендатор платит каждый месяц и хранит ключи от ворот у себя.\n"
    "Сторож обходит территорию дважды за ночь и записывает показания счётчиков.\n"
)
ENGLISH = "The warehouse on Harbour Street is rented by the month.\nThe watchman walks the yard twice a night.\n"
GERMAN = "Die Kaffeemaschine hat zwei Jahre Garantie.\nFür Reparaturen gilt die Größe der Rechnung, nicht die Länge des Gesprächs.\n"
JAPANESE = "倉庫の鍵は管理人が保管している。\n夜警は一晩に二度、敷地を見回る。\n"

# name: (text, codec). A byte order mark only where the codec writes one.
TEXTS = {
    "windows-1251.txt": (RUSSIAN, "cp1251"),
    "koi8-r.txt": (RUSSIAN, "koi8_r"),
    "ibm866.txt": (RUSSIAN, "cp866"),
    "windows-1252.txt": (GERMAN, "cp1252"),
    "shift_jis.txt": (JAPANESE, "shift_jis"),
    "utf-16le-no-mark.txt": (RUSSIAN + ENGLISH, "utf-16-le"),
    "utf-16be-mark.txt": (RUSSIAN + ENGLISH, "utf-16"),
    "utf-8-mark.md": ("# Склад\n\n" + RUSSIAN, "utf-8-sig"),
}


def png() -> bytes:
    """A one-pixel PNG."""

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    header = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(b"\x00\xff\x00\x00")) + chunk(b"IEND", b"")


def main() -> None:
    ENCODINGS.mkdir(exist_ok=True)
    for name, (text, codec) in TEXTS.items():
        data = text.encode(codec)
        if codec == "utf-16":
            # Python writes UTF-16 in the machine's order; this one is big-endian.
            data = b"\xfe\xff" + text.encode("utf-16-be")
        (ENCODINGS / name).write_bytes(data)
    # UTF-8 with one byte damaged, and UTF-8 cut off in the middle of a letter.
    damaged = bytearray((RUSSIAN + ENGLISH).encode("utf-8"))
    damaged[40] = 0xFF
    (ENCODINGS / "utf-8-damaged.txt").write_bytes(bytes(damaged))
    (ENCODINGS / "utf-8-cut.txt").write_bytes(RUSSIAN.encode("utf-8")[:-2])
    (ENCODINGS / "binary.txt").write_bytes(bytes(range(256)) * 8)

    KINDS.mkdir(exist_ok=True)
    # A PDF saved with no extension, as browsers save downloads.
    shutil.copyfile(FORMATS.parent / "pdf" / "handbook.pdf", KINDS / "invoice")
    # A Word document renamed to .pdf.
    shutil.copyfile(FORMATS / "lease.docx", KINDS / "report.pdf")
    # A note with no extension.
    (KINDS / "notes").write_bytes(ENGLISH.encode("utf-8"))
    # Pictures: one named a PDF, one named nothing.
    (KINDS / "picture.pdf").write_bytes(png())
    (KINDS / "photo").write_bytes(png())
    # A zip that is neither a Word document nor a book, named a book.
    with zipfile.ZipFile(KINDS / "archive.epub", "w") as archive:
        info = zipfile.ZipInfo("readme.txt", (2026, 1, 1, 0, 0, 0))
        archive.writestr(info, ENGLISH)


if __name__ == "__main__":
    main()
