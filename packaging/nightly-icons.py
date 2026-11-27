#!/usr/bin/env python3
"""Derive TextureCraft's Nightly icons from the committed stable renders.

The Nightly icon is the same artwork on a deeper background — no badge, crescent or letter. Each
pixel is fitted as a blend of the tile's background and one of the two engraved foregrounds (ink,
paper); only the background's contribution is replaced, so the engraving and the alpha padding are
untouched. Every frame comes from the committed stable master of its own size (the `.ico` and
`.icns` frames, the hicolor PNGs and the 1024 master), never from an upscale.

    packaging/nightly-icons.py           # write the Nightly assets
    packaging/nightly-icons.py --check   # fail if the committed Nightly assets differ

No third-party packages: zlib and struct only, as the build server has no Pillow or ImageMagick.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "assets" / "app-icon"
STABLE_ID = "ai.storyteller.texturecraft"

# The palette the artwork is drawn in, read from the tile itself (the three colours it is made of).
INK = (11, 11, 12)
PAPER = (239, 233, 220)
STABLE_BG = (194, 103, 43)
NIGHTLY_BG = (100, 52, 24)

# How far a pixel may sit from the fitted blend before it is left alone (anti-aliased edges onto the
# transparent margin, and the few pixels that shade the tile itself).
RESIDUAL = 24
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"

# Sizes the Nightly set carries (the 256 one is also the window and package icon). They live in
# `assets/app-icon/nightly/`, not in `hicolor/`: Stable's Linux and FreeBSD packagers copy that
# whole theme tree, so Nightly renders stored under it would ship in every Stable package.
NIGHTLY_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)


def decode_png(data):
    """(width, height, RGBA bytes) for an 8-bit RGB/RGBA PNG, filters included."""
    if data[:8] != PNG_MAGIC:
        raise ValueError("not a PNG")
    pos, idat, header = 8, [], None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        kind = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
        pos += 12 + length
    if header is None:
        raise ValueError("no IHDR")
    width, height, depth, color, _compression, _filter, interlace = header
    if depth != 8 or interlace != 0 or color not in (2, 6):
        raise ValueError(f"unsupported PNG (depth {depth}, color {color}, interlace {interlace})")
    channels = 3 if color == 2 else 4
    raw = zlib.decompress(b"".join(idat))
    stride = width * channels
    out = bytearray(width * height * 4)
    previous = bytearray(stride)
    position = 0
    for row in range(height):
        filter_type = raw[position]
        position += 1
        line = bytearray(raw[position:position + stride])
        position += stride
        if filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = previous[i]
                c = previous[i - channels] if i >= channels else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                predictor = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + predictor) & 0xFF
        elif filter_type != 0:
            raise ValueError(f"unknown PNG filter {filter_type}")
        if channels == 4:
            out[row * width * 4:(row + 1) * width * 4] = line
        else:
            for x in range(width):
                source = x * 3
                target = (row * width + x) * 4
                out[target:target + 3] = line[source:source + 3]
                out[target + 3] = 255
        previous = line
    return width, height, out


def encode_png(width, height, rgba):
    raw = bytearray()
    stride = width * 4
    for row in range(height):
        raw.append(0)
        raw.extend(rgba[row * stride:(row + 1) * stride])

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    return (
        PNG_MAGIC
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def solve(pixel):
    """(ink, paper, background) weights for one pixel, clamped to the three-colour simplex.

    The engraving is drawn as a mix of ink and paper over the tile, so a shaded grey is ink plus
    paper with no background at all; only the background share moves in a Nightly icon.
    """
    rows = [[INK[c] - STABLE_BG[c] for c in range(3)], [PAPER[c] - STABLE_BG[c] for c in range(3)]]
    rhs = [pixel[c] - STABLE_BG[c] for c in range(3)]
    a11 = sum(v * v for v in rows[0])
    a12 = sum(rows[0][c] * rows[1][c] for c in range(3))
    a22 = sum(v * v for v in rows[1])
    b1 = sum(rows[0][c] * rhs[c] for c in range(3))
    b2 = sum(rows[1][c] * rhs[c] for c in range(3))
    determinant = a11 * a22 - a12 * a12
    if determinant == 0:
        return 0.0, 0.0, 1.0
    ink = (b1 * a22 - b2 * a12) / determinant
    paper = (b2 * a11 - b1 * a12) / determinant
    ink, paper = max(0.0, min(1.0, ink)), max(0.0, min(1.0, paper))
    if ink + paper > 1.0:
        total = ink + paper
        ink, paper = ink / total, paper / total
    return ink, paper, 1.0 - ink - paper


def recolour(rgba):
    """The same frame with only its background contribution moved to the Nightly colour."""
    out = bytearray(rgba)
    cache = {}
    for index in range(0, len(rgba), 4):
        if rgba[index + 3] == 0:
            continue
        pixel = (rgba[index], rgba[index + 1], rgba[index + 2])
        got = cache.get(pixel)
        if got is None:
            ink, paper, background = solve(pixel)
            fitted = tuple(ink * INK[c] + paper * PAPER[c] + background * STABLE_BG[c] for c in range(3))
            residual = sum(abs(pixel[c] - fitted[c]) for c in range(3))
            # A pixel the three-colour palette cannot explain is left exactly as it is.
            if residual <= RESIDUAL:
                got = bytes(min(255, max(0, round(ink * INK[c] + paper * PAPER[c] + background * NIGHTLY_BG[c]))) for c in range(3))
            else:
                got = bytes(pixel)
            cache[pixel] = got
        out[index:index + 3] = got
    return out


def read_icon_frames(container):
    """(kind, name, png_bytes) frames of an .ico or .icns file, in file order."""
    if container[:4] == b"icns":
        frames = []
        position = 8
        while position < len(container):
            kind = container[position:position + 4]
            (size,) = struct.unpack(">I", container[position + 4:position + 8])
            body = container[position + 8:position + size]
            if body[:8] != PNG_MAGIC:
                raise ValueError(f"{kind!r} is not a PNG frame; regenerate the stable .icns")
            frames.append(("icns", kind, body))
            position += size
        return frames
    reserved, kind, count = struct.unpack("<HHH", container[:6])
    if reserved != 0 or kind != 1:
        raise ValueError("not an .ico")
    frames = []
    for index in range(count):
        offset = 6 + index * 16
        width, height, _colors, _reserved, _planes, _bpp, size, start = struct.unpack("<BBBBHHII", container[offset:offset + 16])
        body = container[start:start + size]
        if body[:8] != PNG_MAGIC:
            raise ValueError("not a PNG .ico frame; regenerate the stable .ico")
        frames.append(("ico", (width or 256, height or 256), body))
    return frames


def write_ico(frames):
    header = struct.pack("<HHH", 0, 1, len(frames))
    entries, payloads, offset = bytearray(), bytearray(), 6 + 16 * len(frames)
    for (width, height), png in frames:
        entries.extend(struct.pack("<BBBBHHII", width % 256, height % 256, 0, 0, 1, 32, len(png), offset))
        payloads.extend(png)
        offset += len(png)
    return header + bytes(entries) + bytes(payloads)


def write_icns(frames):
    body = bytearray()
    for kind, png in frames:
        body.extend(kind + struct.pack(">I", len(png) + 8) + png)
    return b"icns" + struct.pack(">I", len(body) + 8) + bytes(body)


def nightly_frames(container):
    frames = read_icon_frames(container)
    out = []
    for kind, name, png in frames:
        width, height, rgba = decode_png(png)
        out.append((kind, name, (width, height), encode_png(width, height, recolour(rgba))))
    return out


def targets():
    """Every Nightly asset the app and its packaging use, as (path, bytes)."""
    assets = []
    icon_frames = nightly_frames((ICONS / "texturecraft.icns").read_bytes())
    assets.append((ICONS / "texturecraft-nightly.icns", write_icns([(name, png) for _kind, name, _size, png in icon_frames])))
    ico_frames = nightly_frames((ICONS / "texturecraft.ico").read_bytes())
    assets.append((ICONS / "texturecraft-nightly.ico", write_ico([(size, png) for _kind, _name, size, png in ico_frames])))
    for size in NIGHTLY_SIZES:
        source = ICONS / "hicolor" / f"{size}x{size}" / "apps" / f"{STABLE_ID}.png"
        width, height, rgba = decode_png(source.read_bytes())
        assets.append((ICONS / "nightly" / f"{size}.png", encode_png(width, height, recolour(rgba))))
    width, height, rgba = decode_png((ICONS / "texturecraft-1024.png").read_bytes())
    assets.append((ICONS / "texturecraft-nightly-1024.png", encode_png(width, height, recolour(rgba))))
    # The window icon is the same 256 render the Nightly set carries.
    window = ICONS / "nightly" / "256.png"
    assets.append((ICONS / "texturecraft-nightly.png", next(data for path, data in assets if path == window)))
    return assets


def provenance(assets):
    inputs = [ICONS / "texturecraft.icns", ICONS / "texturecraft.ico", ICONS / "texturecraft-1024.png"]
    inputs += [ICONS / "hicolor" / f"{size}x{size}" / "apps" / f"{STABLE_ID}.png" for size in NIGHTLY_SIZES]
    digest = lambda data: hashlib.sha256(data).hexdigest()
    return {
        "title": "TextureCraft nightly deep-background icons",
        "license": "MIT OR Apache-2.0",
        "generator": "packaging/nightly-icons.py",
        "generator_sha256": digest(Path(__file__).read_bytes()),
        "inputs": [{"path": str(path.relative_to(ROOT)), "sha256": digest(path.read_bytes())} for path in inputs],
        "method": {
            "kind": "derivative of the project-owned stable renders; no badge, crescent or letter",
            "original_palette": {"ink": "#0b0b0c", "paper": "#efe9dc", "background": "#c2672b"},
            "nightly_background": "#643418",
            "transform": "Fit each pixel as the tile background plus one engraved foreground (ink or paper) and replace only the background contribution; keep alpha, padding and foreground detail. Each frame comes from the stable master of its own size: the .icns and .ico PNG frames are recoloured in place, and the per-size PNGs (in nightly/) and the 1024 master are recoloured from their own renders. No upscaling.",
        },
        "outputs": {str(path.relative_to(ROOT)): digest(data) for path, data in assets},
        "ico_frames": ["the stable .ico's own frames (16, 20, 24, 32, 40, 48, 64, 128, 256)"],
        "icns_slots": "the stable .icns's own PNG slots (icp4 through ic10)",
        "validation": "Byte comparison against the committed assets (packaging/nightly-archive-tests.py and the icon check in packaging/nightly-tests.py); native Explorer/Finder rendering is not exercised here. The per-size PNGs are kept out of hicolor/ because Stable's Linux and FreeBSD packages copy that theme tree whole.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if the committed assets differ from these bytes")
    args = parser.parse_args()
    assets = targets()
    record = (json.dumps(provenance(assets), indent=2) + "\n").encode()
    if args.check:
        stale = [path for path, data in assets if not path.is_file() or path.read_bytes() != data]
        if not (ICONS / "nightly-provenance.json").is_file() or (ICONS / "nightly-provenance.json").read_bytes() != record:
            stale.append(ICONS / "nightly-provenance.json")
        if stale:
            for path in stale:
                print(f"stale: {path.relative_to(ROOT)}", file=sys.stderr)
            return 1
        print(f"{len(assets)} Nightly icon files match the generator")
        return 0
    for path, data in assets:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        print(f"wrote {path.relative_to(ROOT)} ({len(data)} bytes)")
    (ICONS / "nightly-provenance.json").write_bytes(record)
    print("wrote assets/app-icon/nightly-provenance.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
