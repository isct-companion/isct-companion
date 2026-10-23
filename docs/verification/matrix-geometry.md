# Matrix camera geometry rewrite: report and replay

Branch `claude/matrix-cv-rewrite-*`, based on `codex/ocr-locator-tuning`
(96a97f3f, the pipeline with the colour bands and the dispatcher). Character
recognition and E/H tuning are out of scope; every number here is geometry or
timing. Deep verification is left to a separate pass: the replay commands are
at the end.

## What changed

| | before | after |
|---|---|---|
| geometry per frame | full locate from scratch (coarse 640 px, colour/luma band refinement) | detect once, then track; refit every 8 frames or on weak tracking |
| fit | 4 body corners from band refinement; grid support only scored | one homography over every printed-line intersection (RANSAC + DLT + LM), with hard gates |
| per frame work | decode JPEG, locate, cut and score 70 cells (≈62 ms) | track the 960 px planes (≈15 ms); cells only on keyframes (≈29 ms, off the fast loop) |
| transport | 2200 px JPEG as base64 text | 960 px luma + R−G raw bytes; full-res luma only for a requested keyframe |
| drawn box | latest Core bounds over the live video | the analysed frame painted with the outline fitted on it |
| frame identity | none | frame id, Core sequence and `boundsFrameId`; per-frame trace and timeline in `latest.json` |

Core keeps the session, deadline, admission, dispatcher and voting. Its new
`track()` owns the drawn outline and decides when a keyframe is due (a frame
that qualifies, at most every 100 ms); `observe()` takes the keyframe's cells
as before.

## Data and ground truth

Two local recordings of the same card from one 59.5 s phone video, frames
scaled to 2200 px wide: **continuous** (300 consecutive frames at 30 fps, hand
motion and blur) and **sampled** (119 frames at 2 fps across the whole video,
every frame analysed independently). Labels come from
`scripts/ocr/matrix_sift_labels.py`: SIFT matches against one measured still
photo, a RANSAC homography, strict coverage gates; no grid detector involved.
116/300 and 45/119 frames are labelled; the rest are not guessed. A labelled
frame is **missed** with no grid, **wrong** when the worst body corner is more
than ¼ cell from the label; **geometry failure rate** = (missed + wrong) /
labelled. Frames, labels, traces and overlays are images of a real card and stay
in the local `.cache/matrix-cv-rewrite/`; only aggregate numbers are committed
(`matrix-geometry/*.json`).

Offline timing is the Rust code on an M-series Mac, one process, excluding
camera, webview and IPC. The live timeline is simulated from the recorded
capture times: a latest-frame-only loop (newest frame when free, older ones
dropped), each frame costing its measured time, the box drawn at the next
60 Hz refresh.

## Before / after (continuous, 300 frames)

| | before | after | OpenCV reference |
|---|---:|---:|---:|
| **geometry failure rate** (116 labelled) | **30.2 %** | **1.7 %** | 0.0 % |
| labelled missed / wrong ¼ cell / wrong ½ cell | 20 / 15 / 9 | 0 / 2 / 0 | 0 / 0 / 0 |
| corner error p50 / p95 (cells) | 0.068 / 2.57 | 0.069 / 0.21 | 0.054 / 0.17 |
| **localisation rate** (located / analysed) | **59.0 %** (177) | **81.0 %** (243) | 81.7 % (245) |
| **processing**: per-frame ms p50 / p95 | 61.6 / 85.1 | 24.1 / 78.5 | Python |
| throughput, frames/s (1000 / mean ms) | 15.6 | 32.8 | Python |
| simulated frames analysed / s (30 fps camera) | 15.9 | 26.1 | — |
| simulated located boxes / s | 9.1 | 21.8 | — |
| **box-draw latency** p50 / p95 (simulated) | **100 / 117 ms** | **17 / 100 ms** | — |
| box older than the picture under it, p50 / p95 | 67 / 100 ms | 0 ms by construction | — |

After: tracked frames cost 14.5 ms p50, refits 24 ms, detections 67 ms, and a
lost frame (detection that finds nothing) 43 ms; 38 keyframes were cut at
29 ms p50 on the slow path. "Box older than the picture" is how far behind the
picture the box was measured. Before, Core's latest bounds were drawn over the
live video, 67–100 ms after their frame. After, the webview paints the analysed
frame together with its own outline, so the two always match; the simulated
"age" column for the new loop (p50 0, p95 72 ms) is then how old the displayed
picture is, a display lag shared by picture and box.

The device report from before this work (77 captures in 5.6 s, 65–95 ms per
round trip, with JPEG encoding, base64 and IPC on top of the 62 ms analysis) is
the live baseline. The live numbers for the new loop come from
`latest.json` in manual check `OCR-CAMERA-GEOMETRY`; they were not measured
here.

## Independent frames (no tracking)

| | before | after, detect only | reference, detect only |
|---|---:|---:|---:|
| sampled 2 fps: geometry failure rate (45 labelled) | 71.1 % | 35.6 % | 22.2 % |
| sampled: missed / wrong ¼ cell | 26 / 6 | 16 / 0 | 10 / 0 |
| continuous: geometry failure rate (116 labelled) | 30.2 % | 11.2 % | 4.3 % |
| continuous: missed / wrong ¼ cell | 20 / 15 | 12 / 1 | 4 / 1 |
| detection ms p50 / p95 | 61.6 / 85.1 (locate) | 61.0 / 96.0 | Python |

Detection from scratch rarely returns a wrong grid (one labelled ¼-cell error
per run); its misses are frames where the header is out of view or blur
erases the row steps. Live, tracking carries the grid across those frames.

## Rust against the OpenCV reference, frame by frame

`scripts/ocr/matrix_trace_parity.py` (`matrix-geometry/parity.json`):

| | both located | only reference | only Rust | difference p50 / p95 (cells) | > ¼ cell |
|---|---:|---:|---:|---:|---:|
| continuous, detect only | 186 | 20 | 8 | 0.028 / 0.090 | 1 |
| sampled, detect only | 51 | 10 | 1 | 0.033 / 0.062 | 0 |
| continuous, tracking | 240 | 5 | 3 | 0.062 / 0.296 | 17 |

Where both detect, the fits agree to about 0.03 cell. The one detect-only frame
over ¼ cell (continuous 0097, unlabelled, heavy motion blur) is a Rust grid
about one row low on the right; it passed every gate. Tracking agrees less
closely frame by frame (different corner detector windows, RANSAC instead of
MAGSAC for frame-to-frame motion) while both stay on the labels. Rust finds
fewer grids from scratch than the reference; the reference fits every seed
above its prior while Rust fits the 10 strongest to bound detection at about
60 ms (fitting all of them costs 182 ms p50 for 4 more detections in 300).

## How the geometry works

The printed card has no lines between columns: cells in a row form one pink or
white band. The usable structure is the dark header band (1.05 rows tall), the
dark/grey label column (1.03 columns wide), alternating pink (rows 1, 3, 5, 7)
and white rows, and the body edges. Both constants were measured on labelled
frames (`matrix_cv_reference.py --calibrate`).

- **Detect**: dark masks (two local-mean thresholds, two fixed levels) give
  straight band edges with a consistent dark depth behind them; each band
  yields four homography seeds (which side is the body, which end is A or 1).
  Seeds are ranked by their anchor score and the 10 strongest are fitted,
  growing from the seed's own band to the full grid.
- **Fit**: near the prediction, 20 edge samples per printed line, taken in the
  ink-free gaps between letters, locate the row boundaries as signed R−G steps
  (row parity fixes the sign) and the label and body edges as luminance steps.
  Each line is fitted robustly, the lines are intersected, and one homography
  is fitted to the up-to-24 intersections. The header top seeds detection and
  anchors indexing but is not fitted, because its height varies more than the
  body lines.
- **Gates**: inliers on both body edges, each from the top to the bottom rows;
  residual ≤ 0.08 cell; convex and clockwise (a mirror passes the symmetric row
  pattern otherwise); condition number and foreshortening bounds; the printed
  anchors must beat every half- and whole-cell shift by 0.3 (off-by-one
  indexing); column gaps clear of ink (a grid squeezed along a row). No fixed
  image aspect ratio is imposed.
- **Track**: Shi-Tomasi corners inside the card, pyramidal Lucas-Kanade with a
  forward-backward check, a RANSAC frame-to-frame homography. Refit seeded by
  the track every 8 frames or when inliers drop below half or the residual
  exceeds 0.15 cell; detection only when nothing survives. A sudden area or
  angle change needs a second agreeing measurement. A One Euro filter smooths
  the drawn corners; crops use the unsmoothed fit.
- **Keyframes**: a frame whose grid was just fitted with residual < 0.08 cell,
  requested by Core at most every 100 ms; its cells are cut from the
  full-resolution luma of the same frame.

Findings along the way that shaped this:

- **Row steps need colour.** Read from luminance only, every labelled frame
  failed (the fit settles half a row off), so the fast transport carries an
  R−G plane.
- **Flat-topped step responses.** A sharp edge gives a plateau as wide as the
  derivative window; taking its first (numpy) or last (Rust `max_by`) sample
  biased fits by 4 px on synthetic cards. Both implementations now take the
  plateau's middle and reject one clipped by the search window.
- **Bent cards.** A hard 0.05-cell RANSAC threshold let one side of a curled
  card outvote the other; the consensus uses 0.1 cell and the 0.08-cell RMS
  gate keeps the precision.
- **Pixel conventions.** Analysis planes put pixel centres at integers while
  normalized quads and crops use pixel edges; normalizing with `(x + 0.5) / w`
  removed a half-pixel bias that cost crop accuracy at full resolution.
- The printed pink is far less saturated in camera frames (R−G ≈ 15–25) than
  in the synthetic fixtures (≈ 67). Geometry works at that contrast; the
  existing glare check relies on a saturated pink and was not tuned here.

## Code and dependencies

- `crates/isct-ocr/src/engine/grid/` (detect, fit, track) replaces
  `scan_image_locator.rs` and `color_refinement.rs`; still photos use the same
  detector. Engine production code: +1985 / −2401 lines. Over the branch,
  production code is +2652 / −2443 (Core trace and track API, desktop
  transport and recorder, webview loop), tests +468 / −278, examples
  +155 / −32 and dev scripts +1226 / −20, all excluding generated types and
  fixtures.
- `engine/vendor/opencv_homography.rs`: OpenCV 4.10.0's RANSAC `findHomography`
  (normalized DLT, RANSAC registrator, Levenberg-Marquardt refinement), ported
  to Rust, with its license headers and modifications noted. Full texts are in
  `THIRD_PARTY_NOTICES.md`.
- `optical-flow-lk = "=0.6.0"` (MIT, depends only on `image`) for pyramidal
  LK and Shi-Tomasi corners, with its f32 reference kernel selected rather than
  that release's new fixed-point SIMD path. `nalgebra` 0.35 was already in the
  lockfile through imageproc. No native C++ dependency.
- Fixtures `matrix-card.jpg` and `matrix-letter-boundaries.jpg` were repainted
  in the printed layout by `scripts/ocr/matrix_fixture_layout.py` (same glyphs
  and geometry; white rows, no column lines, measured header/label sizes).

## Replay

Paths are local; frames, labels and outputs are private card images.

```sh
# Rust tracker over images or an app recording (ISCT_MATRIX_RECORD_DIR session folder)
cargo run -p isct-ocr --release --features training-tools --example matrix_track_replay -- FRAMES > after.jsonl
cargo run -p isct-ocr --release --features training-tools --example matrix_track_replay -- FRAMES --detect-only > after-detect.jsonl
# The pipeline before this branch, from its Phase 1 commit (stage quads: coarse/color/luma)
git worktree add /tmp/before aae9d4c4 && (cd /tmp/before && cargo run -p isct-ocr --release --features training-tools --example matrix_track_replay -- FRAMES) > before.jsonl
# Independent labels and the OpenCV reference (Python with opencv-python, numpy)
python scripts/ocr/matrix_sift_labels.py --measurement measurement.json --frames FRAMES --out labels.json
python scripts/ocr/matrix_cv_reference.py FRAMES [--detect-only] > reference.jsonl
# Metrics, overlays and timeline; frame-by-frame agreement
python scripts/ocr/matrix_track_report.py --frames FRAMES --trace before=before.jsonl \
  --trace after=after.jsonl --trace reference=reference.jsonl --labels sift=labels.json \
  [--live latest.json] --out REPORT
python scripts/ocr/matrix_trace_parity.py --frames FRAMES reference.jsonl after.jsonl
```

`measurement.json` names a still photo of the card (`source`), its body
corners (`bodyCorners`) and an optional cv2 `rotate` code. The report's
overlays draw each stage (seed, fit, track, smoothed; or coarse, colour, luma
for the old pipeline), the final 11×8 grid and the label, which separates
"detection is off" (the seed or fit is wrong), "rectification is off" (the
fit is right and the grid is not) and "old box on a new frame" (the timeline's
box age).
