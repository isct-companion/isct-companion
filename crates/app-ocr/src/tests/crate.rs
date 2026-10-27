//! The non-ONNX `MatrixOcrService` stub is one unconditional
//! `Err(MatrixOcrError::Unavailable)`. That boundary claim is asserted where it
//! reaches a caller: the disabled-OCR case in `crates/app-ffi/src/tests/crate.rs`.
