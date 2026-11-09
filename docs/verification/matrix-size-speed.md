# Matrix scan size and speed (2026-11-08)

Branch `claude/matrix-size-speed` from `main` 2df28781. Goal: a smaller
install and faster scans without a lower accuracy or any more wrong passes
(slotledger `goal/matrix-size-speed`, `approach/matrix-frame-path-speed`).
Every number below was measured on one Windows x86-64 host (release builds,
ONNX Runtime CPU, two intra-op threads). Phones are slower; the ratios, not
the milliseconds, are what carry over.

## Where the time went

Phone path (`MatrixCameraDriver::submit` with a 4:2:0 frame), 400 frames per
recording, median ms:

| Stage | how often | 1080p before | 1080p after | 720×1280 before | 720×1280 after |
| --- | --- | ---: | ---: | ---: | ---: |
| YUV → analysis planes | every frame | 52 (with full luma) | 8.1 | 50 (with full luma) | 13.7 |
| full-resolution luma | keyframes only | (included above) | 9.4 | (included above) | 4.4 |
| grid tracking (tracked / refit) | every frame | 7.9 / 14.0 | unchanged | 12.1 / 19.0 | unchanged |
| keyframe cut and scores | keyframes | 53.6 | 17.6 | 36.3 | 13.3 |
| recognise ten cells | per batch | 23.6 | ≈11 (INT8) | 15.5 | ≈11 (INT8) |

The cut and the full luma run on the frame thread, so they delay the next
frame. A replay that scaled frame cost by 0.5 raised completed windows from
17 to 20 of 24, while fixing recognition at 5 ms per batch changed nothing:
the frame path, not the model, limited scan time.

## What changed

- **Conversion** (`engine/picture.rs`). Analysis planes are resampled from
  the Y and chroma planes (SIMD `fast_image_resize`, triangle filter) and
  converted at 960 px, never as a full-frame RGB image. The full-resolution
  luma is computed only for a keyframe Core asks for (`MatrixVision::
  full_luma`). It stays byte-identical to the per-pixel YUV→RGB→Rec.601
  formula the training exports were cut with: red and blue are tabulated per
  (Y, V) and (Y, U) code pair from that formula, green is evaluated per pixel,
  and rounding uses an exact branch-free `round`
  (`camera_full_luma_matches_the_per_pixel_conversion_exactly`). The tracker's
  planes differ from before by rounding only.
- **Cut** (`engine/alphabet.rs`). The 70 cells are cut and scored on up to
  four threads; every value is unchanged.
- **Refit** (`engine/grid/track.rs`). A keyframe can only come from a fresh
  fit, and the tracker refitted every 8 frames: failing windows had 82–163
  located frames but 2–8 keyframes. It now refits every 4 frames. Every 3
  gave the same completion count at a higher per-frame cost.
- **Model** (`models/alphabet/README.md`). LiteRT builds embed float16
  weights (5.2 MB instead of 10.2 MB) that read exactly like the float source.
  ONNX Runtime keeps the float source: an INT8 export (2.96 MB) read one
  held-out cell fewer, which the model promotion gate rejects. macOS and iOS
  now run float16 weights on Core ML (below). The ONNX Runtime backend left
  with Linux as a product target; the float source stays as the export source.
- **Removed**: the PaddleOCR model, `MatrixOcrService`, Core's
  `MatrixScanner` and their tests (about 1,900 lines). The release linker had
  already dropped the unused Paddle model, so its removal does not shrink the
  binary.

## Session replay

`matrix_session_replay` over four local recordings, six windows each (10 s
limit, Core clock on the recording's timeline, measured host costs). Only the
third card (`final`) was never used for training or checkpoint selection;
the other three recordings are of the two training cards, so their windows
test the pipeline, not the model's generalisation.

| Build | completed and correct | wrong pass | frame ms p50 (final / test3 / test1 / test2) |
| --- | ---: | ---: | --- |
| main (before) | 17 / 24 | 0 | 75 / 54 / 110 / 78 |
| faster conversion and cut | 19 / 24 | 0 | 27 / 26 / 28 / 18 |
| + refit every 4 frames | 23 / 24 | 0 | 28 / 26 / 46 / 18 |
| + INT8 model (rejected) | 23 / 24 | 0 | 28 / 27 / 48 / 18 |

Completion times on the held-out card went from 1.6–2.2 s to 0.5–1.1 s. In
the remaining timeout (test1 from 20 s) the grid is located on 73 of 199
frames and only one keyframe reaches Core: geometry, not recognition, is
what that stretch of the recording lacks.

Size: the replay executable, which links the same OCR stack as the apps,
went from 31.3 MB to 24.6 MB with the INT8 model (`fast_image_resize` adds
about 0.5 MB, the INT8 model saved 7.3 MB of it); with ONNX Runtime the
shipped float source keeps it near 32 MB.

## Android package (arm64 phones)

Release APKs built locally with R8 (`assembleRelease`, unsigned), sizes as
packaged (native libraries are stored uncompressed):

| Step | APK | main contents |
| --- | ---: | --- |
| main: universal APK | 95.1 MB | Core for arm64 (45.9 MB) and x86_64 (18.6 MB), bundled ML Kit barcode library for four ABIs (20.3 MB), libc++ 1.3 MB, dex 5.7 MB |
| arm64 only, Play-services ML Kit, ONNX INT8, one-channel resize | 46.9 MB | Core 38.7 MB, libc++ 1.3 MB, dex 5.5 MB |
| LiteRT instead of ONNX Runtime, packed relocations | **34.6 MB** | Core 21.9 MB (with the 5.2 MB float16 model), LiteRT 2.3.2 5.6 MB, dex 5.5 MB |

- `abiFilters` keeps arm64-v8a only; x86_64 is built for the SDK's CI
  emulator test alone (`build-android.mjs --with-emulator`).
- Screenshot QR decoding uses `play-services-mlkit-barcode-scanning`, whose
  model Play services downloads at install, instead of bundling
  `libbarhopper` (about 5 MB per ABI).
- ONNX Runtime and the libc++ it needs (about 18 MB) are replaced by the
  `libLiteRt.so` of `com.google.ai.edge.litert:litert:2.3.2` (5.6 MB), opened
  by Core through the TFLite C API with the XNNPACK delegate. Phones without
  Google Play services keep OCR. The Core library's only dependencies are
  libc, libm and libdl. LiteRT 1.4.2 (4.5 MB) handed XNNPACK only the 18
  depthwise convolutions of the model's 126 nodes; 2.3 takes the convolutions
  fed by float16 weights too. The Kotlin API 2.3.2 brings is built with a
  newer Kotlin than the app's and is excluded; R8 drops LiteRT's Java classes.
- RELR (`-z pack-relative-relocs`, API 30+) shrinks the 2.3 MB relocation
  table to 26 KB. Identical-code folding saved nothing measurable and is not used.
- `fast_image_resize` is called through its typed one-channel API: the
  untyped call compiled every pixel format in (1.6 MB of code and a 0.5 MB
  table; now 33 KB).

The model choice for LiteRT (float16 weights, not INT8) is in
`crates/app-ocr/models/alphabet/README.md`. On a physical phone the photo
import read the card and a live scan returned to the account page with the
grid filled. (The live scan's completion used to be dropped: the analyzer
stopped the session before the dialog received COMPLETE; the change from
the unmerged PR #42 now lets the dialog deliver it first.) On an API 37 x86_64
emulator, which runs the arm64 app through ARM translation, LiteRT 1.4.2
read the fixture card, but 2.3.2 crashes in a one-time initializer while
creating the interpreter (a null per-core table, as when CPU detection finds
no cores under translation), so the emulator cannot check OCR any more.

## Windows (LiteRT)

Windows now runs the same float16 `.tflite` model as Android on LiteRT:
Google's prebuilt `libLiteRt.dll` 2.3.1 (12.2 MB, C runtime linked
statically, so no Visual C++ redistributable), fetched by
`crates/app-ocr/build.rs` against a pinned SHA-256 and installed beside the
executable (`tauri.windows.conf.json`). The TFLite C API does not add
XNNPACK by itself, unlike the Java and Python interpreters; without it a
ten-cell batch took 92-146 ms, with it 12 ms. Android gets the same delegate.

| Release build | executable | beside it | total |
| --- | ---: | ---: | ---: |
| 1.1.3 (October) | 70.5 MB | | 70.5 MB |
| this branch with ONNX Runtime (INT8) | 50.1 MB | | 50.1 MB |
| this branch with LiteRT (float16) | 32.6 MB | 12.2 MB | 44.8 MB |

ONNX Runtime took about 19.7 MB of the executable. Session replay of the four
recordings (24 windows) on an idle machine: LiteRT 23 completed with no wrong
pass, ONNX Runtime INT8 also 23; batch p50 12.5 ms against 11 ms. (A run with
other builds competing for the CPU completed 21: replay charges measured host
time.)

## Not covered

Physical scanning (`OCR-CAMPUS-CARD`), phone CPU timings and independent
cards beyond the third. The recognition backend per platform is slotledger
`approach/matrix-inference-backends`.

The NSIS installer from the `desktop-release` workflow (run 42470220248,
unsigned, built from this branch) places `libLiteRt.dll` at the install root
beside `isct-companion.exe`, with the pinned SHA-256, and carries the LiteRT
licence and README. Against the published 1.3.1 installer:

| Windows installer | download | installed exe | beside it | installed total |
| --- | ---: | ---: | ---: | ---: |
| 1.3.1 (release) | 31.4 MB | 66.4 MB | | 66.4 MB |
| this branch (CI) | 24.6 MB | 40.9 MB | 12.2 MB | 53.0 MB |

The macOS builds of the same run (ONNX Runtime with the float source)
compile and bundle.

## macOS and iOS (Core ML, 2026-11-09)

macOS and iOS, Apple silicon and Intel, run the letter model on the system's
Core ML (`engine/letters/coreml.rs`), so the Apple builds link no inference
library: ONNX Runtime and its static library leave them, and Intel Macs,
which ONNX Runtime had no binary for, read the card for the first time. The
model is `alphabet-printed-fp16.mlmodel` (5.2 MB): float16 weights expanded
to float32 at load, float32 arithmetic on the CPU, weights inline so Core
loads it from memory, a fixed batch of ten. Why each of those, and the
rejected float16-arithmetic export, are in
`crates/app-ocr/models/alphabet/README.md`. Measured on one M-series Mac.

Size, release builds on the same host from dev `9c2c9b9a` (ONNX Runtime) and
this change:

| Release build (aarch64-apple-darwin) | ONNX Runtime | Core ML |
| --- | ---: | ---: |
| `matrix_capture` replay (the OCR stack alone) | 27.6 MB | 6.3 MB |
| desktop executable `isct-companion` | 51.8 MB | 30.5 MB |

A ten-cell batch takes about 10 ms on the CPU, as ONNX Runtime took; model
time does not limit scan time (above).

Gate rows run on the Mac, Core ML against the ONNX Runtime build of
`9c2c9b9a` (float source), both `matrix_capture --release`:

| Check | ONNX Runtime | Core ML |
| --- | --- | --- |
| 2026-10-20 video evaluation, 1190 + 1540 observations | 304 / 361 correct, 15 / 3 qualified wrong | identical letters, identical labelled counts |
| Lighting replay, 12 sequences | per sequence as above (glare stripe 63, its four wrong cells) | identical recognition, cell for cell |
| L/Z/I boundary replay, 3 offsets (Times New Roman Italic) | 70 / 70 each, none wrong | 70 / 70 each, none wrong |
| `app-ocr` tests, aarch64 and x86_64 (Rosetta) | pass | pass |

The iOS SDK (device and arm64 simulator XCFramework, `app-ffi --features
ocr` tests) and the iOS app (`xcodebuild`, generic iOS device, unsigned)
build against it; the SDK's static library has no C++ runtime reference left,
so `bindings/swift/Package.swift` no longer links libc++. The simulator smoke
(`ios-smoke.mjs`, real `prepareScanner()` on Core ML) did not run: the Mac had
no iPhone simulator device.

Not run on the Mac, because their data stays on the Windows host: the
held-out third card (`dataset-v6`, 1014 cells) and the session replay of the
four recordings. Run them there before a release
(`matrix_video.py evaluate --model crates/app-ocr/models/alphabet/alphabet-printed-fp16.mlmodel`
needs macOS for coremltools, so the third card's cells are evaluated on a Mac
or the Core ML replay binary is run on one).

## One luma cut path (2026-11-09)

`capture_matrix_frame`, the path of the `matrix_capture` replay, used to cut RGB
cells and convert each to luma. It now cuts from the Rec.601 luma plane like a
camera keyframe, so the two are not byte-identical; the model and the live
keyframe and `scan_photo` paths did not change. Release builds on one Mac, before
(dev `a49e521d`) and after, over the same generated frames:

| Check | before | after |
| --- | --- | --- |
| Lighting replay, 12 sequences | per sequence as below | same correct count and wrong cells in each |
| L/Z/I boundary replay, 3 offsets (Times New Roman Italic) | 70 / 70 each, none wrong | 70 / 70 each, none wrong |

Glare stripe has the same seven wrong cells (30-33, 35-37) in both. How many of
them settle depends on replay timing: of four runs, the old binary settled
33, 35, 36, 37 twice and 33, 35, 37 twice, the new one always the four. Not run:
the held-out third card and the session replay, whose data stays on the Windows host.

## Model promotion gate

The float16 LiteRT model is a new default on Android and Windows, so it
passed the gate in `crates/app-ocr/models/alphabet/README.md` against
`origin/main` (`0d492efd`, ONNX Runtime with the float source), both built as
`matrix_capture --release` on the same Windows host and run over the same
generated frames:

| Check | main | this branch |
| --- | --- | --- |
| Held-out card correct / confident errors | 1006 / 0 | 1006 / 0 |
| Validation confident errors | 4 | 4 |
| Lighting replay, 12 sequences (`matrix_lighting.py`) | per-sequence counts below | identical, cell for cell |
| L/Z/I boundary replay, 3 offsets (Times Italic) | 70 / 70 each, none wrong | 70 / 70 each, none wrong |
| Session replay, 24 windows | 17 completed, none wrong | 23 completed, none wrong |

Lighting, correct of 70: clean, dim, very dim, small dim, moving glare, blur
and tilted 70; static glare 62; glare stripe 63; shadow, dim-rotated and
blank find no grid. Both builds settle the same four wrong cells (33, 35, 36,
37) in the glare-stripe sequence, which an earlier record of this replay had
at zero. That is a regression already on `main`, not one of this change.
