# Printed alphabet classifier

Source: https://huggingface.co/hermitkk/alphabet-classifier at
`cf2bb20a6606fa405afb8f0b3df986c22169760f` (model card license: MIT).
Upstream `outputs/exports/alphabet_model.onnx` SHA-256:
`a0c968787561bea263851dcc23252672858d2b8748d2a92c710bcff7d5978688`.

`alphabet-printed.onnx` is adapted on generated printed letters only. No camera
images or student data were used for this bundled model. SHA-256:
`592d61903de1f04b48079c44be5c247cd2f16bd323b23fb539b2c7392bfc5177`.

MobileNetV2, 27 logits (A–Z then blank), dynamic batch, input N×1×96×96.
Input is bilinearly rectified Rec.601 grayscale normalized to [-1, 1].
Generation, fine-tuning and ONNX parity checks live under `scripts/ocr/`.
Held-out font-family synthetic validation was 2600/2600; this is not a real-card
accuracy claim. The upstream handwriting validation is not a printed-card metric.

For a local experimental build, `ISCT_MATRIX_ALPHABET_MODEL=/absolute/model.onnx`
selects other weights at compile time. Keep camera-adapted models local. The
normal build always uses this synthetic-only model. Runtime is the existing
pinned ONNX Runtime; its license and notices are in `../paddle/`.
