"""Regenerate KBOX PNG app icons from the approved brand master."""

from pathlib import Path

from PIL import Image


ICONS_DIR = Path(__file__).resolve().parent
MASTER = ICONS_DIR.parent.parent / "brand-concepts" / "kbox-app-icon.png"
SIZES = {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}

with Image.open(MASTER) as source:
    source = source.convert("RGBA")
    for filename, size in SIZES.items():
        source.resize((size, size), Image.Resampling.LANCZOS).save(
            ICONS_DIR / filename,
            optimize=True,
        )

print(f"Generated KBOX icons from {MASTER}")
