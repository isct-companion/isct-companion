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
| Dim and rotated | 58 → 70 | 0 → 0 | 8.0 → 6.0 s |
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

## What changed

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
