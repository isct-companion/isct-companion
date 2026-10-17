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

Desktop live capture uses `scan_frame_with_geometry`: the same OCR pipeline
publishes normalized geometry immediately after locating the grid, before the
70 model inferences. `scan` and `scan_frame` retain the photo-import API and
acceptance rules. Live frames do not export debug crops.

`isct_core::matrix_scan::MatrixScanner` owns session identity and per-cell
consensus without depending on camera or model libraries. Each of the 70 cells
retains its own last six high-confidence readings (>= 0.90). Unreadable and
lower-confidence observations do not occupy a slot, dilute existing votes, or
veto a result. For example, A3 can vote over images 1/4/5 while B2 votes over
2/4/5/6. Each retained observation contributes its OCR confidence to its letter's
vote. Acceptance needs at least three qualified samples per cell, a strict
count majority and more than half the vote weight; a tie keeps scanning.

A cell's reported confidence is its winning mean OCR confidence multiplied by
its weighted vote share. Whole-card confidence is the weakest cell's score;
automatic completion requires every cell's sample quorum and majority. Moving
or approaching the camera retains row/column identity. Brief grid loss hides
the outline but retains votes; four consecutive missing frames reset the card.
Stopping or replacing a session invalidates all pending callbacks and votes.

Desktop owns `getUserMedia`, unmirrored video, JPEG frame capture and a single
in-flight OCR request, sampling the next fresh frame 250 ms after completion.
The preview renders one red Matrix boundary, with no cell grid, letters,
confidence counters or partial-result action. Completion stops the camera,
fills the editor and closes the scanner automatically. Closing, backgrounding,
disconnection and errors release tracks; late permission grants are released
too. Physical framing, focus, acquisition time and OS permission prompts are
covered by manual check `OCR-CAMPUS-CARD` in a packaged app.
