# Printed alphabet classifier

Source: https://huggingface.co/hermitkk/alphabet-classifier at
`cf2bb20a6606fa405afb8f0b3df986c22169760f` (model card license: MIT).
Upstream `outputs/exports/alphabet_model.onnx` SHA-256:
`a0c968787561bea263851dcc23252672858d2b8748d2a92c710bcff7d5978688`.

`alphabet-printed.onnx` continues the 2026-10-21 default with generated printed
letters plus explicitly authorized, de-identified glyphs of two test cards,
cut by the production camera path. Promoted as the default at the user's
request on 2026-10-24. SHA-256:
`f9ca9cc5489dab804923bc3446c01e0b6fb6491ad9ddcd4b12731a23b4ce79c5`.

MobileNetV2, 27 logits (A–Z then blank), dynamic batch, input N×1×96×96.
Input is Rec.601 grayscale normalized to [-1, 1]. The camera retains the complete
bilinearly rectified cell; `engine::alphabet` locates ink using a mask, separates
grid edges, then centers and proportionally scales original grayscale pixels
within an 80×76 area on the 96×96 canvas. Ambiguous boundary-connected ink is
rejected explicitly. The mask itself is never the classifier input.
Generation, fine-tuning and ONNX parity checks live under `scripts/ocr/`.
New adaptation datasets use `alphabet_synthetic.py --prepare-binary` with the
Rust `alphabet_prepare` example (`--features training-tools`); Python must not
reimplement glyph preprocessing. `--check-model` compares 512 synthetic inputs
through both Rust recognition and Python ONNX evaluation, including correction
fusion and explicit rejection. `alphabet_calibration.py` fits a temperature on
separate font families and evaluates an untouched test split before card replay.
The earlier 2026-10-20 synthetic adaptation and calibration candidates were
rejected: synthetic scores improved but extreme-dim T/I behavior regressed or
ambiguous acceptance increased. The later video-adapted default keeps the
existing thresholds; see
[the validation record](../../../../docs/verification/matrix-lighting.md).
The original model's held-out font-family synthetic validation was 2600/2600;
this is not a real-card accuracy claim. The upstream handwriting validation is
not a printed-card metric.
The synthetic L/Z/I boundary replay (`scripts/ocr/matrix_letter_boundaries.py`)
must also pass through production acquisition and voting before promotion:
repeated high-confidence mistakes can otherwise appear as settled cells.

## 2026-10-24 default

Trained with `scripts/ocr/alphabet_finetune.py` from the 2026-10-21 default
(`368780a5…`) on TSUBAME's free interactive queue, 12 epochs, epoch 9
selected; all layers trained, half of each epoch drawn from production-
preprocessed synthetic print (held-out font families), GPU augmentation 0.7.
The card data came from `scripts/ocr/matrix_video.py build` over three local
recordings of two test cards: only keyframes Core finds sharp, cells Core
admits, indexing verified, labels from human-checked truth. The last 20% of
each recording (after a two-second gap) selected the checkpoint. A third card
recorded separately was never used for training or selection.

| Third card, 1014 sharp admitted cells | Previous default | This model |
| --- | ---: | ---: |
| Correct | 954 (94.1%) | 1006 (99.2%) |
| Confident errors (>= 0.8) | 18 | 0 |
| Keyframes with all cells correct | 1 / 15 | 11 / 15 |

Session replay of that recording through `MatrixCameraDriver` (blur
rejection, ten-second limit) completed 6 of 6 overlapping windows correctly
in 1.1-2.7 s with no wrong pass; the previous default completed none. On
held-out synthetic Verdana, confident errors fell from 4 to 1. All Rust OCR
tests (L/Z/I boundaries, Latin fixture, Q tail) pass, and 512 synthetic
inputs give identical letters through Rust and Python (max confidence
difference 5e-7). The model is more often confident on synthetic glyphs
quarantined as unreadable (171 to 218 of 2600); Core's blur rejection keeps
such frames from the model in the app. One short recording of one card is
not an independent-card accuracy claim; physical check `OCR-CAMPUS-CARD`
remains. Training data, truth, lineage and the candidate history stay local.

For a local experimental build, `ISCT_MATRIX_ALPHABET_MODEL=/absolute/model.onnx`
selects other weights at compile time. Camera datasets and their provenance
stay local. The authorized video-adapted candidate is now the normal build's
default; other camera-adapted experiments stay local. Runtime is the existing
pinned ONNX Runtime; its license and notices are in `../paddle/`.

The later 2026-10-20 video experiment produced the 2026-10-21 default model,
SHA-256 `368780a5faa7b465e3c68eb6b946551aa5f707a3f68cdaf57e14b9c8bca6090f`.
It used explicitly authorized test-card glyphs plus production-preprocessed
synthetic replay. Earlier login-card crops were excluded. Video, truth, lineage,
reviews and final-test data stayed local; only shuffled single-glyph training
and validation arrays were transferred through the free interactive queue.

On the held-out video segment, readable-crop accuracy improved from 285/379 to
347/379 and qualified errors fell from 37 to three. Production camera replay
improved from 66/70 readings and 47 stable cells to 70/70 and 63 stable cells,
with no stable errors in either run. All 24 Rust tests, twelve lighting replays,
three L/Z/I boundary replays, 512-source parity and the independent synthetic
test passed. Three confident crop errors remain, including one T→I; the native
localizer still misses/misaligns frames. These are same-card holdouts, not
independent-card accuracy. Full metrics and limitations are under
`realVideoAdaptation` in the validation record. The candidate and original-model
backup stay in `.cache/ocr-video-20261020/`; the local `run-candidate.zsh` and
`run-original.zsh` scripts explicitly select the new and original weights for
desktop development. The original synthetic-only model SHA-256 is
`592d61903de1f04b48079c44be5c247cd2f16bd323b23fb539b2c7392bfc5177`;
it can also be recovered from Git before the default-model promotion.
