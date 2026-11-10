#!/usr/bin/env python3
"""Independent body-corner labels: SIFT-align each frame to one measured still.

`--measurement` is a local JSON with `source` (a sharp photo of the same card),
`bodyCorners` (its 7x10 body, pixels, A1 top-left clockwise) and an optional
`rotate` (cv2 rotate code applied to the photo first). Every frame is matched to
the body region of that photo, and a RANSAC homography maps the body corners.
Frames with too few or poorly spread matches are left unlabelled, never guessed.
No grid detector is involved, so the labels can judge any detector.
Output: {"frame.jpg": {"body": [[x, y] x4 normalized], "inliers": n}}. Keep local.
"""
import argparse
import json
from pathlib import Path

import cv2
import numpy as np


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--measurement", type=Path, required=True)
    p.add_argument("--frames", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args()
    meta = json.loads(args.measurement.read_text())
    reference = cv2.imread(meta["source"])
    if "rotate" in meta:
        reference = cv2.rotate(reference, meta["rotate"])
    body = np.float32(meta["bodyCorners"])
    mask = np.zeros(reference.shape[:2], np.uint8)
    cv2.fillPoly(mask, [np.int32(body)], 255)
    sift = cv2.SIFT_create(nfeatures=3000)
    kp, ds = sift.detectAndCompute(cv2.cvtColor(reference, cv2.COLOR_BGR2GRAY), mask)
    matcher = cv2.BFMatcher()
    labels = {}
    paths = sorted(p for p in args.frames.iterdir() if p.suffix in (".jpg", ".png"))
    for path in paths:
        image = cv2.imread(str(path))
        kf, df = sift.detectAndCompute(cv2.cvtColor(image, cv2.COLOR_BGR2GRAY), None)
        if df is None:
            continue
        good = [m[0] for m in matcher.knnMatch(ds, df, k=2) if len(m) == 2 and m[0].distance < .67 * m[1].distance]
        if len(good) < 20:
            continue
        a = np.float32([kp[g.queryIdx].pt for g in good])
        b = np.float32([kf[g.trainIdx].pt for g in good])
        H, inliers = cv2.findHomography(a, b, cv2.RANSAC, 2, maxIters=5000, confidence=.999)
        if H is None:
            continue
        keep = inliers.ravel().astype(bool)
        if keep.sum() < 20:
            continue
        # Matches must cover at least half the body in both directions.
        coverage = np.ptp(a[keep], axis=0) / np.ptp(body, axis=0)
        q = cv2.perspectiveTransform(body.reshape(1, 4, 2), H)[0]
        h, w = image.shape[:2]
        if (keep.sum() >= 20 and keep.mean() >= .4 and coverage.min() > .5 and cv2.isContourConvex(q)
                and (q >= 0).all() and (q[:, 0] < w).all() and (q[:, 1] < h).all()):
            labels[path.name] = {"body": (q / [w, h]).tolist(), "inliers": int(keep.sum())}
    args.out.write_text(json.dumps(labels))
    print(f"labelled {len(labels)} of {len(paths)} frames")


if __name__ == "__main__":
    main()
