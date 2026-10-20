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

Desktop camera scanning has two phases. `capture_matrix_frame` decodes and
locates a frame, rectifies 96×96 glyphs with bilinear interpolation, keeps
Rec.601 grayscale, and measures contrast, Laplacian sharpness, source resolution
and whether texture lies inside the glyph instead of on a cropped grid edge.
It performs no character-model inference and no input binarization. Geometry
localization tries the existing grid/header detector first, then retries with
locally corrected illumination while preserving its label/grid validation.
The label bands are measured separately from the 7×10 body. Internal line
centers calibrate the body bounds before rectification, so a taller/shorter
header or a thick border cannot shift the letter crops. Contour and header
fallback paths use the same validated body coordinates.
Cell quality is measured after bounded local illumination correction so a dim
but readable glyph can start acquisition without lowering the texture gates.
The retained candidate remains the original grayscale image.

`isct_core::matrix_scan::MatrixCameraScanner` owns the bounded acquisition.
The first plausible Matrix (at least half its cells have usable texture) starts
an eight-second deadline. Each cell independently retains up to six candidate
gray images from observations at least 100 ms apart, preferring sharp centered
texture. Clipped highlights combined with uneven paper brightness within a cell
or relative to the card's upper-quartile paper level reject only the affected
cells; uniform white paper is not treated as glare. These rejections
do not suppress the first plausible grid or restart its deadline.
Acquisition stops immediately once every cell has five candidates, or
at the deadline. A periodic shell wakeup enforces the deadline even when video
stalls. Missing geometry never restarts it. Before first detection there is no
capture deadline. Candidate memory is bounded at 70×6×96×96 bytes plus small
metadata; buffers remain resident only for the current session.

The shell releases the camera and closes its dialog before invoking background
processing. `MatrixAlphabetService` uses the bundled A–Z + blank classifier on
batches of up to ten candidate glyphs. Only results at confidence >= 0.8 vote.
Dark/uneven candidates also get an illumination-corrected reading. Confident
disagreement marks the observation for review; both versions together can
contribute only one vote. The gain is bounded, and erased strokes stay unreadable.
Each cell requires at least three qualified observations and a strict weighted
and count majority. Offline votes accumulate across all retained candidates;
a later candidate does not erase earlier votes. The result is returned only
after every candidate is processed. Unsettled cells are marked for review rather
than presented as settled recognition. Closing/cancelling invalidates the session;
workers check ownership between batches and ignore stale results.

Desktop requests a 30 fps stream with one capture request in flight. It consumes
the newest decoded frame, dropping intervening frames without canvas/JPEG work.
The preview displays only one red outline. Processing has inline progress and
can continue while the app is in the background after capture has stopped.
`matrix-scan/latest.json` under the app cache contains acquisition/processing
times, per-cell candidate/vote counts, weak coverage and shell capture timing;
`geometryFrames`, `admissionRejections`, and per-cell `glareRejections` distinguish
missing geometry, a located grid with insufficient texture, and rejected highlights;
it contains no images or recognized letters. `trackedMs` measures first visible
outline to camera release, not the later inference completion.

Desktop debug builds also log per-frame stage timings and min/median/max
source-pixel size, contrast, sharpness and centering, without pixels or letters.
The development profile optimizes OCR and image dependencies so unoptimized
pixel loops do not dominate live tracking.

For an explicitly authorized local diagnosis, launch a debug app with
`ISCT_MATRIX_GLYPH_SAMPLES` pointing to a new directory under an ignored local
cache. Once a frame has 60 usable cells of at least 25 source pixels, it saves
12 shuffled glyphs with random filenames, then stops exporting for that process.
Each `.gray` file is 96×96 unsigned 8-bit grayscale in row-major order. No card
frame, cell coordinates or predictions are saved. The directory must not exist;
Unix permissions are 0700 for the directory and 0600 for each file. Keep these
samples local, out of commits and training uploads. Ordinary launches leave
this export disabled.

Photo imports retain the existing Paddle service and acceptance rules. The
older `scan_live_frame` and `MatrixScanner` API remain for its adaptive replay
profiler. That path scans all 70 cells, refines unsettled cells and verifies the
whole card before completion; its policies are not a second owner of the new
camera acquisition session.

Replay the two-stage camera flow (the input files are `capture-*.jpg`):

```sh
cargo run -p isct-ocr --release --example matrix_capture -- FRAME_DIRECTORY EXPECTED_TEXT CAPTURE_FPS
```

The reported capture duration follows the recording timestamps; it is not a
new live-camera measurement. Frame processing and offline model time are measured
on the host. `ISCT_MATRIX_ALPHABET_MODEL` can select local experimental weights
at build time; ordinary builds embed the synthetic-only printed model documented
under `models/alphabet/`. No captured card data belongs in that directory.

Run a local replay through the real scheduler and model:

```sh
cargo run -p isct-ocr --release --example matrix_profile -- IMAGE_OR_FRAME_DIRECTORY EXPECTED_TEXT [MAX_FRAMES]
```

This reports first-pass time, refinement latency, effective update frequency,
full-card-equivalent throughput, completion time and ground-truth mismatches.
Replay timing excludes camera/IPC overhead; physical focus, framing and live
acquisition remain manual check `OCR-CAMPUS-CARD`. The Latin glyph fixture is
rendered from the system Helvetica font and contains no student data.

## Candidate alphabet classifier evaluation

The reproducible lighting replay and its baseline are documented in
[`docs/verification/matrix-lighting.md`](../../docs/verification/matrix-lighting.md).
`scripts/ocr/matrix_lighting.py` generates public-fixture sequences once and
replays the same files against baseline/candidate `matrix_capture` binaries.
Missed geometry and failed admission remain explicit report rows.
`alphabet_synthetic.py --lighting` adds dim exposure, uneven shadows, gradients,
sensor noise and glare; substantial erased ink is labelled blank. Font families
used for validation are held out of training. These metrics do not replace
the physical-card check.

`matrix_live CAPTURE_DIRECTORY [ALPHABET_ONNX] [TIMEOUT_SECONDS]` can run an
external A–Z + blank classifier against live `latest.jpg` frames. It uses the
production capture preprocessing and legacy core consensus, with direct 96×96 grayscale input.
Unlike production metadata logging, this supervised diagnostic saves source
frames, geometry and readings locally for replay; use a disposable test card.

`scripts/ocr/alphabet_eval.py` evaluates a local rectified card.
`alphabet_motion.py` evaluates the recorded frames.
`alphabet_synthetic.py` generates printed training/validation images with held-out
font families, camera sampling and motion degradation. `alphabet_finetune.py`
continues the specified ONNX weights and exports a dynamic-batch model after
checking conversion/export parity. Synthetic training needs no captured card.
`alphabet_adapt.py` optionally prepares local adaptation data from one labelled
card; keep those data and weights local, and evaluate on a separate motion
recording. Fitting that card does not establish generalization to other cards.

The diagnostic uses bilinear perspective correction and Rec.601 grayscale to
match PIL/TorchVision training pixels, then normalizes to [-1, 1]; it does not
binarize inputs. It evaluates all 70 cells in batches of ten. Core
`MatrixScanner::full_frame()` rejects frames with fewer than 49 qualified cells
to keep false background grids and motion blur out of the vote pool. Accepted
frames still vote independently per cell at confidence >= 0.8. Live adapters
should replace `latest.jpg` atomically at camera speed; the consumer has no
fixed inference frequency or frame queue. This legacy diagnostic does not change the app's embedded model.
