#!/usr/bin/env python3
"""Regenerate assets/icon.ico (16/32/48/256) with ImageMagick primitives.

A monitor outline with a crescent moon cut into its screen, on a deep
indigo rounded square. 32/48/256 are drawn as vector primitives; 16 is
drawn on a 16x16 pixel grid (no anti-aliasing) so the glyph stays crisp
in the tray. No downloaded assets.
"""
import subprocess
from pathlib import Path

OUT = Path(__file__).resolve().parent
BG = "#16162E"
FG = "#ECE9FF"

# 16x16 pixel map: '#' = foreground, '-' = background, '.' = transparent.
PIXELS_16 = [
    "................",
    ".--------------.",
    ".--------------.",
    ".--------------.",
    "..############..",
    "..#----##----#..",
    "..#---#------#..",
    "..#---#------#..",
    "..#----##----#..",
    "..#----------#..",
    "..############..",
    ".......##.......",
    ".......##.......",
    "....########....",
    ".--------------.",
    "................",
]


def pixels16() -> str:
    """Rectangles for each pixel run, drawn without anti-aliasing."""
    cmds = []
    for y, row in enumerate(PIXELS_16):
        assert len(row) == 16, (y, len(row))
        x = 0
        while x < 16:
            ch = row[x]
            start = x
            while x < 16 and row[x] == ch:
                x += 1
            if ch == ".":
                continue
            color = FG if ch == "#" else BG
            cmds.append(f"fill '{color}' stroke none rectangle {start},{y} {x - 1},{y}")
    return " ".join(cmds)


def draw(size: int) -> str:
    s = size / 256.0
    px = lambda v: round(v * s)

    def stroke(w):
        return max(1.0, round(w * s, 2))

    return " ".join([
        f"fill '{BG}' stroke none roundrectangle {px(4)},{px(4)} {px(252)},{px(252)} {px(54)},{px(54)}",
        f"fill none stroke '{FG}' stroke-width {stroke(13)} "
        f"roundrectangle {px(40)},{px(54)} {px(216)},{px(158)} {px(12)},{px(12)}",
        f"fill '{FG}' stroke none rectangle {px(121)},{px(158)} {px(135)},{px(202)}",
        f"fill '{FG}' stroke none roundrectangle {px(92)},{px(202)} {px(164)},{px(214)} {px(6)},{px(6)}",
        f"fill '{FG}' stroke none circle {px(134)},{px(106)} {px(134)},{px(80)}",
        f"fill '{BG}' stroke none circle {px(146)},{px(98)} {px(146)},{px(72)}",
    ])


def main():
    frames = []
    for size in (16, 32, 48, 256):
        p = OUT / f"icon-{size}.png"
        body = pixels16() if size == 16 else draw(size)
        subprocess.run(
            ["convert", "-size", f"{size}x{size}", "xc:none",
             "-antialias" if size != 16 else "+antialias",
             "-draw", body, "-depth", "8", str(p)],
            check=True,
        )
        frames.append(p)

    # Assemble a PNG-compressed ICO by hand: ImageMagick writes the 256
    # frame as a 65 KB uncompressed BMP, which would eat the size budget.
    blobs = [p.read_bytes() for p in frames]
    header = (0).to_bytes(2, "little") + (1).to_bytes(2, "little") + len(blobs).to_bytes(2, "little")
    offset = 6 + 16 * len(blobs)
    entries = b""
    for size, blob in zip((16, 32, 48, 256), blobs):
        dim = 0 if size == 256 else size
        entries += bytes([dim, dim, 0, 0]) + (1).to_bytes(2, "little") + (32).to_bytes(2, "little")
        entries += len(blob).to_bytes(4, "little") + offset.to_bytes(4, "little")
        offset += len(blob)
    ico = OUT / "icon.ico"
    ico.write_bytes(header + entries + b"".join(blobs))
    print(ico, ico.stat().st_size, "bytes")


if __name__ == "__main__":
    main()
