# isct-ocr

`isct-ocr` is the optional, UI-independent matrix-card recognition library.
Its default `onnx` feature embeds the Paddle model and is available on targets
supported by the pinned ONNX Runtime release. Intel macOS and
`--no-default-features` builds expose the same small API and return
`MatrixOcrError::Unavailable` without linking image or ONNX dependencies.

Shells prepare the source and preview PNGs, pass an optional debug directory,
and forward the progress callback. The crate owns geometry, preprocessing,
model inference, result acceptance, and typed errors. Invalid input, missing
grids, model failures, and concurrent scans return errors; the same service may
be called again afterward. Debug crop export is best-effort and never changes a
successful recognition result.

This crate isolates an optional dependency and its failure handling at the API
boundary. It runs in the caller's process and does not claim to contain a native
abort or memory fault; process isolation would require a separate worker.
