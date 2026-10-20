# Matrix camera lighting replay

2026-10-20. Baseline: `07bc5924` (including the morning dim/rotated
localization fix), with the bundled printed alphabet model. Both binaries run
the real Rust geometry, capture admission, candidate retention, classifier and
Core voting. The baseline binary includes only the new diagnostic reporting.

The public Helvetica card fixture is perturbed into 12 deterministic sequences,
45 frames each at 5 fps. Capture times below follow simulated recording time;
they exclude physical exposure, native camera transport and UI overhead.
These sequences are an engineering regression benchmark, not real-card accuracy.
Hashes and measured processing times are in
[`matrix-lighting-results.json`](matrix-lighting-results.json).

| Condition | Correct cells, before → after | Stable wrong cells, before → after | Capture, before → after |
| --- | ---: | ---: | --- |
| Clean | 70 → 70 | 0 → 0 | 0.8 → 0.8 s |
| Dim, 32% exposure | 52 → 70 | 0 → 0 | 8.0 → 0.8 s |
| Very dim, 15% exposure | 0 → 70 | 0 → 0 | Never admitted → 0.8 s |
| Dim, 640 × 440 input | 0 → 70 | 0 → 0 | Never admitted → 0.8 s |
| Uneven shadow | 48 → 70 | 0 → 0 | 8.0 → 0.8 s |
| Dim, 90° rotation with clipped row-label margin | 58 → 70 | 0 → 0 | 8.0 → 6.0 s |
| Fixed glare spot | 64 → 64 | 0 → 0 | 8.0 → 8.0 s |
| Moving glare spot | 70 → 70 | 0 → 0 | 1.6 → 1.6 s |
| Horizontal glare band | 52 → 51 | 3 → 0 | 8.0 → 8.0 s |
| Blur | 70 → 70 | 0 → 0 | 0.8 → 0.8 s |
| 12° tilt | 70 → 70 | 0 → 0 | 2.4 → 2.4 s |
| No card | No geometry → no geometry | 0 → 0 | Not started |

The small dim sequence has 70 correct readings but 69 settled cells; one remains
for review. Fixed glare retains six unreadable cells. The glare band loses one
additional correct reading while eliminating three wrong settled readings;
other low-quality crops remain unsettled. The algorithm does not fill erased
strokes.

## Follow-up: body calibration after physical-card feedback

The table above records the first illumination checkpoint. Subsequent local
inspection of 12 explicitly authorized anonymous glyph crops found grid borders
and clipped letter bottoms. Those private images remain local and are excluded
from this report and training.

The contour path assumed the header was one body row tall. It now measures the
label bands independently and fits internal line centers before dividing the
body. Both axes must supply enough grid lines: a reflection that splits the
outline must not let a partial card pass on vertical evidence alone. The header
fallback breaks tied row-pitch scores with line coverage instead of selecting
the shortest pitch. Fully saturated cells also compare their paper level with
the rest of the card, so a reflection cannot evade rejection by erasing its
local brightness gradient.

With unchanged model weights, the same lighting replay now reads 60 rather than
51 cells under the fixed horizontal glare band; the ten obscured cells remain
for review. All 12 conditions have zero wrong settled cells. Clean, dim,
very-dim, small-dim, shadow, moving-glare, blur and tilt retain 70 correct cells.
The clipped-margin dim/rotated sequence regresses from 70 to 68 correct cells,
both missing cells remaining unsettled; this edge case remains open.
The fixed glare spot retains 64 correct cells and the blank sequence has no
geometry.

A separate public synthetic Times New Roman card uses a 48 × 42 px body cell,
header heights 22/34/42/58 px and grid intensities 35/95/135. Before calibration,
the twelve variants returned 35–67 correct cells; afterward each returns 67/70
with zero wrong settled cells. The three repeated Q glyphs remain unsettled.
These are controlled geometry probes, not a physical-card accuracy estimate.
Summaries and the lighting binary hash are in the `gridCalibration` section of
`matrix-lighting-results.json`.

The OCR suite passes 21 tests, with four private-input tests ignored. It now
protects independent header height, subpixel shifts under a glare stripe and
uniform-white versus relative saturation. Physical verification remains
`OCR-CAMPUS-CARD`, including the actual outline refresh frequency.
This follow-up adds 193 and removes 38 production Rust lines (net +155; no
production-code reduction). Test changes add 66 and remove one line; no test
is removed. Documentation and measured JSON are counted separately.

## What changed

### Complete-cell input after L/Z/I false acceptance

The next report from a physical-card session identified substitutions in cells
presented as normal results. A public Times Italic L/Z/I card reproduced this
through production Rust acquisition, classification and voting: shifting the
complete letters down four source pixels yielded 70 settled cells with 24 wrong
accepted Ls. The letters still fit inside their grid cells. The fixed 9% inset
removed distinguishing strokes before inference; five identical observations
then repeated the same mistake. Synthetic-font classifier accuracy and temporal
agreement therefore did not validate the full input pipeline.

The camera now retains the complete cell. Texture scoring uses an interior
window separately. Offline recognition locates ink using an illumination-adjusted
mask and imageproc connected components, distinguishes boundary grid lines,
then scales the original grayscale glyph proportionally onto a padded canvas.
Only an actual boundary/central-ink join triggers grid-line separation; unresolved
boundary-connected ink is explicitly unreadable. No model weights, confidence
thresholds or voting thresholds changed.

| Public L/Z/I placement | Correct before → after | Wrong settled before → after |
| --- | ---: | ---: |
| Four pixels up | 70 → 70 | 0 → 0 |
| Centered | 40 → 70 | 0 → 0 |
| Four pixels down | 46 → 70 | 24 → 0 |

All 12 lighting sequences have zero wrong settled cells. Clean, dim, very dim,
small dim, shadow, clipped-margin dim rotation, moving glare, blur and tilt each
have 70 correct readings. Small dim has 67 settled cells (previously 69), so
three require review. Fixed glare retains 64 correct/settled cells; the stripe
retains 60; the blank sequence has no geometry. These public probes do not
establish physical-card accuracy or the actual outline frame rate.

The `completeCellInput` section of `matrix-lighting-results.json` records both
replay binaries, the font/fixture hashes and results. The committed
`matrix-letter-boundaries.jpg` fixture is the generated four-pixel-down probe,
not a private card. A new Rust case protects the complete letter identities and
explicit rejection of ambiguous edge-connected ink. All 22 OCR tests pass;
four private-input tests remain ignored, and OCR Clippy passes with warnings
denied. No test was removed. Production Rust adds 167 and removes 10 lines
(net +157; no production-code reduction); tests add 34 lines, with scripts,
fixtures, documentation and measured JSON counted separately.

The reproducible boundary benchmark exits nonzero for wrong accepted cells or
missing/wrong readings:

```sh
python scripts/ocr/matrix_letter_boundaries.py \
  --directory .cache/ocr-letter-boundaries --font "/path/to/Times Italic.ttf" \
  --binary target/release/examples/matrix_capture \
  --output .cache/ocr-letter-boundaries/results.json
```

Use `--font` only to generate, then omit it when comparing binaries over the same
frames. The intentionally identical frames expose repeated systematic errors;
they do not represent independent visual evidence. Model promotion must pass
both the original lighting replay and these boundary cases. Further font and
camera coverage remains necessary before treating agreement as reliability.

### Initial illumination checkpoint

The two admission failures already had geometry on every frame. Their raw
contrast/sharpness was insufficient for the 35-cell startup gate. Measuring
texture after bounded local illumination correction makes these readable dark
cells eligible without lowering the existing thresholds. Candidates retain raw
grayscale pixels.

The original geometry detector still runs first. If it fails, the same detector
and label/grid validation run on a corrected luminance image. A Rust regression
also covers a slightly different dark JPEG that failed geometry, plus rejection
of blank images and unlabelled cropped grids.

Core independently rejects cells with clipped highlights and uneven estimated
paper brightness. Uniform white paper remains valid. Offline classification
compares raw/corrected readings when needed, rejects confident disagreements and
gives each observation at most one vote. Normalized readings add inference cost:
the dim sequence's measured offline processing was approximately 0.39 → 0.87 s,
while capture dropped from 8.0 to 0.8 simulated seconds.

Diagnostics contain no letters or image pixels:

- `geometryFrames`: valid located frames processed by Core before sealing.
- `admissionRejections`: located frames rejected before acquisition started.
- `glareRejections`: per-cell observations rejected due to highlights.

## Reproduce

Use Python with NumPy and Pillow. Build the baseline replay binary before applying
the recognition changes, or retain a binary from the baseline implementation with
the diagnostic-only example changes. Keep it outside `target/` so later builds
do not replace it.

```sh
cargo build -p isct-ocr --example matrix_capture --release --locked
python scripts/ocr/matrix_lighting.py --directory .cache/ocr-lighting \
  --generate crates/isct-ocr/src/tests/fixtures/matrix-card.jpg
python scripts/ocr/matrix_lighting.py --directory .cache/ocr-lighting \
  --binary BASELINE_BINARY --output .cache/ocr-lighting/baseline.json
python scripts/ocr/matrix_lighting.py --directory .cache/ocr-lighting \
  --binary target/release/examples/matrix_capture \
  --output .cache/ocr-lighting/corrected.json
```

The replay reports `noGeometry`, `notAdmitted`, `recordingTooShort` or `processed`.
Missing detections are retained in the results; failed model execution still
fails the command. A private recorded sequence can use `matrix_capture` directly
with a separate expected-text file. Do not upload private recordings.

## Verification

Rust tests cover dark-frame localization/admission, original cell identities,
white-paper versus highlight measurements, per-cell rejection/recovery, the
unchanged deadline, and existing rotated-card behavior. Physical exposure,
reflection movement and native first-detection feedback are covered by
`OCR-CAMPUS-CARD` in `manual-checks.yaml`; that hardware check needs to be rerun.

## TSUBAME training decision

Two eight-epoch experiments ran through `iqrsh` on an H100 MIG device. Only
generated letters were transferred. New lighting samples used Arial/Tahoma variants;
their validation used held-out Verdana variants. Clean replay also includes the
original serif/monospace families and its Verdana/Trebuchet validation split.
The full-card Helvetica replay was
not part of training or checkpoint selection.

| Experiment | Lighting validation | High-confidence wrong letters | Clean validation | Decision |
| --- | --- | --- | --- | --- |
| Bundled model | 1970/2600 (75.77%) | 364 | 2600/2600 | Baseline |
| Lighting only, best epoch 6 | 2568/2600 (98.77%) | 8 | Full-card clean: 66/70; blur: 20/70 | Rejected |
| Lighting + clean replay, final epoch 8 | 2466/2600 (94.85%) | 32 | 2597/2600 | Rejected |

The second experiment mixed 26,000 lighting samples with 26,000 original clean
samples, trained only the last four convolutions and classifier, and kept batch
normalization frozen. Its selection rule prioritizes clean validation accuracy
before lighting accuracy, qualified errors and loss. Every epoch lost at least
one clean sample, so it exported epoch 0: **the exact original model hash**,
`592d61903de1f04b48079c44be5c247cd2f16bd323b23fb539b2c7392bfc5177`.
Batch sizes 1, 7 and 10 also produced identical logits.

The app therefore keeps its existing weights. The engineering replay table above
measures illumination correction and glare admission with that model. The
training results show why single-crop validation alone cannot approve a model:
the lighting-only model improved its validation score while rejecting readable
ordinary letters and blurred cards.

The trainer accepts `--replay-data`, `--train-convs`, `--learning-rate` and
`--resume-reference`. It checks finite loss, model-conversion parity and exported
ONNX parity, and records the selected epoch. Re-exported ONNX graphs may fail
round-trip conversion through onnx2torch; use the upstream graph plus its saved
PyTorch checkpoint, then require that resumed outputs match the bundled ONNX:

```sh
python scripts/ocr/alphabet_synthetic.py --fonts FONT_DIRECTORY \
  --output lighting.npz --lighting
iqrsh -g GROUP -l h_rt=0:30:00 python scripts/ocr/alphabet_finetune.py \
  --model upstream.onnx --resume printed-best.pt \
  --resume-reference alphabet-printed.onnx --data lighting.npz \
  --replay-data original-print.npz --train-convs 4 \
  --epochs 8 --learning-rate 0.00005 --output RUN_DIRECTORY
```

Full metrics, input/output hashes and the failed promotion's whole-card results
are retained in `matrix-lighting-results.json`.

Checkpoint `5b07f96` adds 133 and removes 5 production Rust lines (net +128;
no production-code reduction). Tests add 107 lines; replay examples/scripts add
124 net lines. Documentation, measured JSON and the generated manual table are
counted separately. No test was removed.
