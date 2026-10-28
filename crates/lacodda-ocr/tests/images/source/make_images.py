"""Build the pictures in tests/images from the pages in this folder.

The pictures are committed; this script is how they were made, so they can be
made again. It needs Chrome (to set the text in real fonts, the way a screen
or a printer does) and Pillow (to turn a page into a scan, a photo, a
picture with no background):

    python make_images.py [path-to-chrome]

Every word in them is synthetic.
"""

import io
import pathlib
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent
CHROME = sys.argv[1] if len(sys.argv) > 1 else r"C:\Program Files\Google\Chrome\Application\chrome.exe"


def shot(page: str, width: int, height: int, scale: float = 1) -> Image.Image:
    """A page as Chrome draws it into a window of this size."""
    out = pathlib.Path(tempfile.mkdtemp()) / "shot.png"
    subprocess.run(
        [
            CHROME,
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            f"--force-device-scale-factor={scale}",
            f"--window-size={width},{height}",
            f"--screenshot={out}",
            (HERE / page).as_uri(),
        ],
        check=True,
        capture_output=True,
    )
    with Image.open(out) as image:
        return image.convert("RGBA")


def scanned(page: Image.Image) -> Image.Image:
    """A printed page as a scanner gives it back: grey, a little turned, a
    little soft, with the paper's tone behind the ink."""
    grey = page.convert("L")
    grey = grey.point(lambda v: 30 + v * 0.82)
    grey = grey.rotate(0.6, resample=Image.BICUBIC, expand=False, fillcolor=239)
    return grey.filter(ImageFilter.GaussianBlur(0.6))


def jpeg(image: Image.Image, quality: int, **extra) -> bytes:
    buffer = io.BytesIO()
    image.convert("RGB").save(buffer, "JPEG", quality=quality, **extra)
    return buffer.getvalue()


def main() -> None:
    # A letter printed at 200 dots an inch: an A4 page is 1654 by 2339.
    letter_en = shot("letter-en.html", 1654, 2339)
    letter_ru = shot("letter-ru.html", 1654, 2339)
    letter_es = shot("letter-es.html", 1654, 2339)
    scanned(letter_en).save(OUT / "letter-en.png", optimize=True)
    scanned(letter_ru).save(OUT / "letter-ru.png", optimize=True)
    scanned(letter_es).save(OUT / "letter-es.png", optimize=True)

    # Both scans as one PDF of pictures and nothing else, as a scanner
    # writes one: nooma's tests read it page by page.
    pages = [scanned(letter_en).convert("RGB"), scanned(letter_ru).convert("RGB")]
    pages[0].save(OUT / "scan.pdf", "PDF", resolution=200, save_all=True, append_images=pages[1:])

    # A screen at 100%: small type, a dark theme beside a light one.
    shot("screen.html", 1280, 720).save(OUT / "screen.png", optimize=True)

    # A photo of a page taken with the phone held sideways: the pixels lie on
    # their side, and the EXIF orientation says how to stand them up.
    photo = letter_en.convert("RGB").resize((1240, 1754)).rotate(90, expand=True)
    exif = Image.Exif()
    exif[0x0112] = 6  # Rotate 90 CW to display.
    (OUT / "photo-sideways.jpg").write_bytes(jpeg(photo, 85, exif=exif))

    # Black text on nothing at all: a picture cut out of a document with its
    # background left transparent.
    cut = shot("cutout.html", 900, 260)
    pixels = cut.load()
    for y in range(cut.height):
        for x in range(cut.width):
            r, g, b, _ = pixels[x, y]
            ink = 255 - (r + g + b) // 3
            pixels[x, y] = (0, 0, 0, ink)
    cut.save(OUT / "transparent.png", optimize=True)

    # A line of Spanish whose letters an English reader reads as English
    # letters - `mañana` as `mafiana` - with no digit to give it away.
    shot("accents.html", 1240, 160).convert("L").save(OUT / "accents.png", optimize=True)

    # A picture with no text in it.
    picture = Image.new("RGB", (800, 600), (96, 140, 190))
    draw = ImageDraw.Draw(picture)
    draw.ellipse((260, 140, 540, 420), fill=(236, 196, 80))
    draw.rectangle((0, 470, 800, 600), fill=(70, 120, 60))
    (OUT / "no-text.jpg").write_bytes(jpeg(picture, 90))


if __name__ == "__main__":
    main()
