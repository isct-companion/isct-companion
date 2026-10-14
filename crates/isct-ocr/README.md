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
