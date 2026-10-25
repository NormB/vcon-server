#!/usr/bin/env python3
"""Draws the logo files from assets/brand/mark.svg.

Writes the favicon (SVG and ICO), the Apple touch icon, the social preview
image and the two README lockups. The wordmark is Inter (SIL Open Font
License) turned to outlines, so no font is embedded and none is needed to
show it.

Needs resvg on PATH and the fontTools Python package; the Inter files are
found with fc-match. Run from the repository root:

    python3 scripts/icons.py          # write every file
    python3 scripts/icons.py --check  # exit 1 if any file differs
"""

import struct
import subprocess
import sys
import tempfile
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent.parent
MARK = ROOT / "assets/brand/mark.svg"
STATIC = ROOT / "website/static"

TILE = "#0f172a"
PAGE_DARK = "#0b1120"
TEXT_LIGHT = "#0f172a"
TEXT_DARK = "#e2e8f0"
MUTED_DARK = "#94a3b8"
TAGLINE = ("Store vCons, encrypted,", "for as long as they must be kept")


def font(style):
    path = subprocess.run(
        ["fc-match", "-f", "%{file}", f"Inter:style={style}"],
        check=True, capture_output=True, text=True,
    ).stdout
    face = TTFont(path)
    if face["name"].getDebugName(1).split()[0] != "Inter":
        sys.exit(f"fc-match gave {path}, not Inter")
    return face


def outline(face, text, size, x, baseline):
    """The text as one SVG path, `size` pixels to the em, left edge at x."""
    scale = size / face["head"].unitsPerEm
    glyphs = face.getGlyphSet()
    cmap = face.getBestCmap()
    pen = SVGPathPen(glyphs)
    for char in text:
        name = cmap[ord(char)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, x, baseline)))
        x += glyphs[name].width * scale
    return pen.getCommands(), x


def mark_body():
    """The mark's drawing without its <svg> wrapper, on a 32-unit grid."""
    text = MARK.read_text()
    return text[text.index(">") + 1:text.rindex("</svg>")].strip()


def placed(x, y, size, body):
    return f'<g transform="translate({x} {y}) scale({size / 32})">{body}</g>'


def svg(width, height, body):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}">{body}</svg>\n'
    )


def lockup(text_colour, semibold):
    words, end = outline(semibold, "vcon-server", 40, 76, 47)
    width = round(end) + 4
    return svg(width, 64, placed(0, 0, 64, mark_body()) + f'<path d="{words}" fill="{text_colour}"/>')


def social(semibold, regular, width, height):
    """The mark, the name and the tagline, centred on the height."""
    left = (width - 1000) // 2
    words, _ = outline(semibold, "vcon-server", 112, left + 280, height // 2 - 28)
    tagline = "".join(
        outline(regular, line, 38, left + 284, height // 2 + 42 + 50 * i)[0] for i, line in enumerate(TAGLINE)
    )
    return svg(
        width, height,
        f'<rect width="{width}" height="{height}" fill="{PAGE_DARK}"/>'
        + placed(left, (height - 240) // 2, 240, mark_body())
        + f'<path d="{words}" fill="{TEXT_DARK}"/><path d="{tagline}" fill="{MUTED_DARK}"/>',
    )


def square_mark():
    """The mark on a square tile, for icons the platform rounds itself."""
    body = mark_body().replace('rx="6" ', "", 1)
    return svg(32, 32, body)


def render(svg_text, width, height):
    with tempfile.TemporaryDirectory() as tmp:
        source, target = Path(tmp, "in.svg"), Path(tmp, "out.png")
        source.write_text(svg_text)
        subprocess.run(["resvg", "-w", str(width), "-h", str(height), source, target], check=True)
        return target.read_bytes()


def ico(pngs):
    """An ICO file holding PNG images, which every current browser reads."""
    header = struct.pack("<HHH", 0, 1, len(pngs))
    offset = len(header) + 16 * len(pngs)
    entries, data = b"", b""
    for size, png in pngs:
        entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    return header + entries + data


def outputs():
    semibold, regular = font("SemiBold"), font("Regular")
    mark = MARK.read_text()
    return {
        STATIC / "favicon.svg": mark.encode(),
        STATIC / "favicon.ico": ico([(s, render(mark, s, s)) for s in (16, 32, 48)]),
        STATIC / "apple-touch-icon.png": render(square_mark(), 180, 180),
        STATIC / "og-image.png": render(social(semibold, regular, 1200, 630), 1200, 630),
        ROOT / "assets/brand/social-preview.png": render(social(semibold, regular, 1280, 640), 1280, 640),
        ROOT / "assets/brand/lockup-light.svg": lockup(TEXT_LIGHT, semibold).encode(),
        ROOT / "assets/brand/lockup-dark.svg": lockup(TEXT_DARK, semibold).encode(),
    }


def main():
    check = sys.argv[1:] == ["--check"]
    stale = []
    for path, content in outputs().items():
        if check:
            if not path.exists() or path.read_bytes() != content:
                stale.append(path.relative_to(ROOT))
        else:
            path.write_bytes(content)
    if stale:
        sys.exit("out of date, run scripts/icons.py: " + ", ".join(map(str, stale)))


if __name__ == "__main__":
    main()
