#!/usr/bin/env python3
"""Repaint a public synthetic Matrix fixture in the printed card's layout.

The original synthetic fixtures (1600x1100; body x 250-1350 in 110 px columns,
y 300-895 in 85 px rows; header above, labels at x 140-250) drew every row
pink with dark lines around every cell. The printed card has no lines between
columns: rows alternate pink (1, 3, 5, 7) and white, and the label column
alternates black and grey; the header is 1.05 rows tall and the labels 1.03
columns wide. The geometry rewrite locates the card from exactly those
features, so fixtures must show them. Glyph pixels and their
antialiasing are kept; only the background is repainted.

    python scripts/ocr/matrix_fixture_layout.py FIXTURE [--line-clearance PX]
"""
import argparse
from pathlib import Path

import numpy as np
from PIL import Image

# The original fixture's pink: more saturated than the printed card (R-G about
# 15-25 in camera frames), so existing glare/admission tests keep their meaning.
PINK = np.array([236, 168, 169], np.float32)
WHITE = np.array([238, 237, 234], np.float32)
INK = np.array([25, 25, 25], np.float32)
LABELS = (np.array([28, 28, 28], np.float32), np.array([112, 112, 112], np.float32))
LEFT, TOP, COL, ROW = 250, 300, 110, 85


def repaint(path, clearance):
    image = Image.open(path)
    pixels = np.asarray(image.convert("RGB"), np.float32).copy()
    if pixels.shape[:2] != (1100, 1600):
        raise ValueError("expected a 1600x1100 public Matrix fixture")
    luma = pixels @ np.array([0.299, 0.587, 0.114], np.float32)
    ys, xs = np.mgrid[:1100, :1600]
    right, bottom = LEFT + 10 * COL, TOP + 7 * ROW
    body = (xs >= LEFT) & (xs < right) & (ys >= TOP) & (ys < bottom)
    # The old outer border strokes outside the body become white card paper.
    border = ~body & (xs >= LEFT) & (ys >= TOP) & (xs < right + clearance) & (ys < bottom + clearance)
    pixels[border] = 255
    row = np.clip((ys - TOP) // ROW, 0, 6)
    background = np.where((row % 2 == 0)[..., None], PINK, WHITE)
    # Old cell background luma (median of non-ink body pixels) and grid lines.
    old = float(np.median(luma[body & (luma > 120)]))
    dx = np.minimum((xs - LEFT) % COL, COL - (xs - LEFT) % COL)
    dy = np.minimum((ys - TOP) % ROW, ROW - (ys - TOP) % ROW)
    on_line = (dx < clearance) | (dy < clearance)
    alpha = np.clip((old - luma) / (old - INK.mean()), 0, 1)[..., None]
    alpha = np.where(on_line[..., None], 0, alpha)
    painted = alpha * INK + (1 - alpha) * background
    pixels = np.where(body[..., None], painted, pixels)
    # The printed header is 1.05 rows tall and the label column 1.03 columns
    # wide (measured on the card); the old fixture drew exactly one of each.
    header_top = TOP - round(1.05 * ROW)
    grow_header = (xs >= LEFT) & (xs < right) & (ys >= header_top) & (ys < TOP - ROW)
    pixels[grow_header] = pixels[TOP - ROW + 2, LEFT + 2]
    label_left = LEFT - round(1.03 * COL)
    # Labels: alternating black/grey cells, white digits kept with their edges.
    labels = (xs >= label_left) & (xs < LEFT) & (ys >= TOP) & (ys < TOP + 7 * ROW)
    base = np.where((row % 2 == 0)[..., None], LABELS[0], LABELS[1])
    white = np.clip((luma - 51) / (255 - 51), 0, 1)
    white = np.where(xs >= LEFT - COL, white, 0)[..., None]  # digits only in the old column
    pixels = np.where(labels[..., None], white * 255 + (1 - white) * base, pixels)
    out = Image.fromarray(np.clip(pixels, 0, 255).round().astype(np.uint8))
    if path.suffix == ".jpg":
        out.save(path, quality=95)
    else:
        out.save(path)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("fixture", type=Path)
    p.add_argument("--line-clearance", type=int, default=6,
                   help="px around each old grid line to clear (6 for wide gaps, 2 for 2 px lines)")
    args = p.parse_args()
    repaint(args.fixture, args.line_clearance)


if __name__ == "__main__":
    main()
