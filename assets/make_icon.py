"""Draws the app icon: two linked rings, for two launchers joined together.

Original artwork - deliberately not based on the Battle.net or Steam logos.

    python assets/make_icon.py      (needs Pillow: pip install pillow)

Writes assets/icon.png (256 px, used for the window) and assets/icon.ico (16-256 px, the .exe).
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).parent
S = 4096  # drawn large, then scaled down for smooth edges
U = S / 1024  # design units are on a 1024 grid


def u(v):
    return v * U


def ring(draw, cx, cy, r, width, color, start=0, end=360):
    box = [u(cx - r), u(cy - r), u(cx + r), u(cy + r)]
    draw.arc(box, start, end, fill=color, width=round(u(width)))


def main():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Rounded-square tile with a top-to-bottom navy gradient.
    top, bottom = (31, 47, 82), (12, 18, 34)
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([u(24), u(24), u(1000), u(1000)], radius=u(220), fill=255)
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    blue, steel = (79, 163, 255, 255), (214, 224, 233, 255)
    r, w = 215, 78
    left, right = (392, 512), (632, 512)
    gap = tuple(round(a + (b - a) * 0.5) for a, b in zip(top, bottom)) + (255,)

    # Where the two ring centrelines cross: straight above and below the midpoint.
    half = (right[0] - left[0]) / 2
    rise = math.sqrt(r * r - half * half)
    left_top = math.degrees(math.atan2(-rise, half))  # angle on the left ring (PIL: clockwise)
    right_bottom = math.degrees(math.atan2(rise, -half))  # angle on the right ring

    ring(d, *right, r, w, steel)
    ring(d, *left, r, w, blue)
    # Chain-link look: each ring passes over the other at one crossing (left at the top, right at
    # the bottom), with a background-coloured band either side to separate them.
    ring(d, *left, r, w + 40, gap, left_top - 16, left_top + 16)
    ring(d, *left, r, w, blue, left_top - 24, left_top + 24)
    ring(d, *right, r, w + 40, gap, right_bottom - 16, right_bottom + 16)
    ring(d, *right, r, w, steel, right_bottom - 24, right_bottom + 24)

    big = img.resize((1024, 1024), Image.LANCZOS)
    big.resize((256, 256), Image.LANCZOS).save(OUT / "icon.png")
    big.save(OUT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])


if __name__ == "__main__":
    main()
