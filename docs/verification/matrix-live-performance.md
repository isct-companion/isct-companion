# Desktop Matrix OCR performance

2026-10-17, local release build, six public synthetic 1600 × 1100 card frames
with slight movement, brightness changes and blur. Uses the real OCR engine and
core scheduler, with the desktop 200 ms start-to-start budget. Camera encoding
and IPC are excluded; physical-card results require `OCR-CAMPUS-CARD`.

| Measurement | Result |
| --- | ---: |
| Completed / incorrect cells | 70 / 0 |
| Completion | 5.87 s |
| Updates / cells analyzed | 17 / 280 |
| Overall update frequency | 2.90 fps |
| Initial full-card pass | 1193 ms |
| Refinement mean / P95 | 221 / 359 ms |
| Final full-card verification | 944 ms |
| Full-card-equivalent throughput | 0.68 fps |

These are partial-update frequencies, not 70 fresh readings per frame. The
initial and final full-card passes are slower than ten-cell refinement. Every
cell independently collected four confidence-qualified observations.

The Latin fixture includes thin `I` strokes. A 64-pixel-only crop left one cell
below 0.8; selective 128-pixel recropping recovered it without changing the
threshold or manufacturing confidence.

These numbers came from the `matrix_profile` example of the legacy live
scheduler, removed once camera sessions moved to `MatrixCameraDriver`; replay
sessions now with `matrix_session_replay` (see `crates/isct-ocr/README.md`).
Live-session timing and sample counts are saved to the app cache at
`matrix-scan/latest.json`, without images or recognized characters.
