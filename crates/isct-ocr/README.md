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

Desktop live capture uses `scan_frame`, which runs the same pipeline and also
returns the 70 cell quadrilaterals normalized to the oriented image. `scan`
continues to return just the readings for photo imports. Live frames never use
the debug crop export path.

`isct_core::matrix_scan::MatrixScanner` owns live-session identity and consensus,
without depending on OCR or camera libraries. It keeps the last eight frames,
adds each letter's OCR confidence as its vote weight, and marks a cell stable
with at least three supporting frames, mean confidence >= 0.90, winning weight
share >= 0.85, and agreement from the latest frame (confidence >= 0.75). The
reported confidence is mean winning confidence times winning weight share.
Losing the grid or a large geometry displacement clears the history; stopping
or replacing a session invalidates pending results. These thresholds are
initial policy values to validate against physical-card scans.

The desktop adapter owns `getUserMedia`, unmirrored video, JPEG frame capture,
and a single in-flight OCR request. The next fresh frame is sampled 250 ms
after completion. Core snapshots drive the cell boxes, letters, progress and
automatic acceptance. A partial result can be explicitly sent to the existing
editor for review. Closing, backgrounding, camera disconnection or a failed
request stops tracks and ends the session; a late permission grant is also
released. The camera privacy prompt and physical overlay alignment belong to
manual check `OCR-CAMPUS-CARD` in a packaged app.
