# Printed alphabet classifier

Source: https://huggingface.co/hermitkk/alphabet-classifier at
`cf2bb20a6606fa405afb8f0b3df986c22169760f` (model card license: MIT).
Upstream `outputs/exports/alphabet_model.onnx` SHA-256:
`a0c968787561bea263851dcc23252672858d2b8748d2a92c710bcff7d5978688`.

`alphabet-printed.onnx` is adapted on generated printed letters plus explicitly
authorized, de-identified test-card glyphs from the 2026-10-20 video experiment.
Earlier login-card crops were excluded. Promoted as the default at the user's
request on 2026-10-21. SHA-256:
`368780a5faa7b465e3c68eb6b946551aa5f707a3f68cdaf57e14b9c8bca6090f`.

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

For a local experimental build, `ISCT_MATRIX_ALPHABET_MODEL=/absolute/model.onnx`
selects other weights at compile time. Camera datasets and their provenance
stay local. The authorized video-adapted candidate is now the normal build's
default; other camera-adapted experiments stay local. Runtime is the existing
pinned ONNX Runtime; its license and notices are in `../paddle/`.

The later 2026-10-20 video experiment produced the validated default model,
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
