#!/usr/bin/env python3
"""Builds shared WPF/Tauri Windows icons from the repository's icon.png.

Windows asks for several icon sizes and picks one per context: 16 px in the
notification area, 24/32 px in the title bar and Explorer's small views, 256 px
in Explorer's extra-large view. Shipping a single downscaled frame makes the
small ones mushy, so every size is resampled from the original.

The mark is also trimmed to its own ink. icon.png is a square canvas with a wide,
short drawing centred in it — measured at 77 % wide by 50 % tall — so a quarter of
every square icon slot was empty and the glyph read as undersized next to other
apps, most visibly in the 32 px taskbar button. Cropping the transparent margin
and re-centring with a small uniform border buys about 20 % more glyph for the
same slot without redrawing anything. icon.png itself is left untouched: it stays
the source of truth, and the .ico is the build product.

The WPF fallback and Tauri desktop both embed the generated .ico. The web UI
uses the matching tightened PNG. Run this only when icon.png changes.

Run:  <python> packaging/make-icon.py
"""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:  # pragma: no cover - operator guidance, not a code path
    sys.exit("Pillow is required: <python> -m pip install Pillow")

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "icon.png"
TARGETS = [
    ROOT / "src" / "AnPod.Desktop" / "Assets" / "icon.ico",
    ROOT / "src-tauri" / "icons" / "icon.ico",
]
TAURI_ICON_DIR = ROOT / "src-tauri" / "icons"
FRONTEND_ICON = ROOT / "frontend" / "public" / "icon.png"

# 16 tray, 24/32 title bar, 48/64 Explorer large, 128/256 extra large.
SIZES = [(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]

MINIMUM_SOURCE_EDGE = 256

# Fraction of the canvas the trimmed drawing is allowed to occupy on its longest
# side. 0.90 leaves a visible border at 32 px (the taskbar size), where 0.94 put
# the anti-aliased edge right on the slot boundary and read as bleeding into it.
INK_FILL = 0.90

# Pixels this faint are invisible, but a naive alpha bbox still counts them as
# ink. icon.png carries a whole-canvas haze at alpha 1, which made the bbox the
# entire image and silently disabled the trim; a small threshold ignores the haze
# and keeps the anti-aliased edge of the real drawing.
INK_ALPHA_THRESHOLD = 8


def tighten(image: Image.Image) -> Image.Image:
    """Crops to the drawing's ink and re-centres it in a square canvas.

    Returns the image unchanged when it is already full-bleed or fully
    transparent, so a future icon that needs no trimming is not silently
    rescaled.
    """
    mask = image.getchannel("A").point(
        lambda value: 255 if value > INK_ALPHA_THRESHOLD else 0
    )
    box = mask.getbbox()
    if box is None:
        return image

    ink = image.crop(box)
    longest = max(ink.size)
    canvas = round(longest / INK_FILL)
    if canvas >= max(image.size):
        return image

    square = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    square.paste(ink, ((canvas - ink.width) // 2, (canvas - ink.height) // 2))
    return square


def main() -> int:
    if not SOURCE.exists():
        sys.exit(f"missing source image: {SOURCE}")

    for target in TARGETS:
        target.parent.mkdir(parents=True, exist_ok=True)
    TAURI_ICON_DIR.mkdir(parents=True, exist_ok=True)

    with Image.open(SOURCE) as source:
        image = source.convert("RGBA")
        width, height = image.size
        if min(width, height) < MINIMUM_SOURCE_EDGE:
            sys.exit(
                f"source image is {width}x{height}; need at least "
                f"{MINIMUM_SOURCE_EDGE}x{MINIMUM_SOURCE_EDGE} so the 256 px "
                f"frame is not upscaled"
            )

        # Crop first, then let Pillow resample each requested size from the
        # trimmed original — resampling the trimmed result again would soften the
        # small frames for no reason.
        trimmed = tighten(image)
        print(f"trim   : {width}x{height} -> {trimmed.width}x{trimmed.height}")

        # The 256 px frame is stored as PNG (Vista and later), the rest as BMP.
        for target in TARGETS:
            trimmed.save(target, format="ICO", sizes=SIZES)
        trimmed.save(TAURI_ICON_DIR / "icon.png", format="PNG")
        for edge in (32, 64, 128, 256):
            trimmed.resize((edge, edge), Image.Resampling.LANCZOS).save(
                TAURI_ICON_DIR / f"{edge}x{edge}.png", format="PNG"
            )
        trimmed.resize((256, 256), Image.Resampling.LANCZOS).save(
            TAURI_ICON_DIR / "128x128@2x.png", format="PNG"
        )
        FRONTEND_ICON.parent.mkdir(parents=True, exist_ok=True)
        trimmed.resize((256, 256), Image.Resampling.LANCZOS).save(FRONTEND_ICON, format="PNG")

    with Image.open(TARGETS[0]) as written:
        frames = sorted(written.ico.sizes())

    print(f"source : {SOURCE.name} {width}x{height}")
    for target in TARGETS:
        print(f"target : {target.relative_to(ROOT)} ({target.stat().st_size} bytes)")
    print(f"frames : {', '.join(f'{w}x{h}' for w, h in frames)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
