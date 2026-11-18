#!/usr/bin/env python3
"""Generate LobsterOS anti-aliased font atlases (assets/fonts/*.bin + index.rs).

Inputs: Inter variable TTF (one-time download, SHA-256 pinned), DejaVu Sans /
Sans Bold (per-glyph fallback) and DejaVu Sans Mono (monospace face).
Output is deterministic: the same inputs always produce identical bytes.

Usage: tools/fontgen.py [--inter PATH] [--out DIR] [--download]
"""
import argparse
import hashlib
import struct
import sys
import urllib.request
from pathlib import Path

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

INTER_URL = "https://cdn.jsdelivr.net/gh/google/fonts@main/ofl/inter/Inter%5Bopsz,wght%5D.ttf"
INTER_SHA256 = "29160a80ff49ddcab2c97711247e08b1fab27a484a329ce8b813d820dc559031"
DEJAVU_DIR = Path("/usr/share/fonts/truetype/dejavu")
DEJAVU_SANS = DEJAVU_DIR / "DejaVuSans.ttf"
DEJAVU_BOLD = DEJAVU_DIR / "DejaVuSans-Bold.ttf"
DEJAVU_MONO = DEJAVU_DIR / "DejaVuSansMono.ttf"

MAGIC = b"LFA1"
HEADER_FMT = "<4sBBBBBBH"  # magic, size, cell_w, ascent, descent, line_h, pad, count
RECORD_FMT = "<HBbbBBBI"  # cp, advance, bearing_x, top (signed), w, h, pad, offset
INTER_OPSZ_MIN, INTER_OPSZ_MAX = 14, 32
WEIGHT_REGULAR, WEIGHT_SEMIBOLD = 400, 600
UI_REGULAR_SIZES = (11, 12, 13, 14, 16, 20, 24)
UI_SEMIBOLD_SIZES = (12, 13, 14, 16, 20, 24)
# (pixel size, cell width, cell height, baseline row): 8x16 plus its 2x
# twin for the terminal's large-font mode. A 10x20 cell was dropped to stay
# inside the 400 KiB data budget.
MONO_CELLS = ((13, 8, 16, 12), (26, 16, 32, 25))
RENDER_PAD = 8
SYMBOLS = "•…–—‘’“”←↑→↓✓✕×÷−±°⌫▲▼◀▶☰⏻⏎⇧⌘★●"


def code_points():
    """Every code point the atlases must cover, ascending."""
    points = set(range(0x20, 0x7F)) | set(range(0xA0, 0x100))
    points |= {ord(c) for c in SYMBOLS}
    return sorted(points)


class Face:
    """One TrueType file plus its cmap, at a fixed pixel size."""

    def __init__(self, path, size, variation=None):
        self.cmap = TTFont(str(path)).getBestCmap()
        self.font = ImageFont.truetype(str(path), size)
        if variation:
            self.font.set_variation_by_axes(variation)

    def has(self, cp):
        return cp in self.cmap


def sha256_of(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def ensure_inter(path, download):
    """Return the verified Inter path, downloading once when asked."""
    path = Path(path)
    if not path.exists() and download:
        path.parent.mkdir(parents=True, exist_ok=True)
        urllib.request.urlretrieve(INTER_URL, path)
    if not path.exists():
        sys.exit(f"missing {path}; rerun with --download")
    digest = sha256_of(path)
    if digest != INTER_SHA256:
        sys.exit(f"Inter hash mismatch: {digest} != {INTER_SHA256}")
    return path


def render_glyph(face, cp):
    """Render `cp` anchored at its baseline; returns (image, origin_x, advance)."""
    ch = chr(cp)
    side = int(face.font.size * 3) + 2 * RENDER_PAD
    img = Image.new("L", (side, side), 0)
    origin = (RENDER_PAD + side // 3, side // 2)
    ImageDraw.Draw(img).text(origin, ch, fill=255, font=face.font, anchor="ls")
    return img, origin, face.font.getlength(ch)


def crop_glyph(img, origin):
    """Tight crop; returns (bytes, bearing_x, rows_above_baseline, w, h)."""
    box = img.getbbox()
    if box is None:
        return b"", 0, 0, 0, 0
    left, top, right, bottom = box
    data = img.crop(box).tobytes()
    return data, left - origin[0], origin[1] - top, right - left, bottom - top


class Glyph:
    def __init__(self, cp, advance, bearing_x, above, w, h, data):
        self.cp, self.advance, self.bearing_x = cp, advance, bearing_x
        self.above, self.w, self.h, self.data = above, w, h, data


def ui_glyph(faces, cp):
    """Render with the first face that has the code point."""
    for face in faces:
        if face.has(cp):
            img, origin, adv = render_glyph(face, cp)
            data, bx, above, w, h = crop_glyph(img, origin)
            return Glyph(cp, max(1, round(adv)), bx, above, w, h, data)
    return None


def mono_glyph(faces, cp, cell_w, cell_h, baseline):
    """Render centred in the cell and clipped to it."""
    for face in faces:
        if not face.has(cp):
            continue
        img, origin, adv = render_glyph(face, cp)
        shift = (cell_w - round(adv)) // 2 if face is not faces[0] else 0
        canvas = Image.new("L", (cell_w, cell_h), 0)
        canvas.paste(img, (-origin[0] + shift, baseline - origin[1]))
        data, bx, above, w, h = crop_glyph(canvas, (0, baseline))
        return Glyph(cp, cell_w, bx, above, w, h, data)
    return None


def pack_atlas(size, cell_w, ascent, descent, glyphs):
    """Serialize glyphs into the LFA1 binary layout."""
    line_h = ascent + descent
    records, pixels = [], bytearray()
    for g in glyphs:
        top = ascent - g.above
        assert -128 <= top < 128, (g.cp, top)
        records.append(struct.pack(RECORD_FMT, g.cp, g.advance, g.bearing_x, top, g.w, g.h, 0, len(pixels)))
        pixels += g.data
    header = struct.pack(HEADER_FMT, MAGIC, size, cell_w, ascent, descent, line_h, 0, len(glyphs))
    return header + b"".join(records) + bytes(pixels)


def ui_atlas(inter, size, weight, fallback):
    opsz = min(max(size, INTER_OPSZ_MIN), INTER_OPSZ_MAX)
    primary = Face(inter, size, [opsz, weight])
    faces = [primary, Face(fallback, size)]
    ascent, descent = primary.font.getmetrics()
    glyphs = [g for g in (ui_glyph(faces, cp) for cp in code_points()) if g]
    return pack_atlas(size, 0, ascent, descent, glyphs)


def mono_atlas(spec):
    size, cell_w, cell_h, baseline = spec
    faces = [Face(DEJAVU_MONO, size), Face(DEJAVU_SANS, size)]
    advance = round(faces[0].font.getlength("M"))
    if advance != cell_w:
        sys.exit(f"mono size {size} advance {advance} != cell {cell_w}")
    glyphs = [mono_glyph(faces, cp, cell_w, cell_h, baseline) for cp in code_points()]
    glyphs = [g for g in glyphs if g]
    return pack_atlas(size, cell_w, baseline, cell_h - baseline, glyphs)


def missing_points(faces):
    return [cp for cp in code_points() if not any(f.has(cp) for f in faces)]


def build_all(inter):
    """Yield (stem, face_name, size, blob) for every atlas."""
    for size in UI_REGULAR_SIZES:
        yield f"ui-regular-{size}", "Ui", size, ui_atlas(inter, size, WEIGHT_REGULAR, DEJAVU_SANS)
    for size in UI_SEMIBOLD_SIZES:
        yield f"ui-semibold-{size}", "UiBold", size, ui_atlas(inter, size, WEIGHT_SEMIBOLD, DEJAVU_BOLD)
    for spec in MONO_CELLS:
        yield f"mono-{spec[1]}x{spec[2]}", "Mono", spec[0], mono_atlas(spec)


def index_source(entries):
    """Rust source listing every atlas via include_bytes!."""
    lines = ["// Generated by tools/fontgen.py. Do not edit.", ""]
    for stem, face, size, _ in entries:
        lines.append(f'    AtlasData {{ face: Face::{face}, size: {size}, bytes: include_bytes!("{stem}.bin") }},')
    body = "\n".join(lines[2:])
    return f"{lines[0]}\n[\n{body}\n]\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parent.parent
    parser.add_argument("--inter", default=str(root / "assets/fonts/.cache/Inter.ttf"))
    parser.add_argument("--out", default=str(root / "assets/fonts"))
    parser.add_argument("--download", action="store_true")
    args = parser.parse_args()
    inter = ensure_inter(args.inter, args.download)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    entries = list(build_all(inter))
    for stem, _, _, blob in entries:
        (out / f"{stem}.bin").write_bytes(blob)
    (out / "index.rs").write_text(index_source(entries))
    print(f"wrote {len(entries)} atlases, {sum(len(e[3]) for e in entries)} bytes")
    uncovered = missing_points([Face(inter, 12), Face(DEJAVU_SANS, 12)])
    print("unrenderable:", [f"U+{c:04X}" for c in uncovered] or "none")


if __name__ == "__main__":
    main()
