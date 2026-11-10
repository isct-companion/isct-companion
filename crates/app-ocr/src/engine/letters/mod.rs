//! The letter classifier's runtime, chosen when the crate is compiled: Core ML
//! on macOS and iOS, LiteRT on Android and Windows. Each one only turns ten
//! normalized 96×96 cells into 27 logits;
//! glyph preparation, the two-rendering fusion and the softmax stay in
//! `alphabet` for every target.
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod coreml;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(super) use coreml::Letters;

#[cfg(any(target_os = "android", target_os = "windows"))]
mod litert;
#[cfg(any(target_os = "android", target_os = "windows"))]
pub(super) use litert::Letters;

/// Classes: A–Z, then blank.
pub(super) const CLASSES: usize = 27;
/// Side of the square grayscale input.
pub(super) const SIZE: usize = 96;

/// The model's input: Rec.601 grayscale normalized to [-1, 1], row-major.
pub(super) fn normalized(images: &[Vec<u8>]) -> Vec<f32> {
    images
        .iter()
        .flatten()
        .map(|p| *p as f32 / 127.5 - 1.)
        .collect()
}
