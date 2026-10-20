"""Deterministic public-fixture replay through the production Rust camera flow.

Generate once, then replay the same files with baseline/candidate binaries.
These are controlled image perturbations, not physical camera measurements.
"""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter


CONDITIONS = (
    "clean", "dim", "very-dim", "small-dim", "shadow", "dim-rotated", "glare-static",
    "glare-moving", "glare-stripe", "blur", "tilted", "blank",
)


def generate(root, fixture):
    source = Image.open(fixture).convert("RGB")
    w, h = source.size
    yy, xx = np.mgrid[:h, :w].astype(np.float32)
    x, y = xx / w, yy / h
    for condition in CONDITIONS:
        directory = root / condition
        directory.mkdir(parents=True, exist_ok=True)
        for frame in range(45):  # 8.8 seconds at 5 fps, including the deadline.
            pixels = np.asarray(source, dtype=np.float32).copy()
            if condition in ("dim", "dim-rotated", "very-dim", "small-dim"):
                pixels *= 0.15 if condition == "very-dim" else 0.32
            elif condition == "shadow":
                shade = 0.2 + 0.8 / (1 + np.exp(-(x + 0.25 * y - 0.58) * 35))
                pixels *= shade[:, :, None]
            elif condition == "glare-stripe":
                spot = np.exp(-((y - 0.548) / 0.012) ** 2)
                pixels += 400 * spot[:, :, None]
            elif condition.startswith("glare"):
                center = 0.52 if condition == "glare-static" else 0.3 + (frame % 16) * 0.035
                spot = np.exp(-((x - center) / 0.075) ** 2 - ((y - 0.53) / 0.3) ** 2)
                pixels += 330 * spot[:, :, None]
            elif condition == "blank":
                pixels[:] = 230
            image = Image.fromarray(np.clip(pixels, 0, 255).astype(np.uint8))
            if condition == "small-dim":
                image = image.resize((640, 440), Image.Resampling.LANCZOS)
            if condition == "blur":
                image = image.filter(ImageFilter.GaussianBlur(2.8 if frame < 10 else 0.4))
            if condition == "dim-rotated":
                image = image.rotate(90)
            elif condition == "tilted":
                image = image.rotate(12, resample=Image.Resampling.BILINEAR, fillcolor="white")
            # Subpixel sampling changes while the underlying lighting stays fixed.
            image = image.transform(
                image.size, Image.Transform.AFFINE,
                (1, 0, (frame % 3) * 0.4, 0, 1, (frame % 5) * 0.3),
                resample=Image.Resampling.BILINEAR, fillcolor="white",
            )
            image.save(directory / f"capture-{frame:03}.jpg", quality=90)
    (root / "expected.txt").write_text(("ABCDEFGHIJKLMNOPQRSTUVWXYZ" * 3)[:70])
    (root / "manifest.json").write_text(json.dumps({
        "version": 3, "fixtureSha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
        "fps": 5, "framesPerCondition": 45, "conditions": CONDITIONS,
    }, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--generate", type=Path, help="Public matrix-card.jpg fixture")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.generate:
        generate(args.directory, args.generate)
    if args.binary:
        if not args.output:
            parser.error("--output is required with --binary")
        manifest = json.loads((args.directory / "manifest.json").read_text())
        reports = []
        for condition in manifest["conditions"]:
            result = subprocess.run([
                str(args.binary.resolve()), str(args.directory / condition),
                str(args.directory / "expected.txt"), str(manifest["fps"]),
            ], capture_output=True, text=True, check=True)
            report = {"condition": condition, **json.loads(result.stdout)}
            reports.append(report)
            print(json.dumps({k: v for k, v in report.items() if k != "diagnostics"}), flush=True)
            args.output.write_text(json.dumps({
                "manifest": manifest,
                "binarySha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                "results": reports,
            }, indent=2))


if __name__ == "__main__":
    main()
