#!/usr/bin/env python3
"""Offline OpenCV reference for Matrix geometry: detect, fit, track, smooth.

Development tooling only; nothing here ships. It writes the same per-frame
trace JSONL as `matrix_track_replay` so `matrix_track_report.py` can compare
both on one recording:

    python scripts/ocr/matrix_cv_reference.py FRAMES [--fps 30] > reference.jsonl

Model: the 7x10 body in cell units, u along columns (0..10), v along rows
(0..7), A1 top-left at (0, 0). The card prints a dark header band above row 1,
dark/grey row labels left of column A, thin dark column lines, and alternating
pink (rows 1, 3, 5, 7) and white rows.

Per frame:
1. detect: dark, elongated, letter-filled header candidates seed a homography
   for each of the four orientations; anchors (dark header above, pink row 1,
   dark labels left) pick the orientation.
2. fit: each of the 11x8 grid intersections is measured locally - the column
   line as a dark line in luminance, the row boundary as a signed pink/white
   step in R-G whose sign is fixed by row parity - and ONE homography is fitted
   to all of them with cv2.findHomography(USAC_MAGSAC), which LM-refines on
   inliers. Fitting grows from the header rows to the full grid. Inliers must
   cover every quadrant; the reprojection residual is the quality score.
   Header/label/parity anchors are rescored for every +-1 row/column shift and
   the unshifted grid must win, which rules out off-by-one aliasing.
   Hard rejects: non-convex or clipped body, ill-conditioned or strongly
   foreshortened homography, and a sudden area/angle jump from the previous
   frame unless the next detection confirms it. No fixed image aspect ratio.
3. track: pyramidal KLT (calcOpticalFlowPyrLK, forward-backward checked) on
   Shi-Tomasi corners inside the card (letters and line junctions; row
   boundaries are colour-only, so pure-grey grid crossings are weak KLT
   features), then a RANSAC frame-to-frame homography. Low inlier ratio, high
   residual or every N frames triggers a refit seeded by the tracked grid; a
   failed refit falls back to full detection.
4. smooth: a One Euro filter on the displayed corners only; cropping and the
   keyframe decision use the unsmoothed fit.
"""
import argparse
import json
import math
import sys
import time
from pathlib import Path

import cv2
import numpy as np

COLS, ROWS = 10, 7
WORK = 960  # long side of the analysed image
GRAY_ONLY = False  # --gray-only: what a grey-only transport would leave


def apply(H, uv):
    uv = np.asarray(uv, np.float64)
    p = np.c_[uv.reshape(-1, 2), np.ones(uv.size // 2)] @ H.T
    return (p[:, :2] / p[:, 2:3]).reshape(uv.shape)


def body_of(H):
    return apply(H, [[0, 0], [COLS, 0], [COLS, ROWS], [0, ROWS]])


class Frame:
    def __init__(self, bgr):
        h, w = bgr.shape[:2]
        self.scale = WORK / max(h, w)
        small = cv2.resize(bgr, None, fx=self.scale, fy=self.scale, interpolation=cv2.INTER_AREA)
        f = small.astype(np.float32)
        self.gray = 0.299 * f[..., 2] + 0.587 * f[..., 1] + 0.114 * f[..., 0]
        self.chroma = f[..., 2] - f[..., 1]  # pink rows are positive
        if GRAY_ONLY:
            # Pink rows are darker than white ones in luminance as well.
            self.chroma = -self.gray
        self.u8 = np.clip(self.gray, 0, 255).astype(np.uint8)
        self.h, self.w = self.gray.shape

    def sample(self, image, pts):
        pts = np.asarray(pts, np.float32)
        shape = pts.shape[:-1]
        flat = pts.reshape(-1, 1, 2)
        out = cv2.remap(image, flat[..., 0].copy(), flat[..., 1].copy(), cv2.INTER_LINEAR,
                        borderMode=cv2.BORDER_REPLICATE)
        return out.reshape(shape)


def cell_px(H):
    b = body_of(H)
    return (np.linalg.norm(b[1] - b[0]) + np.linalg.norm(b[2] - b[3])) / (2 * COLS)


# ---------------------------------------------------------------- measurement

# Card constants in cell units, measured on SIFT-labelled frames
# (`--calibrate`): header band height above row 1 and label column width.
HEADER = 1.05
LABELS = 1.03
W = 0.12  # half-width (cells) of the step derivative


def peak(response, offsets):
    """Arg-max with parabolic sub-sample refinement."""
    k = int(np.argmax(response))
    value = response[k]
    shift = offsets[k]
    if 0 < k < len(response) - 1:
        a, b, c = response[k - 1], response[k], response[k + 1]
        d = a - 2 * b + c
        if d < 0:
            shift += 0.5 * (a - c) / d * (offsets[1] - offsets[0])
    return shift, value


def edge_points(frame, H, samples, radius):
    """Sub-pixel step positions along each sample's search direction.

    `samples` rows: (u, v, du, dv, channel, sign). The step is searched along
    (du, dv) around (u, v); `sign` * d(channel)/ds must peak there.
    """
    steps = max(9, int(radius / 0.025) * 2 + 1)
    offsets = np.linspace(-radius, radius, steps)
    found = []
    for channel in ("gray", "chroma"):
        rows = [s for s in samples if s[4] == channel]
        if not rows:
            continue
        a = np.float64([s[:4] for s in rows])
        sign = np.float64([s[5] for s in rows])
        # positions: sample x offset x {-w, +w} x across-span {-0.06, 0, 0.06}
        across = np.array([-0.06, 0, 0.06])
        t = offsets[None, :, None, None] + np.array([-W, W])[None, None, :, None]
        normal = np.stack([-a[:, 3], a[:, 2]], -1)  # perpendicular to search direction
        pu = a[:, 0, None, None, None] + t * a[:, 2, None, None, None] + across * normal[:, 0, None, None, None]
        pv = a[:, 1, None, None, None] + t * a[:, 3, None, None, None] + across * normal[:, 1, None, None, None]
        image = getattr(frame, channel)
        values = frame.sample(image, apply(H, np.stack([pu, pv], -1))).mean(axis=3)
        response = sign[:, None] * (values[:, :, 1] - values[:, :, 0])
        floor = 12 if channel == "gray" else 7
        for k in range(len(rows)):
            shift, strength = peak(response[k], offsets)
            if strength >= floor:
                u, v = a[k, 0] + shift * a[k, 2], a[k, 1] + shift * a[k, 3]
                found.append((rows[k], apply(H, [[u, v]])[0], strength))
    return found


def line_specs():
    """Model lines: (name, axis value, is_horizontal, samples)."""
    # Sample row steps in the ink-free gaps between letters, never through them.
    across_u = np.array([0.08] + [c + d for c in range(1, COLS) for d in (-0.07, 0.07)] + [COLS - 0.08])
    specs = [("header-top", -HEADER, True, [(u, -HEADER, 0, 1, "gray", -1) for u in across_u])]
    for j in range(ROWS + 1):
        sign = 1 if j % 2 == 0 else -1  # rows 1, 3, 5, 7 are pink
        if GRAY_ONLY and j == 0:  # dark header above pink row 1: a luminance rise
            specs.append(("row-0", 0, True, [(u, 0, 0, 1, "gray", 1) for u in across_u]))
            continue
        specs.append((f"row-{j}", j, True, [(u, j, 0, 1, "chroma", sign) for u in across_u]))
    rows_v = np.arange(0.25, ROWS, 0.5)
    header_v = np.linspace(-HEADER + 0.2, -0.2, 3)
    specs.append(("labels-left", -LABELS, False, [(-LABELS, v, 1, 0, "gray", -1) for v in rows_v]))
    specs.append(("body-left", 0, False, [(0, v, 1, 0, "gray", 1) for v in rows_v]
                  + [(0, v, 1, 0, "gray", -1) for v in header_v]))
    pink_v = [r + d for r in range(0, ROWS, 2) for d in (0.25, 0.5, 0.75)]
    specs.append(("body-right", COLS, False, [(COLS, v, 1, 0, "chroma", -1) for v in pink_v]
                  + [(COLS, v, 1, 0, "gray", 1) for v in header_v]))
    return specs


def fit_line(points, tolerance):
    """RANSAC line through image points; returns (point, direction, inliers)."""
    pts = np.float64(points)
    if len(pts) < 4:
        return None
    best = None
    rng = np.random.default_rng(0)
    for _ in range(60):
        i, j = rng.choice(len(pts), 2, replace=False)
        d = pts[j] - pts[i]
        n = np.linalg.norm(d)
        if n < 1e-6:
            continue
        normal = np.array([-d[1], d[0]]) / n
        inl = np.abs((pts - pts[i]) @ normal) < tolerance
        if best is None or inl.sum() > best.sum():
            best = inl
    if best is None or best.sum() < max(4, 0.4 * len(pts)):
        return None
    vx, vy, x0, y0 = cv2.fitLine(pts[best].astype(np.float32), cv2.DIST_HUBER, 0, 0.01, 0.01).ravel()
    return np.array([x0, y0]), np.array([vx, vy]), int(best.sum())


def intersect(a, b):
    (p, d), (q, e) = a[:2], b[:2]
    m = np.array([d, -e]).T
    if abs(np.linalg.det(m)) < 1e-6:
        return None
    s = np.linalg.solve(m, q - p)
    return p + s[0] * d


def measure(frame, H, radius, max_v=ROWS, max_u=COLS):
    """Intersections of the fitted printed lines, as model/image pairs.

    `max_v` limits measurement to rows near the header while a header-only seed
    is still extrapolating the lower rows.
    """
    cp = cell_px(H)
    fitted = {}
    for name, value, horizontal, samples in line_specs():
        if (horizontal and value > max_v) or (not horizontal and value > max_u):
            continue
        samples = [x for x in samples if x[1] <= max_v and x[0] <= max_u]
        found = edge_points(frame, H, samples, radius)
        line = fit_line([p for _, p, _ in found], max(1.0, 0.06 * cp))
        if line is not None:
            fitted[name] = (value, horizontal, line)
    model, image = [], []
    for hn, (hv, horizontal, hl) in fitted.items():
        if not horizontal:
            continue
        for vn, (vv, vertical_is_h, vl) in fitted.items():
            if vertical_is_h:
                continue
            point = intersect(hl, vl)
            if point is not None:
                model.append((vv, hv))
                image.append(point)
    return np.float64(model), np.float64(image), fitted


def anchors(frame, H, du=0, dv=0):
    """Signed step strength of every printed boundary for a grid shifted by (du, dv).

    Header top, label edge, both body edges and the row-parity steps are read
    exactly where the shifted model puts them, with no search. A one-row shift
    flips every parity step; a one-column shift moves the column edges onto
    plain cells; so the true indexing must score clearly highest.
    """
    total, count = 0.0, 0
    for _, _, _, samples in line_specs():
        for channel in ("gray", "chroma"):
            rows = np.float64([x[:4] for x in samples if x[4] == channel])
            if not len(rows):
                continue
            sign = np.float64([x[5] for x in samples if x[4] == channel])
            u, v = rows[:, 0] + du, rows[:, 1] + dv
            ahead = np.stack([u + W * rows[:, 2], v + W * rows[:, 3]], -1)
            behind = np.stack([u - W * rows[:, 2], v - W * rows[:, 3]], -1)
            image = getattr(frame, channel)
            step = sign * (frame.sample(image, apply(H, ahead)) - frame.sample(image, apply(H, behind)))
            floor = 12 if channel == "gray" else 7
            total += float(np.clip(step / floor, -3, 3).sum())
            count += len(step)
    return total / count


def column_gaps(frame, H):
    """Fraction of column gaps (u = 1..9, every row) that are clear of ink.

    Letters sit centred with paper margins, so with the right column pitch the
    strip between two letters is lighter than the darkest ink of the cells on
    either side. A grid squeezed or stretched along a row puts strips on ink.
    """
    c, r = np.meshgrid(np.arange(1, COLS), np.arange(ROWS))
    c, r = c.ravel()[:, None], r.ravel()[:, None]
    dv = np.linspace(0.2, 0.8, 7)[None, :]
    strip = np.concatenate([np.stack(np.broadcast_arrays(c + du, r + dv), -1) for du in (-0.04, 0, 0.04)], 1)
    left = np.stack(np.broadcast_arrays(c - 0.5 + np.linspace(-0.2, 0.2, 5)[None, :], r + 0.5), -1)
    right = np.stack(np.broadcast_arrays(c + 0.5 + np.linspace(-0.2, 0.2, 5)[None, :], r + 0.5), -1)
    gap = frame.sample(frame.gray, apply(H, strip)).min(axis=1)
    ink = np.minimum(frame.sample(frame.gray, apply(H, left)).min(axis=1),
                     frame.sample(frame.gray, apply(H, right)).min(axis=1))
    paper = np.percentile(frame.sample(frame.gray, apply(H, strip)), 90)
    return float(np.mean(gap > ink + 0.35 * (paper - ink)))


def anchored(frame, H):
    base = anchors(frame, H)
    # Whole-cell shifts catch off-by-one indexing; half-cell shifts catch a grid
    # locked between the printed boundaries.
    shifts = [-1, -0.5, 0, 0.5, 1]
    others = [anchors(frame, H, du, dv) for du in shifts for dv in shifts if du or dv]
    return base, base - max(others)


def fit(frame, H, radii=(0.4, 0.3, 0.2), grow=None):
    """Joint homography over measured intersections; None when it fails a gate.

    `grow` starts from the seed's own band - "header": header and three rows;
    "labels": label column and three columns - before fitting the full grid.
    """
    first = {"header": [(0.45, 3, COLS)], "labels": [(0.45, ROWS, 3)], None: []}[grow]
    for radius, max_v, max_u in first + [(r, ROWS, COLS) for r in radii]:
        model, image, fitted = measure(frame, H, radius, max_v, max_u)
        if len(model) < (6 if (max_v, max_u) != (ROWS, COLS) else 8):
            return None, {"reason": f"intersections {len(model)}", "lines": sorted(fitted)}
        threshold = max(1.0, 0.05 * cell_px(H))
        Hn, mask = cv2.findHomography(model, image, cv2.USAC_MAGSAC, threshold, maxIters=2000, confidence=0.999)
        if Hn is None:
            return None, {"reason": "homography"}
        H = Hn
    keep = mask.ravel().astype(bool)
    error = np.linalg.norm(apply(H, model[keep]) - image[keep], axis=1)
    cp = cell_px(H)
    horizontal = {n for n, (_, h, _) in fitted.items() if h}
    vertical = {n for n, (_, h, _) in fitted.items() if not h}
    info = {"inliers": int(keep.sum()), "measured": len(model), "ratio": float(keep.mean()),
            "residual": float(np.sqrt(np.mean(error ** 2)) / cp), "cellPx": float(cp),
            "lines": len(fitted)}
    # Inliers must span the grid: both body edges and rows at top and bottom.
    rows_seen = {int(m[1]) for m in model[keep]}
    if (keep.sum() < 8 or keep.mean() < 0.6 or len(vertical) < 2 or len(horizontal) < 5
            or not ({"body-left", "labels-left"} & vertical) or "body-right" not in vertical
            or min(rows_seen) > 1 or max(rows_seen) < ROWS - 1):
        info["reason"] = "inlier coverage"
        return None, info
    if info["residual"] > 0.08:
        info["reason"] = "residual"
        return None, info
    reason = geometry_reject(frame, H)
    if reason:
        info["reason"] = reason
        return None, info
    score, margin = anchored(frame, H)
    info["anchor"], info["anchorMargin"] = float(score), float(margin)
    info["columnGaps"] = column_gaps(frame, H)
    if info["columnGaps"] < 0.7:
        info["reason"] = "column pitch"
        return None, info
    if margin < 0.3:
        info["reason"] = "anchor aliasing"
        return None, info
    return H, info


def geometry_reject(frame, H):
    b = body_of(H)
    if not cv2.isContourConvex(b.astype(np.float32)):
        return "non-convex"
    if (b < 0).any() or (b[:, 0] >= frame.w).any() or (b[:, 1] >= frame.h).any():
        return "clipped"
    # Homography conditioning in normalized coordinates: model unit box -> image unit box.
    T = np.diag([1 / frame.w, 1 / frame.h, 1])
    M = np.diag([COLS, ROWS, 1])
    Hn = T @ H @ M
    Hn /= Hn[2, 2]
    if np.linalg.cond(Hn) > 1e4:
        return "ill-conditioned"
    # Foreshortening: projective depth across the card stays within 2.5x.
    w = (np.c_[[[-1, -1], [COLS + 1, -1], [COLS + 1, ROWS + 1], [-1, ROWS + 1]], np.ones(4)] @ H.T)[:, 2]
    if (w <= 0).any() or w.max() / w.min() > 2.5:
        return "foreshortened"
    return None


# ------------------------------------------------------------------ detection

def header_candidates(frame):
    """Seeds from straight outer edges of dark, letter-filled bands.

    The header may merge with the label column at its corner, so a band is
    found from one long polygon edge and the dark depth behind it, not from a
    whole-component rectangle. The depth side is the body side.
    """
    out = []
    masks = []
    # Dark relative to the neighbourhood: dim rooms put the paper itself
    # below any fixed level, while the band stays darker than its margin.
    for block, offset in ((max(31, frame.w // 12) | 1, 20), (max(31, frame.w // 6) | 1, 25)):
        masks.append(cv2.adaptiveThreshold(frame.u8, 1, cv2.ADAPTIVE_THRESH_MEAN_C, cv2.THRESH_BINARY_INV,
                                           block, offset))
    masks += [(frame.gray < t).astype(np.uint8) for t in (60, 100)]
    for mask in masks:
        contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        filled = np.zeros_like(mask)
        cv2.drawContours(filled, contours, -1, 1, -1)
        for c in contours:
            if cv2.contourArea(c) < (frame.w * 0.01) ** 2:
                continue
            poly = cv2.approxPolyDP(c, 0.01 * cv2.arcLength(c, True), True).reshape(-1, 2).astype(np.float64)
            for k in range(len(poly)):
                p, q = poly[k], poly[(k + 1) % len(poly)]
                length = np.linalg.norm(q - p)
                if length < frame.w * 0.08:
                    continue
                d = (q - p) / length
                for normal in (np.array([-d[1], d[0]]), np.array([d[1], -d[0]])):
                    depths = []
                    for f in np.linspace(0.15, 0.85, 15):
                        start = p + f * (q - p) + normal * 1.5
                        run = 0
                        while run < length / 4:
                            x, y = (start + normal * run).round().astype(int)
                            if not (0 <= x < frame.w and 0 <= y < frame.h) or not filled[y, x]:
                                break
                            run += 1
                        depths.append(run)
                    depth = float(np.median(depths)) + 1.5
                    if depth < 4 or np.std(depths) > 0.25 * depth:
                        continue
                    if 7 <= length / depth <= 24:
                        out.append(("header", p, q, normal, depth))
                    elif 3.5 <= length / depth <= 9:
                        out.append(("labels", p, q, normal, depth))
    return out


def hypotheses(candidate):
    """Header outer edge p-q, depth towards the body: A at either end."""
    kind, p, q, normal, depth = candidate
    near, far = (p, q), (p + normal * depth, q + normal * depth)
    if kind == "labels":
        # Label column: outer edge u = -LABELS, inner edge u = 0, rows 0..7.
        src = np.float32([[-LABELS, 0], [-LABELS, ROWS], [0, ROWS], [0, 0]])
        for outer, inner in ((near, far), (far, near)):
            for k in (0, 1):
                a, b = outer if k == 0 else outer[::-1]
                c, d = inner if k == 0 else inner[::-1]
                yield cv2.getPerspectiveTransform(src, np.float32([a, b, d, c])).astype(np.float64)
        return
    # The found edge may be the header's top (body beyond the band) or its
    # bottom (body on this side of the edge): try both, with A at either end.
    for top, bottom in ((near, far), (far, near)):
        for k in (0, 1):
            a, b = top if k == 0 else top[::-1]
            c, d = bottom if k == 0 else bottom[::-1]
            src = np.float32([[0, -HEADER], [COLS, -HEADER], [COLS, 0], [0, 0]])
            dst = np.float32([a, b, d, c])
            yield cv2.getPerspectiveTransform(src, dst).astype(np.float64)


def detect(frame):
    best = None
    for candidate in header_candidates(frame):
        grow = candidate[0]
        for H0 in hypotheses(candidate):
            if anchors(frame, H0) < 0.2:
                continue
            H, info = fit(frame, H0, radii=(0.4, 0.3, 0.2), grow=grow)
            if H is not None and (best is None or info["anchor"] > best[2]["anchor"]):
                best = (H0, H, info)
    return best


# ------------------------------------------------------------------- tracking

class OneEuro:
    def __init__(self, min_cutoff=1.5, beta=0.02, d_cutoff=1.0):
        self.min_cutoff, self.beta, self.d_cutoff = min_cutoff, beta, d_cutoff
        self.x = self.dx = self.t = None

    @staticmethod
    def alpha(cutoff, dt):
        tau = 1 / (2 * math.pi * cutoff)
        return 1 / (1 + tau / dt)

    def __call__(self, x, t):
        if self.x is None or t <= self.t:
            self.x, self.dx, self.t = x, np.zeros_like(x), t
            return x
        dt = t - self.t
        dx = (x - self.x) / dt
        self.dx = self.dx + self.alpha(self.d_cutoff, dt) * (dx - self.dx)
        cutoff = self.min_cutoff + self.beta * np.abs(self.dx)
        self.x = self.x + self.alpha(cutoff, dt) * (x - self.x)
        self.t = t
        return self.x


class Tracker:
    def __init__(self, refit_every=8):
        self.refit_every = refit_every
        self.H = None
        self.prev = None
        self.points = None
        self.since_fit = 0
        self.smooth = OneEuro()
        self.last_accepted = None  # (body, t)
        self.pending = None

    def features(self, frame, H):
        mask = np.zeros((frame.h, frame.w), np.uint8)
        outline = apply(H, [[-1, -1], [COLS, -1], [COLS, ROWS], [-1, ROWS]])
        cv2.fillConvexPoly(mask, outline.astype(np.int32), 255)
        pts = cv2.goodFeaturesToTrack(frame.u8, 200, 0.01, max(3, 0.35 * cell_px(H)), mask=mask, blockSize=5)
        return pts

    def jump(self, H, t):
        """Sudden area/angle change from the last accepted grid (within 0.3 s)."""
        if self.last_accepted is None or t - self.last_accepted[1] > 0.3:
            return False
        a, b = self.last_accepted[0], body_of(H)
        area = cv2.contourArea(b.astype(np.float32)) / max(cv2.contourArea(a.astype(np.float32)), 1)
        angle = lambda q: math.degrees(math.atan2(*(q[1] - q[0])[::-1]))
        turn = abs((angle(b) - angle(a) + 180) % 360 - 180)
        return not (0.6 < area < 1.65) or turn > 25

    def accept(self, frame, H, t):
        if self.jump(H, t):
            # A real move is confirmed by the next independent detection.
            confirmed = self.pending is not None and not _jump_between(self.pending, body_of(H))
            self.pending = body_of(H)
            if not confirmed:
                return False
        self.pending = None
        self.H = H
        self.last_accepted = (body_of(H), t)
        self.prev = frame
        self.points = self.features(frame, H)
        self.since_fit = 0
        return True

    def step(self, frame, t):
        stages, info, timing = {}, {}, {}
        mode = "lost"
        started = time.perf_counter()
        tracked = None
        if self.H is not None and self.points is not None and len(self.points) >= 8:
            nxt, status, _ = cv2.calcOpticalFlowPyrLK(self.prev.u8, frame.u8, self.points, None,
                                                      winSize=(21, 21), maxLevel=3)
            back, status_b, _ = cv2.calcOpticalFlowPyrLK(frame.u8, self.prev.u8, nxt, None,
                                                         winSize=(21, 21), maxLevel=3)
            good = (status.ravel() == 1) & (status_b.ravel() == 1) & \
                (np.linalg.norm((back - self.points).reshape(-1, 2), axis=1) < 1.0)
            if good.sum() >= 12:
                F, mask = cv2.findHomography(self.points[good], nxt[good], cv2.USAC_MAGSAC, 1.5)
                if F is not None:
                    keep = mask.ravel().astype(bool)
                    a = cv2.perspectiveTransform(self.points[good][keep], F)
                    residual = float(np.sqrt(np.mean(np.sum((a - nxt[good][keep]) ** 2, -1))) / cell_px(self.H))
                    info.update(trackInliers=int(keep.sum()), trackRatio=float(keep.sum() / len(self.points)),
                                trackResidual=residual)
                    if keep.sum() / len(self.points) >= 0.5 and residual < 0.15:
                        tracked = F @ self.H
                        self.points = nxt[good][keep].reshape(-1, 1, 2)
                        stages["track"] = tracked
        timing["track"] = (time.perf_counter() - started) * 1000
        H = None
        due = tracked is None or self.since_fit + 1 >= self.refit_every
        if tracked is not None and not due:
            H, mode = tracked, "track"
            self.prev, self.H = frame, tracked
            self.since_fit += 1
            if len(self.points) < 40:
                self.points = self.features(frame, tracked)
        else:
            started = time.perf_counter()
            seed = tracked if tracked is not None else self.H
            refit = None
            if seed is not None:
                refit, fit_info = fit(frame, seed, radii=(0.3, 0.2))
                info.update({"fit": fit_info})
            timing["refit"] = (time.perf_counter() - started) * 1000
            if refit is not None and self.accept(frame, refit, t):
                H, mode = refit, "refit"
                stages["fit"] = refit
            elif tracked is not None:
                # Healthy tracking survives one failed refit; detection waits.
                H, mode = tracked, "track"
                self.prev, self.H = frame, tracked
                self.since_fit += 1
            else:
                started = time.perf_counter()
                found = detect(frame)
                timing["detect"] = (time.perf_counter() - started) * 1000
                if found is not None:
                    seed, H_det, det_info = found
                    stages["detect"], stages["fit"] = seed, H_det
                    info.update({"fit": det_info})
                    if self.accept(frame, H_det, t):
                        H, mode = H_det, "detect"
                        self.smooth = OneEuro()
                if H is None:
                    self.H, self.points = None, None
        keyframe = mode in ("detect", "refit") and info.get("fit", {}).get("residual", 1) < 0.08
        if H is not None:
            stages["smoothed"] = None  # filled by caller with display corners
        return H, mode, stages, info, timing, keyframe


def _jump_between(a, b):
    area = cv2.contourArea(b.astype(np.float32)) / max(cv2.contourArea(a.astype(np.float32)), 1)
    return not (0.8 < area < 1.25) or np.max(np.linalg.norm(a - b, axis=1)) > 0.5 * np.linalg.norm(a[1] - a[0]) / COLS


def label_homography(frame, body):
    src = np.float32([[0, 0], [COLS, 0], [COLS, ROWS], [0, ROWS]])
    return cv2.getPerspectiveTransform(src, np.float32(np.array(body) * [frame.w, frame.h])).astype(np.float64)


def calibrate(frames, labels):
    """Median model position of the header top and label left edge under labels."""
    found = {"header-top": [], "labels-left": []}
    for name, value in sorted(labels.items()):
        frame = Frame(cv2.imread(str(frames / name)))
        H = label_homography(frame, value["body"] if isinstance(value, dict) else value)
        for spec, axis_value, horizontal, samples in line_specs():
            if spec not in found:
                continue
            points = [p for _, p, _ in edge_points(frame, H, samples, 0.45)]
            if len(points) >= 6:
                uv = apply(np.linalg.inv(H), np.float64(points))
                found[spec].append(float(np.median(uv[:, 1] if horizontal else uv[:, 0])))
    for spec, values in found.items():
        print(spec, len(values), "median", np.median(values), "iqr", np.percentile(values, [25, 75]))


def main():
    global GRAY_ONLY, WORK
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("frames", type=Path)
    p.add_argument("--calibrate", type=Path, help="labels JSON: print header/label constants")
    p.add_argument("--fps", type=float, default=30)
    p.add_argument("--refit-every", type=int, default=8)
    p.add_argument("--detect-only", action="store_true", help="independent detect+fit on every frame")
    p.add_argument("--gray-only", action="store_true", help="use luminance for the row steps too")
    p.add_argument("--work", type=int, default=960, help="long side of the analysed image")
    args = p.parse_args()
    GRAY_ONLY, WORK = args.gray_only, args.work
    cv2.setNumThreads(1)
    if args.calibrate:
        calibrate(args.frames, json.loads(args.calibrate.read_text()))
        return
    paths = sorted(q for q in args.frames.iterdir() if q.suffix in (".jpg", ".png"))
    stamps = {}
    index_file = args.frames / "frames.jsonl"
    if index_file.exists():
        for line in index_file.read_text().splitlines():
            if line.strip():
                v = json.loads(line)
                stamps[v["file"]] = (v["frameId"], v["capturedMs"])
    origin = stamps[paths[0].name][1] if paths and paths[0].name in stamps else 0
    tracker = Tracker(args.refit_every)
    for index, path in enumerate(paths):
        frame_id, t = (stamps[path.name][0], (stamps[path.name][1] - origin) / 1000) if path.name in stamps \
            else (index, index / args.fps)
        bgr = cv2.imread(str(path))
        started = time.perf_counter()
        frame = Frame(bgr)
        prepare_ms = (time.perf_counter() - started) * 1000
        if args.detect_only:
            tracker = Tracker(args.refit_every)
        H, mode, stages, info, timing, keyframe = tracker.step(frame, t)
        total = (time.perf_counter() - started) * 1000
        norm = lambda M: (body_of(M) / [frame.w, frame.h]).tolist()
        out_stages = {k: norm(v) for k, v in stages.items() if v is not None}
        if H is not None:
            display = tracker.smooth(body_of(H).ravel(), t).reshape(4, 2)
            out_stages["smoothed"] = (display / [frame.w, frame.h]).tolist()
        row = {"frame": path.name, "frameId": frame_id, "t": t, "pipeline": "opencv-reference",
               "mode": mode, "keyframe": bool(keyframe), "stages": out_stages,
               "final": norm(H) if H is not None else None, "info": info,
               "ms": {"prepare": prepare_ms, **timing, "total": total}}
        sys.stdout.write(json.dumps(row) + "\n")


if __name__ == "__main__":
    main()
