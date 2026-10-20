# Printed alphabet classifier

Source: https://huggingface.co/hermitkk/alphabet-classifier at
`cf2bb20a6606fa405afb8f0b3df986c22169760f` (model card license: MIT).
Upstream `outputs/exports/alphabet_model.onnx` SHA-256:
`a0c968787561bea263851dcc23252672858d2b8748d2a92c710bcff7d5978688`.

`alphabet-printed.onnx` is adapted on generated printed letters only. No camera
images or student data were used for this bundled model. SHA-256:
`592d61903de1f04b48079c44be5c247cd2f16bd323b23fb539b2c7392bfc5177`.

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
The 2026-10-20 adaptation and calibration candidates were rejected: synthetic
scores improved but extreme-dim T/I behavior regressed or ambiguous acceptance
increased. The bundled weights and thresholds remain unchanged; see
[the validation record](../../../../docs/verification/matrix-lighting.md).
Held-out font-family synthetic validation was 2600/2600; this is not a real-card
accuracy claim. The upstream handwriting validation is not a printed-card metric.
The synthetic L/Z/I boundary replay (`scripts/ocr/matrix_letter_boundaries.py`)
must also pass through production acquisition and voting before promotion:
repeated high-confidence mistakes can otherwise appear as settled cells.

For a local experimental build, `ISCT_MATRIX_ALPHABET_MODEL=/absolute/model.onnx`
selects other weights at compile time. Keep camera-adapted models local. The
normal build always uses this synthetic-only model. Runtime is the existing
pinned ONNX Runtime; its license and notices are in `../paddle/`.
