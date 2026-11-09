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
It is the float source: training continues from it and the runtimes' models
are converted from it. No build embeds it; the INT8 export below read one card
cell fewer, which the promotion gate does not allow.

`alphabet-printed-fp16.tflite` is what Android and Windows builds embed, for
LiteRT (see "2026-11-08 LiteRT" below). SHA-256:
`519d41e9f9102e29ae1713f64eee963f07ce35db0e06e7a1d453f71581165189`.

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
slotledger `constraint/matrix-model-promotion-gate`.
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

For a local experimental build, `ISCT_MATRIX_ALPHABET_MODEL=/absolute/model`
selects other weights at compile time, in the format that target's runtime
reads (`.mlmodel` on Apple, `.tflite` on Android and Windows). Camera datasets
and their provenance stay local. The authorized video-adapted candidate is now
the normal build's default; other camera-adapted experiments stay local.

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

## 2026-11-08 INT8 (rejected)

`scripts/ocr/alphabet_quantize.py` quantizes the float source to QDQ INT8:
weights per channel, activations per tensor at the 99.99th percentile,
calibrated on 1000 cells (up to 40 per class) of the camera-path training
split. 10,215,510 bytes become 2,955,225; a ten-cell batch takes 6.5 ms
instead of 10.7 ms in ONNX Runtime on one x86-64 host.

| Held-out third card, 1014 sharp admitted cells | Float | INT8 |
| --- | ---: | ---: |
| Correct | 1006 | 1005 |
| Confident errors (>= 0.8) | 0 | 0 |
| Keyframes with all cells correct | 11 / 15 | 10 / 15 |

On the 3156 validation cells both make 4 confident errors (3117 and 3119
qualified). The 512-input Rust/Python parity check gives a largest
confidence difference of 7e-7; one reading at confidence 0.26 breaks an E/L
near-tie differently between the two ONNX Runtime versions, which the check
allows only below the 0.8 vote threshold. Session replay of four recordings
(24 windows, `docs/verification/matrix-size-speed.md`) completes 23 with no
wrong pass, the same as the float model on the same frame path. It was not
kept: one fewer correct cell fails the promotion gate's no-loss rule.

## 2026-11-08 LiteRT

Android and Windows run the classifier with LiteRT (`engine/letters/litert.rs`).
`scripts/ocr/alphabet_litert.py`, run in WSL because litert-torch has no
Windows build, rebuilds the float source as the native PyTorch module
(`alphabet_model.load`), folds the one-to-three channel expand into the first
convolution (the three channels are equal), converts it with litert-torch
0.10 for batches of one to ten, and stores float16 weights with
ai-edge-quantizer 0.10 (`FLOAT_CASTING`, float32 compute). The input stays
[batch, 1, 96, 96].

Every export is judged by the same `matrix_video.py evaluate` report (a
`.tflite` model runs in the ai-edge-litert 2.3 interpreter with XNNPACK):

| Model | bytes | third card correct | its confident errors | validation confident errors |
| --- | ---: | ---: | ---: | ---: |
| float ONNX source | 10,215,510 | 1006 | 0 | 4 |
| ONNX INT8 (rejected) | 2,955,225 | 1005 | 0 | 4 |
| LiteRT float | 10,238,332 | 1006 | 0 | 4 |
| LiteRT INT8 (`--precision int8`) | 3,088,416 | 1005 | 0 | 5 |
| LiteRT float16 weights (Android, Windows) | 5,208,704 | 1006 | 0 | 4 |

The LiteRT INT8 candidate lifted one N→Y misreading, which every model makes,
above the 0.8 vote threshold; float16 weights read exactly like the float
source for 2.1 MB more, so they were chosen. The runtimes compute in float32
from them. Rounding the weights to bfloat16 instead also reads like the float
source, with nine times the weight error (0.37 % against 0.04 %) and almost
no runtime support; running the whole network in float16 or bfloat16 in
PyTorch reads like the float source too.


## 2026-11-08 Core ML

macOS and iOS run the classifier with the system's Core ML
(`engine/letters/coreml.rs`), so the Apple apps no longer link ONNX Runtime.
`scripts/ocr/alphabet_coreml.py` (macOS, coremltools 9.0, torch 2.7.0)
rebuilds the float source as the native PyTorch module, folds the channel
expand into the first convolution as the LiteRT export does, traces a batch
of ten and converts to an ML Program for iOS 17 / macOS 14 (the Core SDK's
floor) that computes in
float32 from float16 weights: each float32 weight becomes a `constexpr_cast`
of its float16 rounding, expanded once at load, the Core ML form of LiteRT's
FLOAT_CASTING. Every weight is written inline into the single specification
`alphabet-printed-fp16.mlmodel` (5,169,810 bytes, SHA-256
`d8981ca015d6579e959d4d9366f00013354bac243b879925db2f247269e9b843`), because
Core embeds it and loads it from memory: an in-memory ML Program must not
reference a weight blob file. Input `cells` [10, 1, 96, 96] in [-1, 1],
output `logits` [10, 27]: the batch is fixed at ten, Core's batch, because
Core ML failed to resize a flexible one (`ct.RangeDim`) below its default;
Core pads a smaller batch with paper and reads only its rows. Two exports of
the same source are not byte-identical (coremltools' serialization), so an
export is judged by what it reads, never by its digest.

Core ML runs it on the CPU (`CPUOnly`) on every device, about 10 ms for a
ten-cell batch on one M-series Mac, as ONNX Runtime took. Two rejected
alternatives, judged by the same report:

- float16 arithmetic (`ct.precision.FLOAT16`, 5,174,407 bytes): on the Neural
  Engine it read like the float source at 0.7 ms a batch, but on the CPU,
  where Macs without a Neural Engine run it, one correct validation cell fell
  below the 0.8 vote threshold, which the gate's no-loss rule rejects;
- the Neural Engine or GPU for this float32 model: the Neural Engine computes
  only in float16, and iOS refuses GPU work from a background app, where a
  scan's recognition may still be running. Model time does not limit scan
  time (`docs/verification/matrix-size-speed.md`).

`matrix_video.py evaluate` reads a `.mlmodel` through coremltools on the CPU,
as Core does. On the 2026-10-20 video evaluation sets available on the Mac
(`.cache/ocr-video-20261020/prepared-all-frames`: 1190 validation and 1540
test observations, labels by human-checked truth), against the float source:

| Model | validation correct / qualified correct / qualified wrong | test correct / qualified correct / qualified wrong | letters unlike float | cells crossing 0.8 |
| --- | --- | --- | ---: | ---: |
| float ONNX source | 304 / 301 / 15 | 361 / 356 / 3 | — | — |
| LiteRT float16 weights | 304 / 301 / 15 | 361 / 356 / 3 | 0 | 2 |
| Core ML float16 weights (shipped) | 304 / 301 / 15 | 361 / 356 / 3 | 0 | 2 |
| Core ML float16 arithmetic, CPU (rejected) | 304 / **300** / 15 | 361 / 356 / 3 | 3 | 6 |

The cells crossing the vote threshold in the shipped export are ambiguous
ones (no truth); every labelled count is identical to the float source. The
held-out third card of the gate (`dataset-v6`) and the recordings for session
replay are not on the Mac; see `docs/verification/matrix-size-speed.md` for
which gate rows were run where.
