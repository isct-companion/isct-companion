//! Core ML, the system's own runtime on macOS and iOS, so the apps link no
//! inference library of their own. The model is the ML Program export
//! (`scripts/ocr/alphabet_coreml.py`): float16 weights expanded to float32 when
//! it loads, float32 arithmetic, and every weight inline in its one
//! specification file: it is embedded in the binary and loaded from memory
//! (`MLModelAsset(specification:)`), so there is no model file to install,
//! find or compile at run time. It runs on the CPU on every device: the
//! Neural Engine computes only in float16, which read like the float source
//! there but, on the CPU of Macs without one, cost a correct cell; and iOS
//! refuses GPU work from an app in the background, where a scan's recognition
//! may still be running. A ten-cell batch takes about 10 ms.
use super::{normalized, CLASSES, SIZE};
use crate::MatrixOcrError;
use block2::RcBlock;
use objc2::{rc::Retained, runtime::ProtocolObject, AnyThread};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLFeatureValue, MLModel,
    MLModelAsset, MLModelConfiguration, MLMultiArray, MLMultiArrayDataType,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSNumber, NSString};
use std::{cell::RefCell, ptr::NonNull, sync::mpsc};

const MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/matrix-alphabet.mlmodel"));
/// The export's input and output names.
const INPUT: &str = "cells";
const OUTPUT: &str = "logits";
/// The export's fixed batch, Core's batch: the Neural Engine failed to resize
/// a flexible one. A smaller batch is padded with paper and only its own rows
/// are read; inference keeps the rows independent.
const BATCH: usize = 10;

fn failed(what: &str, error: &NSError) -> MatrixOcrError {
    MatrixOcrError::Model(format!(
        "Core ML {what} failed: {}",
        error.localizedDescription()
    ))
}

pub(in crate::engine) struct Letters(Retained<MLModel>);
// SAFETY: an MLModel may be used from any thread. `MatrixAlphabetService`
// keeps `Letters` behind a mutex, so it is never shared, only moved between
// the camera consumer's threads, and each prediction runs to completion
// before the mutex is released.
unsafe impl Send for Letters {}

impl Letters {
    pub(in crate::engine) fn new() -> Result<Self, MatrixOcrError> {
        objc2::rc::autoreleasepool(|_| {
            let specification = NSData::with_bytes(MODEL);
            // SAFETY: the data is a complete ML Program specification with no
            // blob file reference (checked when it was exported).
            let asset =
                unsafe { MLModelAsset::modelAssetWithSpecificationData_error(&specification) }
                    .map_err(|e| failed("model", &e))?;
            // SAFETY: a new configuration, set before it is handed to Core ML.
            let configuration = unsafe {
                let configuration = MLModelConfiguration::new();
                configuration.setComputeUnits(MLComputeUnits::CPUOnly);
                configuration
            };
            // Core ML loads an asset only asynchronously. The caller already
            // runs on a worker thread, and the handler is always called once,
            // with a model or with an error.
            let (sender, receiver) = mpsc::sync_channel(1);
            let handler = RcBlock::new(move |model: *mut MLModel, error: *mut NSError| {
                // SAFETY: Core ML passes valid (or nil) objects it owns for the
                // duration of the call; retaining them keeps them past it.
                let result = match unsafe { (Retained::retain(model), Retained::retain(error)) } {
                    (Some(model), _) => Ok(model),
                    (None, Some(error)) => Err(failed("loading", &error)),
                    (None, None) => Err(MatrixOcrError::Model("Core ML returned no model.".into())),
                };
                let _ = sender.send(result);
            });
            // SAFETY: the asset and configuration stay alive for the call, and
            // the handler is 'static.
            unsafe {
                MLModel::loadModelAsset_configuration_completionHandler(
                    &asset,
                    &configuration,
                    &handler,
                )
            };
            receiver
                .recv()
                .map_err(|_| MatrixOcrError::Model("Core ML dropped the model load.".into()))?
                .map(Self)
        })
    }

    /// `CLASSES` logits per image, in order.
    pub(in crate::engine) fn logits(
        &mut self,
        images: &[Vec<u8>],
    ) -> Result<Vec<f32>, MatrixOcrError> {
        // Core ML autoreleases its intermediate objects; the camera consumer's
        // threads have no pool of their own to drain them.
        if images.is_empty() || images.len() > BATCH {
            return Err(MatrixOcrError::InvalidImage(
                "Invalid grayscale candidate batch.".into(),
            ));
        }
        let mut cells = normalized(images);
        cells.resize(BATCH * SIZE * SIZE, 1.);
        objc2::rc::autoreleasepool(|_| {
            let input = multi_array(&cells)?;
            let features = NSDictionary::from_retained_objects(
                &[&*NSString::from_str(INPUT)],
                &[Retained::into_super(Retained::into_super(
                    // SAFETY: wraps the array just built.
                    unsafe { MLFeatureValue::featureValueWithMultiArray(&input) },
                ))],
            );
            // SAFETY: the dictionary maps the model's one input name to a feature value.
            let provider = unsafe {
                MLDictionaryFeatureProvider::initWithDictionary_error(
                    MLDictionaryFeatureProvider::alloc(),
                    &features,
                )
            }
            .map_err(|e| failed("input", &e))?;
            // SAFETY: the provider holds every input the model declares.
            let output = unsafe {
                self.0
                    .predictionFromFeatures_error(ProtocolObject::from_ref(&*provider))
            }
            .map_err(|e| failed("prediction", &e))?;
            // SAFETY: plain reads of the returned feature provider.
            let logits = unsafe { output.featureValueForName(&NSString::from_str(OUTPUT)) }
                .and_then(|value| unsafe { value.multiArrayValue() })
                .ok_or_else(|| MatrixOcrError::Model("Missing alphabet output.".into()))?;
            read_logits(&logits, images.len())
        })
    }
}

fn numbers(values: &[usize]) -> Retained<NSArray<NSNumber>> {
    NSArray::from_retained_slice(
        &values
            .iter()
            .map(|&v| NSNumber::new_usize(v))
            .collect::<Vec<_>>(),
    )
}

/// A float32 [`BATCH`, 1, 96, 96] array holding `values`.
fn multi_array(values: &[f32]) -> Result<Retained<MLMultiArray>, MatrixOcrError> {
    let shape = [BATCH, 1, SIZE, SIZE];
    // SAFETY: a valid shape of a supported data type.
    let array = unsafe {
        MLMultiArray::initWithShape_dataType_error(
            MLMultiArray::alloc(),
            &numbers(&shape),
            MLMultiArrayDataType::Float32,
        )
    }
    .map_err(|e| failed("input", &e))?;
    let written = RefCell::new(false);
    let handler = block2::StackBlock::new(
        |bytes: NonNull<std::ffi::c_void>, size: isize, strides: NonNull<NSArray<NSNumber>>| {
            // SAFETY: Core ML passes the array's own strides for the call.
            let strides = unsafe { strides.as_ref() };
            // A new array is dense and row-major; anything else would need
            // its strides honoured, which this export never asks for.
            let dense = strides.len() == 4
                && (0..4).all(|i| {
                    strides.objectAtIndex(i).as_usize() == shape[i + 1..].iter().product::<usize>()
                });
            if !dense || size < 0 || (size as usize) < std::mem::size_of_val(values) {
                return;
            }
            // SAFETY: the buffer is writable for `size` bytes during the call,
            // and holds at least `values.len()` float32 elements.
            unsafe {
                std::ptr::copy_nonoverlapping(values.as_ptr(), bytes.as_ptr().cast(), values.len())
            };
            *written.borrow_mut() = true;
        },
    );
    // SAFETY: the handler runs synchronously, before this call returns.
    unsafe { array.getMutableBytesWithHandler(&handler) };
    written
        .into_inner()
        .then_some(array)
        .ok_or_else(|| MatrixOcrError::Model("Core ML input was not written.".into()))
}

/// The first `batch` rows of the [`BATCH`, `CLASSES`] float32 output,
/// honouring its strides.
fn read_logits(array: &MLMultiArray, batch: usize) -> Result<Vec<f32>, MatrixOcrError> {
    let invalid = || MatrixOcrError::Model("Invalid alphabet output.".into());
    // SAFETY: plain reads of the array's description.
    let (data_type, shape, strides) = unsafe { (array.dataType(), array.shape(), array.strides()) };
    let shape: Vec<usize> = shape.iter().map(|n| n.as_usize()).collect();
    let strides: Vec<usize> = strides.iter().map(|n| n.as_usize()).collect();
    if data_type != MLMultiArrayDataType::Float32 || shape != [BATCH, CLASSES] || strides.len() != 2
    {
        return Err(invalid());
    }
    let last = batch.checked_sub(1).ok_or_else(invalid)? * strides[0] + (CLASSES - 1) * strides[1];
    let logits = RefCell::new(None);
    let handler = block2::StackBlock::new(|bytes: NonNull<std::ffi::c_void>, size: isize| {
        if size < 0 || (size as usize) < (last + 1) * std::mem::size_of::<f32>() {
            return;
        }
        // SAFETY: the buffer is readable for `size` bytes during the call, and
        // every offset read below is at most `last`.
        let data = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<f32>(), last + 1) };
        *logits.borrow_mut() = Some(
            (0..batch)
                .flat_map(|i| (0..CLASSES).map(move |j| (i, j)))
                .map(|(i, j)| data[i * strides[0] + j * strides[1]])
                .collect(),
        );
    });
    // SAFETY: the handler runs synchronously, before this call returns.
    unsafe { array.getBytesWithHandler(&handler) };
    logits.into_inner().ok_or_else(invalid)
}
