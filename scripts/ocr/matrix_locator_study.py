#!/usr/bin/env python3
"""Local paired OCR-input ablation; private frames and labels stay in --out.

Requires matplotlib/numpy for the figure. Reuses the Rust production capture
and alphabet model. Capture latency excludes model inference; ocr_ms includes
normalization and inference of all 70 cells. This is a single-frame comparison,
not a Core dispatcher/session success measurement.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def summarize(rows, expected):
    detected = [r for r in rows if "readings" in r]
    matches = [[a["text"] == b for a, b in zip(r["readings"], expected)] for r in detected]
    assert all(len(r["readings"]) == 70 for r in detected)
    return {
        "frames": len(rows), "detected": len(detected),
        "correct_characters": sum(map(sum, matches)),
        "character_opportunities": len(rows) * 70,
        "exact_frames": sum(all(m) for m in matches),
        "high_confidence_correct": sum(a["text"] == b and a["confidence"] >= .8 for r in detected for a, b in zip(r["readings"], expected)),
        "high_confidence_wrong": sum(bool(a["text"]) and a["text"] != b and a["confidence"] >= .8 for r in detected for a, b in zip(r["readings"], expected)),
        "pink_correct": sum(sum(m[i] for i in range(70) if i // 10 % 2 == 0) for m in matches),
        "pink_opportunities": len(detected) * 40,
        "white_correct": sum(sum(m[i] for i in range(70) if i // 10 % 2 == 1) for m in matches),
        "white_opportunities": len(detected) * 30,
        "q_correct": sum(sum(m[i] for i, c in enumerate(expected) if c == "Q") for m in matches),
        "q_opportunities": len(detected) * expected.count("Q"),
        "capture_p50_ms": float(np.percentile([r["ms"] for r in rows], 50)),
        "capture_p95_ms": float(np.percentile([r["ms"] for r in rows], 95)),
        "ocr_p50_ms": float(np.median([r["ocr_ms"] for r in detected])) if detected else None,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True, help="Local file: 70 letters, whitespace allowed")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--target", type=Path, default=None)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--variants", nargs="+", choices=["luma-input", "red-chroma-input", "paper-distance-input"], default=["luma-input", "red-chroma-input", "paper-distance-input"])
    args = parser.parse_args()
    if any(v != "luma-input" for v in args.variants) and "luma-input" not in args.variants:
        parser.error("input comparisons require luma-input as the paired baseline")
    root = Path(__file__).resolve().parents[2]
    target = (args.target or root / "target").resolve()
    expected = "".join(args.expected.read_text().split())
    if len(expected) != 70 or any(c not in "ABCDEFGHIJKLMNOPQRSTUVWXYZ" for c in expected):
        parser.error("expected file must contain exactly 70 A-Z letters")
    args.out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    for key in ("MATRIX_GLYPH", "MATRIX_HINTS"):
        env.pop(key, None)
    if not args.skip_build:
        subprocess.run(["cargo", "build", "-p", "isct-ocr", "--example", "matrix_locator_bench", "--features", "training-tools", "--locked"], cwd=root, env=env, check=True)
    variants = {
        "luma-input": {"MATRIX_GLYPH": "luma"},
        "red-chroma-input": {"MATRIX_GLYPH": "red-chroma"},
        "paper-distance-input": {"MATRIX_GLYPH": "paper-distance"},
    }
    all_rows, results = {}, {}
    for name in args.variants:
        settings = variants[name]
        print(f"Running {name}", flush=True)
        path = args.out / f"{name}.jsonl"
        with path.open("w") as output:
            subprocess.run([str(target / "debug/examples/matrix_locator_bench"), str(args.frames.resolve()), "--ocr"], cwd=root, env={**env, **settings}, stdout=output, check=True)
        rows = [json.loads(line) for line in path.read_text().splitlines()]
        all_rows[name] = rows
        results[name] = summarize(rows, expected)
        print(json.dumps(results[name]), flush=True)
    geometry = lambda rows: [(r["frame"], r.get("quads")) for r in rows]
    for name in ("red-chroma-input", "paper-distance-input"):
        if name in all_rows and geometry(all_rows[name]) != geometry(all_rows["luma-input"]):
            raise RuntimeError(f"Geometry changed in paired comparison: {name}")
    report = {
        "scope": "Single video, per-frame raw model matches; missed detections count as zero. No Core admission/votes/session deadlines. Pink/white split assumes alternating pink first row.",
        "same_geometry_for_glyph_inputs": True,
        "variants": results,
    }
    (args.out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    names = list(results)
    fig, axes = plt.subplots(1, 3, figsize=(13, 4), layout="constrained")
    label_map = dict(zip(variants, ["Luma", "Red chroma", "Paper distance"]))
    color_map = dict(zip(variants, ["#376fcb", "#d98861", "#499c82"]))
    labels = [label_map[n] for n in names]
    colors = [color_map[n] for n in names]
    for ax, key, title in zip(axes, ["exact_frames", "correct_characters", "capture_p50_ms"], ["Exact 70/70 frames (all frames)", "Correct characters (all frames)", "Capture p50 (ms; excludes OCR)"]):
        values = [results[n][key] for n in names]
        bars = ax.bar(range(len(names)), values, color=colors)
        ax.bar_label(bars, fmt="%.1f" if key.endswith("ms") else "%d", padding=3)
        ax.set_xticks(range(len(names)), labels, rotation=25, ha="right")
        ax.set_title(title, fontsize=10)
        ax.spines[["top", "right"]].set_visible(False)
        ax.set_ylim(0, max(values) * 1.2 + 1)
    fig.suptitle(f"Paired OCR input study · {len(all_rows[names[0]])} video frames")
    fig.savefig(args.out / "comparison.png", dpi=170)
    plt.close(fig)


if __name__ == "__main__":
    main()
