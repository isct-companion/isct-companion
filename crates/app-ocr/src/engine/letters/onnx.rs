//! ONNX Runtime (statically linked by `ort`) with the float source model.
use super::{normalized, CLASSES, SIZE};
use crate::MatrixOcrError;
use ort::{session::Session, value::Tensor};

fn model_error(error: impl std::fmt::Display) -> MatrixOcrError {
    MatrixOcrError::Model(error.to_string())
}

const MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/matrix-alphabet.onnx"));

pub(in crate::engine) struct Letters(Session);
impl Letters {
    pub(in crate::engine) fn new() -> Result<Self, MatrixOcrError> {
        Ok(Self(
            Session::builder()
                .map_err(model_error)?
                .with_intra_threads(2)
                .map_err(model_error)?
                .with_inter_threads(1)
                .map_err(model_error)?
                .commit_from_memory(MODEL)
                .map_err(model_error)?,
        ))
    }
    /// `CLASSES` logits per image, in order.
    pub(in crate::engine) fn logits(
        &mut self,
        images: &[Vec<u8>],
    ) -> Result<Vec<f32>, MatrixOcrError> {
        let input = Tensor::from_array(([images.len(), 1, SIZE, SIZE], normalized(images)))
            .map_err(model_error)?;
        let output = self.0.run(ort::inputs![input]).map_err(model_error)?;
        let Some((_, output)) = output.into_iter().next() else {
            return Err(MatrixOcrError::Model("Missing alphabet output.".into()));
        };
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(model_error)?;
        if shape.as_ref() != [images.len() as i64, CLASSES as i64] {
            return Err(MatrixOcrError::Model("Invalid alphabet output.".into()));
        }
        Ok(data.to_vec())
    }
}
