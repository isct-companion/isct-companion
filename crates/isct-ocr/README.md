# isct-ocr

`isct-ocr` is the optional, UI-independent matrix-card recognition library.
Its default `onnx` feature embeds the Paddle model and is available on targets
supported by the pinned ONNX Runtime release. Intel macOS and
`--no-default-features` builds expose the same small API and return
`MatrixOcrError::Unavailable` without linking image or ONNX dependencies.

`MatrixOcrService::scan` accepts one encoded image as raw bytes. The crate
validates the 20 MiB and 40-megapixel bounds, applies file orientation and
alpha compositing, then creates the 2200 px source and 600 px geometry image.
It owns geometry, preprocessing, model inference and result acceptance. Shells
only provide codecs unavailable to the crate (currently HEIC to JPEG), run the
synchronous call on a blocking worker, pass an optional debug directory and
forward `OcrProgress { completed, total }`. Returning `false` from the progress
callback cancels with `MatrixOcrError::Cancelled`.

Invalid input, missing grids, model failures, cancellation and concurrent scans
return typed errors; the same service may be called again afterward. Unsupported
image formats are `InvalidImage`, while builds without the model backend return
`Unavailable`. Debug crop export is best-effort and never changes a successful
recognition result.

This crate isolates an optional dependency and its failure handling at the API
boundary. It runs in the caller's process and does not claim to contain a native
abort or memory fault; process isolation would require a separate worker.

Desktop live capture uses `scan_live_frame`: a frame is decoded and located once,
the outer geometry is published immediately, and only the requested cells are
cropped and preprocessed. Model inference uses batches of up to ten, with a
64-pixel single-character input. Low-confidence narrow results retry the
established 320-pixel input in a batch. Original/cleaned-image review remains
active. Cells still below 0.8 get a 128-pixel recrop to preserve thin strokes,
then keep the better measured result. Photo imports retain their existing 320-pixel API and acceptance rules.

`isct_core::matrix_scan::MatrixScanner` owns scheduling and per-cell consensus.
The first plan contains all 70 cells; subsequent plans contain up to ten
unsettled cells, ordered by oldest attempt so difficult cells cannot starve the
others. Settled cells get bounded background checks. Each cell independently
retains its last six qualified observations (confidence >= 0.80), requiring at
least three and a strict count/weight majority. Low-confidence samples do not
consume slots. A new clear contradiction reopens a settled cell.

When all cells settle, a full-card verification pass checks for contradictions.
At least 49 cells must be readable in that pass, but blurred cells retain their
independently collected votes. The scanner completes only after this check.
Movement preserves row/column identity; four consecutive missing-grid frames
reset the card. Stale/modified plans are rejected, so a frame cannot donate
votes twice. Cancellation invalidates pending inference and geometry callbacks.

Desktop captures only the latest frame with one request in flight. Its 5 Hz
budget is measured start-to-start instead of adding a fixed delay after work.
The preview displays one red outline. Completion releases the camera, fills the
editor and closes automatically. `matrix-scan/latest.json` under the app cache
contains timing, qualified-sample counts, weak/blank/conflict counts and shell
capture timing; it never contains recognized letters or images. Capture
`trackedMs` measures first visible outline to stop, while `elapsedMs` includes
all camera acquisition time after video playback starts.

Run a local replay through the real scheduler and model:

```sh
cargo run -p isct-ocr --release --example matrix_profile -- IMAGE_OR_FRAME_DIRECTORY EXPECTED_TEXT [MAX_FRAMES]
```

This reports first-pass time, refinement latency, effective update frequency,
full-card-equivalent throughput, completion time and ground-truth mismatches.
Replay timing excludes camera/IPC overhead; physical focus, framing and live
acquisition remain manual check `OCR-CAMPUS-CARD`. The Latin glyph fixture is
rendered from the system Helvetica font and contains no student data.
