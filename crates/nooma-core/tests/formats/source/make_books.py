"""Build the EPUB fixtures in tests/formats from book.md.

The books are committed; this script is how they were made, so they can be
made again. pandoc writes the books themselves - an EPUB 3 with a navigation
document and an EPUB 2 with an NCX, the two shapes books come in - and the
variants are the pandoc book with one thing changed, written here:

    python make_books.py [path-to-pandoc]

Every word in them is synthetic.
"""

import os
import pathlib
import subprocess
import sys
import zipfile

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent
PANDOC = sys.argv[1] if len(sys.argv) > 1 else "pandoc"

# The same bytes every time: pandoc dates the book and its zip entries by this.
ENV = {**os.environ, "SOURCE_DATE_EPOCH": "1767225600"}
STAMP = (2026, 1, 1, 0, 0, 0)


def pandoc(target: str, *options: str) -> None:
    subprocess.run([PANDOC, str(HERE / "book.md"), "-o", str(OUT / target), *options], check=True, env=ENV)


def rewrite(source: str, target: str, change) -> None:
    """Copy a book, entry by entry, letting `change` drop, edit or add entries.

    `change(name, data)` returns the entry's new bytes, or None to drop it;
    it is called once more with name None to return entries to add.
    """
    with zipfile.ZipFile(OUT / source) as book, zipfile.ZipFile(OUT / target, "w") as out:
        for info in book.infolist():
            data = change(info.filename, book.read(info))
            if data is not None:
                write(out, info.filename, data)
        for name, data in change(None, None) or []:
            write(out, name, data)


def write(out: zipfile.ZipFile, name: str, data: bytes) -> None:
    # The mimetype entry is stored, as the format requires; the rest deflated.
    info = zipfile.ZipInfo(name, STAMP)
    info.compress_type = zipfile.ZIP_STORED if name == "mimetype" else zipfile.ZIP_DEFLATED
    out.writestr(info, data)


def without_contents(name, data):
    """No navigation document and no NCX: the book's headings must do."""
    if name is None:
        return None
    if name.endswith("nav.xhtml") or name.endswith("toc.ncx"):
        return None
    if name.endswith("content.opf"):
        text = data.decode("utf-8")
        text = "\n".join(line for line in text.splitlines() if 'id="nav"' not in line and 'id="ncx"' not in line)
        return text.replace('<spine toc="ncx">', "<spine>").encode("utf-8")
    return data


def encryption(*entries: tuple[str, str]) -> bytes:
    data = "".join(
        f'<EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#"><EncryptionMethod Algorithm="{algorithm}"/>'
        f'<CipherData><CipherReference URI="{uri}"/></CipherData></EncryptedData>'
        for algorithm, uri in entries
    )
    return f'<?xml version="1.0"?><encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container">{data}</encryption>'.encode()


def locked(name, data):
    """A chapter enciphered, as a store's DRM leaves a book: the bytes of the
    page are noise, and META-INF/encryption.xml says which cipher made it."""
    if name is None:
        return [("META-INF/encryption.xml", encryption(("http://www.w3.org/2001/04/xmlenc#aes128-cbc", "EPUB/text/ch001.xhtml")))]
    if name.endswith("ch001.xhtml"):
        return bytes((byte * 167 + 13) % 256 for byte in data)
    return data


def fonts_only(name, data):
    """Only a font obfuscated, as many books are: the book reads as any other."""
    if name is None:
        return [
            ("META-INF/encryption.xml", encryption(("http://www.idpf.org/2008/embedding", "EPUB/fonts/serif.otf"))),
            ("EPUB/fonts/serif.otf", bytes(range(256)) * 4),
        ]
    return data


def main() -> None:
    pandoc("book.epub")
    pandoc("book-epub2.epub", "-t", "epub2")
    rewrite("book.epub", "no-contents.epub", without_contents)
    rewrite("book.epub", "locked.epub", locked)
    rewrite("book.epub", "fonts.epub", fonts_only)


if __name__ == "__main__":
    main()
