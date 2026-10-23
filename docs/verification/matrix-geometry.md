# Matrix geometry rewrite: diagnostics, reference and comparison

Working notes and results for the camera geometry rewrite (branch
`claude/matrix-cv-rewrite-*`, based on `codex/ocr-locator-tuning`). Character
recognition is out of scope; every number here is geometry and timing.

## Data and ground truth

Two local recordings of the same physical card, from one 59.5 s phone video
(`IMG_5265.MOV`, 1920×1080 HEVC), scaled to 2200 px wide:

- **sampled**: 119 frames at 2 fps over the whole video (large pose changes);
- **continuous**: 300 consecutive frames at 30 fps (26–36 s; hand motion, blur).

Labels come from `scripts/ocr/matrix_sift_labels.py`: SIFT matches between each
frame and one measured still photo of the card, a RANSAC homography, and strict
inlier/coverage gates. No grid detector is involved. 45/119 and 116/300 frames
are labelled; the rest are left unlabelled rather than guessed. A body whose
worst corner is more than ¼ cell from the label is **wrong**; a labelled frame
with no body is **missed**; the **geometry failure rate** is
(missed + wrong) / labelled. Frames, labels, traces and overlays are card
images: they stay under the local `.cache/matrix-cv-rewrite/`, only aggregate
metrics are committed.

The three live metrics are kept apart:

- **processing FPS**: frames the geometry loop finishes per second;
- **localisation rate**: located frames / processed frames;
- **box-draw latency**: outline drawn minus capture of the frame it was measured
  on. **Box age** is how much older that frame is than the video frame on
  screen when the outline is drawn.

Offline, a latest-frame-only loop is simulated over the recorded capture times
and each frame's measured cost (30 fps camera, 60 Hz display, no transport
cost). In the app, `latest.json` now records the measured version of all three
(see `crates/isct-ocr/README.md`, *Geometry replay and capture timeline*).

## Phase 1: what the current pipeline gets wrong

Overlays of the coarse (orange), colour-refined (magenta), luminance-refined
(cyan) and final grid (red) against the label (white) separate the causes:

- **Detection is off**: most wrong grids are already wrong at the coarse stage;
  colour refinement keeps them. Typical: glare or blur on columns H–J shrinks
  the body to 7–8 columns, or the body slips one row. The stage overlays show
  the coarse quad and the final grid agreeing with each other, not the label.
- **Rectification is off**: rare. When the coarse body is right, the final
  grid's error is small (median 0.07 cell).
- **Old box on a new frame**: structural. Every frame is located from scratch
  (decode + locate + 70 crops ≈ 62 ms), so at most ~16 frames/s are analysed
  and the outline drawn is 67–100 ms older than the video under it, before any
  JPEG/IPC cost. The device report from before this change (77 captures in
  5.6 s, 65–95 ms per round trip) is consistent with that.

## Phase 2: OpenCV reference (`scripts/ocr/matrix_cv_reference.py`)

The printed card has no column lines inside the body (cells in a row form one
pink or white band), so "grid-line intersections" are where the 9 horizontal
printed lines (header top, the 8 row boundaries) cross the 3 vertical ones
(label column left edge, body left and right edges). Each line is fitted from
20 edge samples (signed luminance steps for header/label/body edges, signed
pink/white R−G steps whose sign is fixed by row parity), and one homography is
fitted to the up-to-27 intersections with `cv2.findHomography(USAC_MAGSAC)`.
Header height (1.05 cells) and label width (1.03 cells) were calibrated on
labelled frames (`--calibrate`).

Gates: inliers on both body edges and on top and bottom rows; residual
≤ 0.08 cell; convex, unclipped body; condition number and foreshortening
bounds; anchor score (signed strength of every printed boundary) beating every
±1 row/column shift by 0.3; column-gap check (strips between letters clear of
ink, ≥ 70 %), which rejects grids squeezed along a row. No fixed aspect ratio.

Tracking: KLT on Shi-Tomasi corners inside the card (letters and junctions; the
row boundaries are colour-only and weak in grey), forward–backward checked,
RANSAC frame-to-frame homography. Refit seeded by the tracked grid every 8
frames or when inliers < 50 % / residual > 0.15 cell; detection only when no
grid survives. A sudden area/angle change needs a second detection to confirm.
Displayed corners use a One Euro filter; crops use the unsmoothed fit.

| continuous, 300 frames | current Rust | reference, detect each frame | reference, tracking |
|---|---:|---:|---:|
| located frames | 177 (59 %) | 211 (70 %) | 251 (84 %) |
| labelled: missed / wrong ¼ cell / wrong ½ cell | 20 / 15 / 9 | 7 / 0 / 0 | 0 / 2 / 0 |
| **geometry failure rate** | **30.2 %** | **6.0 %** | **1.7 %** |
| corner error p50 / p95 (cells) | 0.068 / 2.57 | 0.051 / 0.12 | 0.057 / 0.13 |
| frames tracked / refitted / detected | — | — / — / 211 | 212 / 29 / 10 |

| sampled, 119 frames, every frame independent | current Rust | reference |
|---|---:|---:|
| located frames | 44 | 62 |
| labelled: missed / wrong ¼ cell | 26 / 6 | 11 / 0 |
| **geometry failure rate** | **71.1 %** | **24.4 %** |
| corner error p50 / p95 (cells) | 0.091 / 2.90 | 0.047 / 0.14 |

Detection from scratch never produced a wrong grid. Its misses are frames where
the header leaves the picture or blur removes the row steps; tracking carries
the grid through those. The two tracked ¼-cell errors are drift between refits.

Timing: the current pipeline's 62 ms p50 includes 35 ms of JPEG decode. The
reference is Python and its committed timings were taken with three replays
running at once (a lone run tracks at 24 ms p50, 20 ms of it resizing); they
are not a performance target. Timing claims belong to the Rust implementation.

**Row steps need colour.** Sampling row steps in the ink-free gaps between
letters (instead of through them) took tracking from 3.4 % to 1.7 %. With the
same code reading the row steps from luminance only (`--gray-only`), every
labelled frame came out wrong: pink and white rows differ by only ~25 grey
levels next to thin dark row lines, and the fit settles about half a row off
while still passing every gate (100 % failure, p50 0.43 cell). The fast
transport therefore has to keep a colour-difference plane, not luminance alone.

Replay (paths are local):

```sh
cargo run -p isct-ocr --release --features training-tools --example matrix_track_replay -- FRAMES > current.jsonl
.cache/ocr-venv/bin/python scripts/ocr/matrix_cv_reference.py FRAMES > reference.jsonl
.cache/ocr-venv/bin/python scripts/ocr/matrix_sift_labels.py --measurement measurement.json --frames FRAMES --out labels.json
.cache/ocr-venv/bin/python scripts/ocr/matrix_track_report.py --frames FRAMES \
  --trace current=current.jsonl --trace reference=reference.jsonl --labels sift=labels.json --out REPORT
```

Summaries: `matrix-geometry/phase2-*.json`, timeline `phase2-continuous-timeline.png`.
