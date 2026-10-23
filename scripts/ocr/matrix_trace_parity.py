#!/usr/bin/env python3
"""Frame-by-frame agreement of two geometry traces (e.g. Rust vs OpenCV reference).

    python scripts/ocr/matrix_trace_parity.py --frames FRAMES A.jsonl B.jsonl [--stage fit]

For every frame: located by both, by one only, or by neither, and where both
located, the worst body-corner difference in cells (cell width from trace A).
`--stage` compares a named stage instead of the final grid (for example the
`fit` quad, to separate fitting from tracking differences). Prints counts,
difference percentiles and the most different frames.
"""
import argparse
import json
from pathlib import Path

import cv2
import numpy as np


def load(path):
    return {r["frame"]: r for r in map(json.loads, Path(path).read_text().splitlines()) if r}


def quad(row, stage):
    if stage == "final":
        return row.get("final")
    return row.get("stages", {}).get(stage)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--frames", type=Path, required=True)
    p.add_argument("a", type=Path)
    p.add_argument("b", type=Path)
    p.add_argument("--stage", default="final")
    p.add_argument("--worst", type=int, default=10)
    args = p.parse_args()
    a, b = load(args.a), load(args.b)
    sizes = {}
    counts = {"both": 0, "onlyA": 0, "onlyB": 0, "neither": 0}
    diffs = []
    for name in sorted(set(a) & set(b)):
        qa, qb = quad(a[name], args.stage), quad(b[name], args.stage)
        key = "both" if qa and qb else "onlyA" if qa else "onlyB" if qb else "neither"
        counts[key] += 1
        if key != "both":
            continue
        if name not in sizes:
            image = cv2.imread(str(args.frames / name))
            sizes[name] = (image.shape[1], image.shape[0])
        w, h = sizes[name]
        pa, pb = np.array(qa) * [w, h], np.array(qb) * [w, h]
        cell = (np.linalg.norm(pa[1] - pa[0]) + np.linalg.norm(pa[2] - pa[3])) / 20
        diffs.append((float(np.max(np.linalg.norm(pa - pb, axis=1)) / cell), name))
    values = [d for d, _ in diffs]
    report = {
        "stage": args.stage, **counts,
        "diffCellsP50": float(np.percentile(values, 50)) if values else None,
        "diffCellsP95": float(np.percentile(values, 95)) if values else None,
        "within0.05": sum(v <= 0.05 for v in values),
        "within0.1": sum(v <= 0.1 for v in values),
        "over0.25": sum(v > 0.25 for v in values),
        "worst": [(n, round(d, 3)) for d, n in sorted(diffs, reverse=True)[: args.worst]],
        "onlyA": [n for n in sorted(set(a) & set(b)) if quad(a[n], args.stage) and not quad(b[n], args.stage)][:20],
        "onlyB": [n for n in sorted(set(a) & set(b)) if quad(b[n], args.stage) and not quad(a[n], args.stage)][:20],
    }
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
