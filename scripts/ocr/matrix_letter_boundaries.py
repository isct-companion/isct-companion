"""Public L/Z/I cards that expose confident errors after cell-edge cropping.

No private inputs. Reuses the real Rust acquisition/classification/voting replay.
The exit status is a promotion gate, not a claim that repeated frames provide
independent evidence. Identical frames deliberately expose systematic errors.
"""

import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from matrix_lighting import replay


def generate(root, fixture, font_path):
    letters = ("LZI" * 24)[:70]
    font = ImageFont.truetype(str(font_path), 110)
    conditions = []
    for offset in [-4, 0, 4]:
        condition = f"offset-{offset}"
        conditions.append(condition)
        directory = root / condition
        directory.mkdir(parents=True, exist_ok=True)
        image = Image.open(fixture).convert("RGB")
        if image.size != (1600, 1100):
            raise ValueError("Expected the repository's public matrix-card.jpg fixture")
        draw = ImageDraw.Draw(image)
        # Printed layout: rows alternate pink (1, 3, 5, 7) and white, with no
        # lines between cells (see matrix_fixture_layout.py).
        for row in range(7):
            fill = (236, 168, 169) if row % 2 == 0 else (238, 237, 234)
            draw.rectangle((250, 300 + row * 85, 1349, 300 + (row + 1) * 85 - 1), fill=fill)
        for cell, letter in enumerate(letters):
            box = font.getbbox(letter)
            width, height = box[2] - box[0], box[3] - box[1]
            left = (110 - width) / 2
            top = (85 - height) / 2 + offset
            # Keep complete letters clear of the cell edges.
            if min(left, top) < 2 or left + width > 108 or top + height > 83:
                raise ValueError("Font does not fit the probe; use Times Italic at 110 px")
            draw.text(
                (250 + cell % 10 * 110 + left - box[0],
                 300 + cell // 10 * 85 + top - box[1]),
                letter, font=font, fill=(25,) * 3,
            )
        for frame in range(45):
            image.save(directory / f"capture-{frame:03}.jpg", quality=95)
    (root / "expected.txt").write_text(letters)
    (root / "manifest.json").write_text(json.dumps({
        "version": 1,
        "probe": "letter-boundaries",
        "fixtureSha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
        "fontSha256": hashlib.sha256(font_path.read_bytes()).hexdigest(),
        "fontName": font.getname(),
        "fontSize": 110,
        "fps": 5,
        "framesPerCondition": 45,
        "identicalFrames": True,
        "conditions": conditions,
    }, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--font", type=Path, help="Generate with a local Times Italic TTF")
    parser.add_argument("--fixture", type=Path, default=Path(
        "crates/isct-ocr/src/tests/fixtures/matrix-card.jpg"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.font:
        generate(args.directory, args.fixture, args.font)
    if args.binary:
        if not args.output:
            parser.error("--output is required with --binary")
        reports = replay(args.directory, args.binary, args.output)
        wrong = sum(len(r["wrongStableCells"]) for r in reports)
        missing = sum(70 - r["correct"] for r in reports)
        print(json.dumps({"wrongStableCells": wrong, "missingOrWrongCells": missing}))
        if wrong or missing:
            raise SystemExit(1)
    elif not args.font:
        parser.error("Provide --font to generate or --binary to replay")


if __name__ == "__main__":
    main()
