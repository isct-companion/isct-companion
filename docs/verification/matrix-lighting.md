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
ONNX parity, and records the selected epoch. `alphabet_model.py` rebuilds the
network natively (MobileNetV2 with folded batch norms) and copies the ONNX
weights in, checked against ONNX Runtime, so re-exported graphs load as well
as the upstream one (onnx2torch could not convert them):

```sh
python scripts/ocr/alphabet_synthetic.py --fonts FONT_DIRECTORY \
  --prepare-binary target/release/examples/alphabet_prepare \
  --output lighting.npz --lighting
iqrsh -l h_rt=0:30:00 python scripts/ocr/alphabet_finetune.py \
  --model upstream.onnx --resume printed-best.pt \
  --resume-reference alphabet-printed.onnx --data lighting.npz \
  --replay-data original-print.npz --train-convs 4 \
  --epochs 8 --learning-rate 0.00005 --output RUN_DIRECTORY
```

Full metrics, input/output hashes and the failed promotion's whole-card results
are retained in `matrix-lighting-results.json`.

### Production-preprocessed adaptation and confidence calibration

The subsequent physical scan was reported all correct and fast, but ten cells
remained unsettled. Its numeric diagnostics showed nine cells with fewer than
three qualified votes and one conflict. The displayed score combines vote
agreement and classifier confidence; it is not a calibrated probability of
physical-card correctness.

Python now generates only the source glyph image. `alphabet_prepare`, enabled by
the `training-tools` Cargo feature, calls the same Rust
`prepare_matrix_alphabet_input` used by `MatrixAlphabetService::recognize`.
Ink segmentation, edge handling, aspect-preserving normalization and illumination
correction therefore have one implementation. The binary streams fixed-size
records containing rejection status and both grayscale views. Python performs
only the same final `pixel / 127.5 - 1` tensor conversion as Rust.

The four font-family splits are Arial/Times/Courier for training, Verdana for
checkpoint selection, Trebuchet for temperature fitting and Georgia for final
testing. There are 26,000 generated training sources and 3,900 sources in each
held-out split. Half the sources enable lighting perturbations; all splits also
include blur, grid fragments and explicit blanks. Rejected preprocessing
sources stay in evaluation as blank predictions. Raw/corrected views contribute
one source's weight in calibration, and evaluation reproduces the production
confidence-conflict rule. These deliberately difficult synthetic distributions
are not estimates of physical-card accuracy.

The first adaptation exposed a label defect in the old generator: motion-kernel
width or erased-ink area alone relabelled still-readable N/T/H/I glyphs as blank.
That run reduced apparent errors by learning more rejection and was discarded.
The corrected generator requires Rust preprocessing and labels these ambiguous
sources `-1`, rather than assigning an unsupported blank truth. They remain in
held-out artifacts with separate source/acceptance counts, but do not contribute
to supervised training, accuracy or temperature fitting. Deterministic generation
uses a new seed for each label-policy checkpoint; checkpoints must preserve
correct qualified coverage, accuracy and wrong-accepted counts.

Full-card replay exposed a second ambiguity: generated blank rectangles can
normalize to the same pixels as a sans-serif I/L/T. These are also labelled
unknown whenever Rust retains a glyph; only explicit no-glyph rejections remain
certain blank sources. The final dataset uses seed 20261031. Unknown-source
acceptance must not increase when selecting a checkpoint or applying temperature
scaling. The unknown set remains a separate stress indicator, not labelled error
ground truth.

The data/parity/calibration tools require NumPy, Pillow, ONNX, ONNX Runtime and
SciPy; fine-tuning additionally uses PyTorch and torchvision. Pinned versions
are in `scripts/ocr/requirements.txt`.
Training and calibration reject legacy raw-only NPZs. Regenerate replay datasets
through Rust too; their evaluation uses the same two-view fusion and rejection
records as the primary dataset.

```sh
cargo build -p isct-ocr --release --locked --features training-tools \
  --example alphabet_prepare
python scripts/ocr/alphabet_synthetic.py --fonts FONT_DIRECTORY \
  --prepare-binary target/release/examples/alphabet_prepare \
  --lighting --train 26000 --validation 3900 --seed 20261031 --output normalized.npz
python scripts/ocr/alphabet_synthetic.py --fonts FONT_DIRECTORY \
  --prepare-binary target/release/examples/alphabet_prepare \
  --check-model crates/isct-ocr/models/alphabet/alphabet-printed.onnx \
  --output parity.json
python scripts/ocr/alphabet_calibration.py --model CANDIDATE.onnx \
  --data normalized.npz --output CALIBRATION_DIRECTORY
```

The parity probe's 512 public synthetic inputs produced identical Rust/Python
predictions, including 139 rejections, 179 dual views and one confident conflict.
Maximum confidence difference was 0.000001185. Rust tests also check malformed
input lengths and the prepared-input/model boundary. The OCR suite passes 24
tests with four private-input cases ignored; Clippy passes with warnings denied.

The calibration tool fits one positive temperature by minimizing held-out NLL,
following [Guo et al., *On Calibration of Modern Neural Networks*](https://arxiv.org/abs/1706.04599).
It checks exported logits at batch sizes 1, 7 and 10. A candidate must not increase
wrong accepted predictions at either 0.8 or 0.9, lose correct qualified coverage
or lose total correct predictions on the untouched test split. Passing that
synthetic gate still requires both full Rust card replays before promotion.

**Decision: keep the bundled model and existing confidence/voting thresholds.**
The final eight-epoch candidate used 19,181 prepared training views. On the
independent Georgia split (3,188 known labels plus 712 unknown sources), correct
qualified readings increased from 2,141 to 2,153, wrong qualified readings fell
from five to three, and unknown qualified readings fell from 236 to 231.
However, full production replay still lost two T readings at 15% exposure:
70 correct/settled cells became 68. The separate Rust dark-input case also
caught T→I. That candidate passed 23 tests and failed one, so it was rejected.
All three L/Z/I placements stayed 70/70, the other lighting conditions preserved
correct coverage, and small-dim settling improved from 67 to 70; these gains do
not override the dark-input regression.

Temperature fitting on this candidate yielded 0.778945. It increased acceptance
of unknown test sources from 231 to 260 and was rejected too. The original
model's fitted temperature was also rejected. Neither fitting nor adaptation
changes the app in this checkpoint. The retained model passes all 24 OCR tests,
with four private-input cases ignored. The shared-preprocessing refactor preserves
all twelve lighting results and all three boundary results. Its Rust/Python
parity probe passes 512 inputs; the rejected candidate passes the same parity
probe, distinguishing input agreement from recognition quality.

Full records are under `productionInputAdaptation` in
`matrix-lighting-results.json`. Replay timing was collected during concurrent
verification work and is not a controlled performance comparison. Production
Rust adds 38 and removes 14 lines (net +24; no production-code reduction); tests
add 12 lines and none are removed. Training tools, examples, documentation and
measured JSON are counted separately.

Use direct `iqrsh` for the small interactive experiments in this project, not
runq. For eligible intramural/access-card accounts, omit `-g` to use the free
interactive queue; specifying a group incurs charges according to the
[TSUBAME scheduler guide](https://www.t4.cii.isct.ac.jp/docs/all/handbook.ja/jobs/).
Only generated pixels and public model artifacts are transferred. The previously
authorized anonymous physical glyph crops remain local and are not training data.

### Recorded test-card preparation

Since 2026-10-24 card recordings become training data through the camera path
itself, on the local RTX 3090 machine; nothing is uploaded. Earlier datasets
cut every frame with the still-photo detector and needed a person to approve
each grid and list its readable cells; the frames, crops and review rules
below replace that.

A local manifest names each card once, with its seven human-checked rows and
its split, and lists the recordings of each card (see the docstring of
`scripts/ocr/matrix_video.py`). The split belongs to the card: every recording
of a card lands in one split, so validation and test measure cards the model
never saw, as the app does. A video repeated under another name is rejected.

```sh
python scripts/ocr/matrix_video.py export --manifest LOCAL/manifest.json --output LOCAL/exports
python scripts/ocr/matrix_video.py build --manifest LOCAL/manifest.json   --exports LOCAL/exports --output LOCAL/dataset-v1
python scripts/ocr/alphabet_finetune.py --model crates/isct-ocr/models/alphabet/alphabet-printed.onnx   --data LOCAL/dataset-v1/training.npz --replay-data LOCAL/synthetic.npz --replay-fraction 0.5   --augment 0.7 --output LOCAL/run-1
python scripts/ocr/matrix_video.py evaluate --data LOCAL/dataset-v1/evaluation-local.npz   --model LOCAL/run-1/alphabet-printed.onnx --lineage LOCAL/dataset-v1/lineage-local.json   --output LOCAL/run-1/evaluation.json
```

- **Export** runs the Rust `matrix_dataset` example per recording: one
  production tracker over the decoded 4:2:0 frames, and every frame a live
  session would cut (`isct_core::matrix_scan::keyframe_ready`) cut by the
  production `MatrixVision::cut`. It stores the 70 cells per keyframe, Core's
  quality measurements and admission verdict (`matrix_cell_admissible`), and
  the bundled model's reading, plus each cell's edge acuity and Core's
  sharpness verdict on the keyframe. Exports are reused while the video and the
  exporter binary are unchanged.
- **Build** labels each cell from its card's truth by position. A keyframe
  Core rejects as blurred (`matrix_frame_sharp`, median cell edge acuity below
  0.20) is left out of every split, exactly as the app drops it, so the model
  is neither trained nor validated on smears it will never be shown. A
  remaining keyframe is used only if its indexing is verified: confident model readings must match
  the truth at the fitted positions far more often than at any one-cell shift.
  The reading accepts or rejects whole keyframes; it never labels a cell, so
  cells the current model gets wrong keep their true letter. Within a keyframe
  only cells Core would admit as candidates are kept: those are the only cells
  the model sees in the app. Training cells are deduplicated per recording and
  cell (24×24 thumbnail MAE below 2), passed through Rust `alphabet_prepare`,
  and written with lineage, per-split letter counts and a few review mosaics
  per recording (accepted keyframes, and rejected ones). The mosaics are for a
  spot check: a crop that is unreadable yet admitted is label noise the
  validation card will expose, not something a per-frame review must remove.
- **Training** mixes the card data with synthetic print at a fixed share
  (`--replay-fraction`), because a few cards do not contain every letter, and
  perturbs samples on the GPU (`--augment`: small affine jitter, Gaussian or
  motion blur, exposure, contrast and noise) instead of storing reviewed
  augmented copies. Checkpoints are selected on the validation card only; the
  test card is evaluated once, then the candidate goes through the session
  replay below before any promotion.

`matrix_session_replay VIDEO EXPECTED OUT` replays one recording as a fresh
camera session at every second through Core's `MatrixCameraDriver` (`submit`,
the phone path) with a clock that follows the recording, a latest-frame slot
and one OCR consumer, and reports per window whether the session completed,
whether every cell was right, wrong confirmed cells and a completed session
that is wrong. `ISCT_MATRIX_ALPHABET_MODEL` selects candidate weights at build
time. Stage costs are measured on the host; `--geometry-scale` and `--ocr-ms`
model a slower device or fix the model latency for comparisons.

Checkpoint selection now applies the same error/coverage preservation gate to
both real validation and synthetic replay; higher real accuracy cannot override
lost replay coverage or increased qualified/unknown acceptance. The final real
test remains local and is opened only after checkpoint selection.

#### Recorded test-card results

The 59.49-second recording was split into 0–30 s training, 32–41 s validation
and 43–59 s final testing before extraction, with two-second gaps. At two fps,
the native detector returned grids for 40 of 110 frames; many were misaligned.
Reference-assisted tracking recovered 74 frames. Visual review of 25 fixed-time
representative frames confirmed 653 training, 322 validation and 379 test glyphs.
Review admitted 918 raw-crop perturbations and quarantined 388 others. After
Rust preparation, 1,975 real training views were mixed with 19,181 synthetic
views. The real card lacks D/K; synthetic replay retains those classes. A fresh
build from the frozen raw crops and reviews reproduces both NPZ artifacts byte
for byte. Source grouping and all review/provenance artifacts remain local.

One eight-epoch direct `iqrsh` experiment selected epoch seven. Both real and
synthetic validation had to preserve correct qualified coverage, total correct
readings and qualified/unknown error limits. Training took 28.7 seconds after
initialization; the ten-minute interactive allocation ended and was checked
empty. No group option or runq was used. The session spent several minutes in
CPU model/data initialization before the short GPU training phase.

| Measurement | Original | Local candidate |
| --- | ---: | ---: |
| Readable validation crops correct | 208/322 | 294/322 |
| Validation qualified errors, confidence ≥0.8 | 48 | 14 |
| Readable final-test crops correct | 285/379 | 347/379 |
| Final-test qualified errors, confidence ≥0.8 | 37 | 3 |
| Final-test errors at confidence ≥0.9 | 32 | 3 |
| Unknown final-test sources accepted | 18/41 | 18/41 |
| Independent synthetic test correct qualified | 2,141 | 2,153 |
| Independent synthetic test qualified errors | 5 | 3 |
| Independent synthetic unknown accepted | 236/712 | 226/712 |
| Native whole-camera held-out readings correct | 66/70 | 70/70 |
| Native whole-camera stable cells | 47/70 | 63/70 |
| Native whole-camera stable wrong cells | 0 | 0 |

The native replay uses five-fps frames from the untouched 43–59 s segment,
production detection/candidate selection and Core voting, without assisted
geometry. Both models process the same 58 frames and 417 retained candidates,
with 7.4 simulated seconds of capture. Qualified observations rise from 242 to
272; empty readings stay 127, weak readings fall 48→18 and wrong qualified
observations fall 57→49. Some wrong observations therefore remain even though
none become settled errors. Timings came from concurrent verification and do
not establish a speed improvement.

All twelve lighting cases preserve correct readings and zero stable errors;
small-dim stable coverage improves 67→70. All three L/Z/I placements remain
70/70 with zero stable errors. The candidate passes 24 Rust tests (four private
input tests remain ignored) and 512 Rust/Python parity sources. The shared Rust
dataset boundary is covered by the existing capture/order test, including
degenerate, crossed, nonfinite and out-of-bounds corner rejection. Clippy passes
with warnings denied. No test was removed.

Three confident readable-crop errors remain: one each for B, F and T, including
T→I. I's correct qualified crop coverage falls 38→37 while total correct I stays
38/40; L/Z qualified errors fall to zero. Neither model reads any of the four
fully readable held-out single-frame cards perfectly. The 70/70 result comes
from temporal acquisition/voting. This single-card, single-recording holdout
does not establish cross-card accuracy. Native localization is still a separate
failure mode; physical shell check `OCR-CAMPUS-CARD` remains due.

Candidate SHA-256:
`368780a5faa7b465e3c68eb6b946551aa5f707a3f68cdaf57e14b9c8bca6090f`.
It is retained locally with the original model, experiment metadata and local
launch/rollback scripts. The model README requires camera-adapted weights to
stay local, so the bundled synthetic-only model remains
`592d61903de1f04b48079c44be5c247cd2f16bd323b23fb539b2c7392bfc5177`.
`realVideoAdaptation` in the JSON record contains aggregates and hashes, with no
video, answer table, crop images or cell/time provenance.

This checkpoint adds 64 and removes 31 Rust `src` lines, including the
training-tools-only boundary (net +33; no production-code reduction). The
existing test gains 33 lines and no test is removed. Replay examples add 109 and
remove three lines; Python tooling adds 453 and removes 13. Build metadata,
documentation and measured JSON are counted separately.

#### Default promotion, 2026-10-21

At the user's explicit request, the validated video candidate
`368780a5faa7b465e3c68eb6b946551aa5f707a3f68cdaf57e14b9c8bca6090f`
replaces the bundled default model. Confidence and voting thresholds stay the
same. The original weights remain in the local backup and pre-promotion Git
history. The earlier local-only decision above records the experiment checkpoint;
this explicit promotion supersedes it. Physical check `OCR-CAMPUS-CARD` is pending
the user's new App session.

Checkpoint `5b07f96` adds 133 and removes 5 production Rust lines (net +128;
no production-code reduction). Tests add 107 lines; replay examples/scripts add
124 net lines. Documentation, measured JSON and the generated manual table are
counted separately. No test was removed.
