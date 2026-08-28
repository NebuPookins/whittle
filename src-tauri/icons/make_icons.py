#!/usr/bin/env python3
"""Generate Whittle's app icons.

Dependency-free: rasterises with 4x supersampling and writes PNGs using only
zlib + struct. Re-run from this directory if the mark ever changes:

    python3 make_icons.py
"""
import struct
import zlib

SS = 4  # supersampling factor

BG = (0x12, 0x16, 0x1D, 255)   # near-black panel
DIM = (0x2C, 0x35, 0x45, 255)  # discarded region
HOT = (0xF5, 0xA6, 0x23, 255)  # kept region (amber)
EDGE = (0xFF, 0xD1, 0x80, 255) # marker handles


def rounded_rect(x, y, w, h, r):
    """Return a coverage predicate for a rounded rectangle."""
    def inside(px, py):
        if not (x <= px <= x + w and y <= py <= y + h):
            return False
        cx = min(max(px, x + r), x + w - r)
        cy = min(max(py, y + r), y + h - r)
        return (px - cx) ** 2 + (py - cy) ** 2 <= r * r
    return inside


def render(size):
    """Render the mark at `size` px, supersampled, returning RGBA rows."""
    n = size * SS
    u = n / 100.0  # 1 unit == 1% of the icon edge

    panel = rounded_rect(6 * u, 6 * u, 88 * u, 88 * u, 20 * u)
    # The timeline bar, and the bright kept-segment sitting on it.
    bar = rounded_rect(16 * u, 43 * u, 68 * u, 14 * u, 7 * u)
    kept = rounded_rect(40 * u, 40 * u, 26 * u, 20 * u, 6 * u)
    # Two markers closing in on the kept segment.
    left = rounded_rect(36 * u, 30 * u, 6 * u, 40 * u, 3 * u)
    right = rounded_rect(64 * u, 30 * u, 6 * u, 40 * u, 3 * u)

    buf = bytearray(n * n * 4)
    for py in range(n):
        yc = py + 0.5
        row = py * n * 4
        for px in range(n):
            xc = px + 0.5
            if left(xc, yc) or right(xc, yc):
                c = EDGE
            elif kept(xc, yc):
                c = HOT
            elif bar(xc, yc):
                c = DIM
            elif panel(xc, yc):
                c = BG
            else:
                c = (0, 0, 0, 0)
            i = row + px * 4
            buf[i:i + 4] = bytes(c)
    return downsample(buf, n, size)


def downsample(buf, n, size):
    """Box-filter n*n RGBA down to size*size, compositing on straight alpha."""
    rows = []
    for oy in range(size):
        row = bytearray()
        for ox in range(size):
            r = g = b = a = 0
            for dy in range(SS):
                for dx in range(SS):
                    i = ((oy * SS + dy) * n + (ox * SS + dx)) * 4
                    pa = buf[i + 3]
                    r += buf[i] * pa
                    g += buf[i + 1] * pa
                    b += buf[i + 2] * pa
                    a += pa
            if a:
                row += bytes((round(r / a), round(g / a), round(b / a),
                              round(a / (SS * SS))))
            else:
                row += b"\0\0\0\0"
        rows.append(bytes(row))
    return rows


def write_png(path, rows, size):
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as fh:
        fh.write(png)
    print(f"  {path} ({size}x{size}, {len(png)} bytes)")


if __name__ == "__main__":
    for name, size in [("32x32.png", 32), ("128x128.png", 128),
                       ("128x128@2x.png", 256), ("icon.png", 512)]:
        write_png(name, render(size), size)
