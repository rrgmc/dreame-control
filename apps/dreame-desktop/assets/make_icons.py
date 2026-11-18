"""Draws the Dreame Control app icon and writes it in the formats each system needs.

    python make_icons.py

Writes, next to this script:
- icon-1024.png and icon-256.png (window icon, Linux, AppImage),
- icon.ico (Windows executable and installer; PNG entries 16 to 256 px),
- icon.icns (macOS app bundle; PNG entries 16 to 1024 px).

Standard library only, so it runs anywhere. The icon is a white robot vacuum seen from above (a
ring with a bumper arc and a centre dot) on the app's blue, in a rounded square.
"""
import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
BLUE = (37, 99, 235)
WHITE = (255, 255, 255)


def coverage(x, y):
    """Colour and alpha of the icon at (x, y) in a 0..1 square."""
    # Rounded square background.
    r = 0.22
    dx = max(abs(x - 0.5) - (0.5 - r), 0.0)
    dy = max(abs(y - 0.5) - (0.5 - r), 0.0)
    if math.hypot(dx, dy) > r:
        return None
    cx, cy = x - 0.5, y - 0.5
    d = math.hypot(cx, cy)
    # Robot body: a ring.
    if 0.27 <= d <= 0.33:
        return WHITE
    # Bumper: a thick arc across the top of the body.
    a = math.atan2(-cy, cx)
    if 0.19 <= d <= 0.23 and math.radians(35) <= a <= math.radians(145):
        return WHITE
    # Centre dot: the lidar turret.
    if d <= 0.075:
        return WHITE
    return BLUE


def draw(size, ss=4):
    """RGBA rows of the icon at `size` pixels, `ss` x `ss` samples per pixel."""
    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(ss):
                for sx in range(ss):
                    c = coverage((px + (sx + 0.5) / ss) / size, (py + (sy + 0.5) / ss) / size)
                    if c is not None:
                        acc[0] += c[0]
                        acc[1] += c[1]
                        acc[2] += c[2]
                        acc[3] += 255
            n = ss * ss
            a = acc[3] // n
            if a:
                # Average the colour over the covered samples only.
                k = acc[3] // 255
                row += bytes((acc[0] // k, acc[1] // k, acc[2] // k, a))
            else:
                row += bytes(4)
        rows.append(bytes(row))
    return rows


def png(rows):
    size = len(rows)

    def chunk(kind, data):
        c = struct.pack(">I", len(data)) + kind + data
        return c + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    raw = b"".join(b"\0" + r for r in rows)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def ico(pngs):
    """An ICO file of PNG entries, smallest first."""
    head = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    dirs, data = b"", b""
    for size, blob in pngs:
        s = 0 if size >= 256 else size
        dirs += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return head + dirs + data


def icns(pngs):
    """An ICNS file of PNG entries."""
    kinds = {16: b"icp4", 32: b"icp5", 64: b"icp6", 128: b"ic07", 256: b"ic08", 512: b"ic09", 1024: b"ic10"}
    body = b"".join(kinds[s] + struct.pack(">I", 8 + len(b)) + b for s, b in pngs if s in kinds)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main():
    sizes = [16, 32, 48, 64, 128, 256, 512, 1024]
    blobs = {}
    for s in sizes:
        blobs[s] = png(draw(s, ss=4 if s <= 256 else 2))
        print("drew", s)
    for s in (256, 1024):
        with open(os.path.join(HERE, f"icon-{s}.png"), "wb") as f:
            f.write(blobs[s])
    with open(os.path.join(HERE, "icon.ico"), "wb") as f:
        f.write(ico([(s, blobs[s]) for s in (16, 32, 48, 64, 128, 256)]))
    with open(os.path.join(HERE, "icon.icns"), "wb") as f:
        f.write(icns([(s, blobs[s]) for s in (16, 32, 64, 128, 256, 512, 1024)]))
    print("wrote icon-256.png, icon-1024.png, icon.ico, icon.icns")


if __name__ == "__main__":
    main()
