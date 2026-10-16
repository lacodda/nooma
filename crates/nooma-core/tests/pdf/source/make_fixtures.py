"""Build the PDF fixtures in tests/pdf from handbook.html.

The PDFs are committed; this script is how they were made, so they can be made
again. It needs Chrome (for a real font program and a real text layer, the way
most PDFs are printed) and pypdf (for the outline, the metadata and the
encryption):

    python make_fixtures.py [path-to-chrome]

Every byte of text in them is synthetic.
"""

import pathlib
import subprocess
import sys
import tempfile

from pypdf import PdfReader, PdfWriter

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent
CHROME = sys.argv[1] if len(sys.argv) > 1 else r"C:\Program Files\Google\Chrome\Application\chrome.exe"


def printed() -> PdfReader:
    """handbook.html, printed by Chrome."""
    raw = pathlib.Path(tempfile.mkdtemp()) / "handbook.pdf"
    subprocess.run(
        [CHROME, "--headless=new", "--disable-gpu", "--no-pdf-header-footer", f"--print-to-pdf={raw}", (HERE / "handbook.html").as_uri()],
        check=True,
        capture_output=True,
    )
    return PdfReader(raw)


def handbook(reader: PdfReader) -> PdfWriter:
    """Three pages with an outline and a title in the metadata."""
    writer = PdfWriter(clone_from=reader)
    writer.add_metadata({"/Title": "Coffee Machine Handbook", "/Producer": "nooma fixtures"})
    writer.add_outline_item("Warranty", 0)
    russian = writer.add_outline_item("Гарантия", 1)
    writer.add_outline_item("Сроки ремонта", 1, parent=russian)
    writer.add_outline_item("Repairs", 2)
    return writer


def write(writer: PdfWriter, name: str) -> None:
    with open(OUT / name, "wb") as file:
        writer.write(file)


def main() -> None:
    reader = printed()
    write(handbook(reader), "handbook.pdf")

    # Locked by an owner password only: anyone may open it, and a reader has
    # to decrypt it with the empty user password. The old RC4 cipher, as
    # older producers wrote it.
    locked = handbook(reader)
    locked.encrypt(user_password="", owner_password="owner", algorithm="RC4-128")
    write(locked, "owner-locked.pdf")

    # Opens only with the password.
    protected = handbook(reader)
    protected.encrypt(user_password="secret", owner_password="secret", algorithm="AES-256")
    write(protected, "password.pdf")

    # Pages with no text on them, as a scan has.
    scan = PdfWriter()
    scan.add_blank_page(width=595, height=842)
    scan.add_blank_page(width=595, height=842)
    scan.add_metadata({"/Title": "Scanned lease", "/Producer": "nooma fixtures"})
    write(scan, "scan.pdf")


if __name__ == "__main__":
    main()
