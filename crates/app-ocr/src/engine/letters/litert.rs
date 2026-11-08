//! LiteRT (TensorFlow Lite) through its C API. On Android the library is the
//! `libLiteRt.so` the app packages with `com.google.ai.edge.litert:litert`;
//! on Windows it is Google's prebuilt
//! `libLiteRt.dll`, which `build.rs` fetches and the installer places beside
//! the executable. Both export the C API and need nothing beyond the system's
//! own libraries, so no Google Play services and no Kotlin bridge are
//! involved. The library is opened at run time: a build without it reports a
//! model error instead of failing to load the whole program. The Rust TFLite
//! crates build TensorFlow from source, so the C functions used here are
//! declared directly.
use super::{normalized, CLASSES, SIZE};
use crate::MatrixOcrError;
use std::ffi::c_void;

const MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/matrix-alphabet.tflite"));
/// LiteRT 2.x ships `libLiteRt.so`, 1.x `libtensorflowlite_jni.so`; both
/// export the same C API, so the app's Gradle dependency alone picks one.
#[cfg(target_os = "android")]
const LIBRARIES: &[&str] = &["libLiteRt.so", "libtensorflowlite_jni.so"];
#[cfg(target_os = "windows")]
const LIBRARIES: &[&str] = &["libLiteRt.dll"];
/// `kTfLiteFloat32` and `kTfLiteInt8` in `TfLiteType`.
const FLOAT32: i32 = 1;
const INT8: i32 = 9;

/// `TfLiteQuantizationParams`: real = scale × (quantized − zero point).
#[repr(C)]
#[derive(Clone, Copy)]
struct Quantization {
    scale: f32,
    zero_point: i32,
}

type Handle = *mut c_void;

/// `TfLiteXNNPackDelegateOptions`, whose fields after `num_threads` change
/// between LiteRT versions. It is only ever filled by the library's own
/// `TfLiteXNNPackDelegateOptionsDefault` and read by its own
/// `TfLiteXNNPackDelegateCreate`; Core sets the first field, which every
/// version starts with. The structure is larger than 16 bytes in every
/// version, so it is returned through a caller buffer on Windows x64 and
/// AArch64, and this buffer is larger than any version needs.
#[repr(C, align(8))]
struct XnnpackOptions {
    num_threads: i32,
    rest: [u8; 252],
}

/// The C API functions, copied out of the library while it stays loaded.
struct Api {
    model_create: unsafe extern "C" fn(*const c_void, usize) -> Handle,
    model_delete: unsafe extern "C" fn(Handle),
    options_create: unsafe extern "C" fn() -> Handle,
    options_delete: unsafe extern "C" fn(Handle),
    options_threads: unsafe extern "C" fn(Handle, i32),
    interpreter_create: unsafe extern "C" fn(Handle, Handle) -> Handle,
    interpreter_delete: unsafe extern "C" fn(Handle),
    resize_input: unsafe extern "C" fn(Handle, i32, *const i32, i32) -> i32,
    allocate: unsafe extern "C" fn(Handle) -> i32,
    input: unsafe extern "C" fn(Handle, i32) -> Handle,
    output: unsafe extern "C" fn(Handle, i32) -> Handle,
    invoke: unsafe extern "C" fn(Handle) -> i32,
    tensor_type: unsafe extern "C" fn(Handle) -> i32,
    quantization: unsafe extern "C" fn(Handle) -> Quantization,
    num_dims: unsafe extern "C" fn(Handle) -> i32,
    dim: unsafe extern "C" fn(Handle, i32) -> i32,
    byte_size: unsafe extern "C" fn(Handle) -> usize,
    copy_in: unsafe extern "C" fn(Handle, *const c_void, usize) -> i32,
    copy_out: unsafe extern "C" fn(Handle, *mut c_void, usize) -> i32,
    xnnpack_defaults: unsafe extern "C" fn() -> XnnpackOptions,
    xnnpack_create: unsafe extern "C" fn(*const XnnpackOptions) -> Handle,
    xnnpack_delete: unsafe extern "C" fn(Handle),
    add_delegate: unsafe extern "C" fn(Handle, Handle),
}

fn failed(what: &str) -> MatrixOcrError {
    MatrixOcrError::Model(format!("LiteRT {what} failed."))
}

pub(in crate::engine) struct Letters {
    api: Api,
    model: Handle,
    /// XNNPACK, LiteRT's optimized CPU kernels. The Java and Python
    /// interpreters add it themselves; the C API leaves it to the caller, and
    /// without it a ten-cell batch took about ten times longer.
    xnnpack: Handle,
    interpreter: Handle,
    /// Batch size the input tensor is currently allocated for.
    batch: usize,
    /// The exported input is [batch, 1, 96, 96] (PyTorch's NCHW, kept by
    /// litert-torch) rather than [batch, 96, 96, 1]. With one channel both
    /// hold the same bytes; only the shape passed to a resize differs.
    channels_first: bool,
    // Declared last: dropped after `Drop::drop` has released the handles.
    _library: libloading::Library,
}
// SAFETY: a TFLite interpreter may be used from any thread, one at a time.
// `MatrixAlphabetService` keeps `Letters` behind a mutex, so it is never
// shared, only moved between the camera consumer's threads.
unsafe impl Send for Letters {}

impl Letters {
    pub(in crate::engine) fn new() -> Result<Self, MatrixOcrError> {
        let mut missing = Vec::new();
        let library = LIBRARIES
            .iter()
            // SAFETY: loading the packaged LiteRT library runs only its own
            // initializers; it has no load-time requirements on the caller.
            .find_map(|&name| match unsafe { libloading::Library::new(name) } {
                Ok(library) => Some(library),
                Err(e) => {
                    missing.push(format!("{name}: {e}"));
                    None
                }
            })
            .ok_or_else(|| {
                MatrixOcrError::Model(format!("LiteRT is not available ({})", missing.join("; ")))
            })?;
        macro_rules! symbol {
            ($name:literal) => {
                // SAFETY: each name is a function of the TFLite C API
                // (tensorflow/lite/core/c/c_api.h) with exactly this signature.
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| MatrixOcrError::Model(format!("LiteRT {}: {e}", $name)))?
            };
        }
        let api = Api {
            model_create: symbol!("TfLiteModelCreate"),
            model_delete: symbol!("TfLiteModelDelete"),
            options_create: symbol!("TfLiteInterpreterOptionsCreate"),
            options_delete: symbol!("TfLiteInterpreterOptionsDelete"),
            options_threads: symbol!("TfLiteInterpreterOptionsSetNumThreads"),
            interpreter_create: symbol!("TfLiteInterpreterCreate"),
            interpreter_delete: symbol!("TfLiteInterpreterDelete"),
            resize_input: symbol!("TfLiteInterpreterResizeInputTensor"),
            allocate: symbol!("TfLiteInterpreterAllocateTensors"),
            input: symbol!("TfLiteInterpreterGetInputTensor"),
            output: symbol!("TfLiteInterpreterGetOutputTensor"),
            invoke: symbol!("TfLiteInterpreterInvoke"),
            tensor_type: symbol!("TfLiteTensorType"),
            quantization: symbol!("TfLiteTensorQuantizationParams"),
            num_dims: symbol!("TfLiteTensorNumDims"),
            dim: symbol!("TfLiteTensorDim"),
            byte_size: symbol!("TfLiteTensorByteSize"),
            copy_in: symbol!("TfLiteTensorCopyFromBuffer"),
            copy_out: symbol!("TfLiteTensorCopyToBuffer"),
            xnnpack_defaults: symbol!("TfLiteXNNPackDelegateOptionsDefault"),
            xnnpack_create: symbol!("TfLiteXNNPackDelegateCreate"),
            xnnpack_delete: symbol!("TfLiteXNNPackDelegateDelete"),
            add_delegate: symbol!("TfLiteInterpreterOptionsAddDelegate"),
        };
        // SAFETY: MODEL is static, so the buffer outlives the model as the C
        // API requires; every handle is checked before use and released once.
        unsafe {
            let model = (api.model_create)(MODEL.as_ptr().cast(), MODEL.len());
            if model.is_null() {
                return Err(failed("model"));
            }
            let options = (api.options_create)();
            if options.is_null() {
                (api.model_delete)(model);
                return Err(failed("options"));
            }
            // Two threads, as the earlier ONNX Runtime session used.
            (api.options_threads)(options, 2);
            let mut xnnpack_options = (api.xnnpack_defaults)();
            xnnpack_options.num_threads = 2;
            let xnnpack = (api.xnnpack_create)(&xnnpack_options);
            if xnnpack.is_null() {
                (api.options_delete)(options);
                (api.model_delete)(model);
                return Err(failed("XNNPACK"));
            }
            (api.add_delegate)(options, xnnpack);
            let interpreter = (api.interpreter_create)(model, options);
            (api.options_delete)(options);
            if interpreter.is_null() {
                (api.xnnpack_delete)(xnnpack);
                (api.model_delete)(model);
                return Err(failed("interpreter"));
            }
            let input = (api.input)(interpreter, 0);
            let channels_first =
                !input.is_null() && (api.num_dims)(input) == 4 && (api.dim)(input, 1) == 1;
            Ok(Self {
                api,
                model,
                xnnpack,
                interpreter,
                batch: 0,
                channels_first,
                _library: library,
            })
        }
    }

    /// `CLASSES` logits per image, in order.
    pub(in crate::engine) fn logits(
        &mut self,
        images: &[Vec<u8>],
    ) -> Result<Vec<f32>, MatrixOcrError> {
        let api = &self.api;
        let input = normalized(images);
        let mut logits = vec![0f32; images.len() * CLASSES];
        // SAFETY: the interpreter is live; tensor pointers are re-read after
        // every allocation, and each copy states the exact byte size, which
        // the C API checks against the tensor.
        unsafe {
            if self.batch != images.len() {
                let (n, side) = (images.len() as i32, SIZE as i32);
                let dims = if self.channels_first {
                    [n, 1, side, side]
                } else {
                    [n, side, side, 1]
                };
                if (api.resize_input)(self.interpreter, 0, dims.as_ptr(), 4) != 0
                    || (api.allocate)(self.interpreter) != 0
                {
                    self.batch = 0;
                    return Err(failed("allocation"));
                }
                self.batch = images.len();
            }
            // The INT8 export takes and returns int8 tensors with a declared
            // scale; a float export takes the normalized values as they are.
            let tensor = (api.input)(self.interpreter, 0);
            if tensor.is_null() {
                return Err(failed("input"));
            }
            let copied = match (api.tensor_type)(tensor) {
                FLOAT32 if (api.byte_size)(tensor) == input.len() * 4 => {
                    (api.copy_in)(tensor, input.as_ptr().cast(), input.len() * 4)
                }
                INT8 if (api.byte_size)(tensor) == input.len() => {
                    let q = (api.quantization)(tensor);
                    let codes: Vec<i8> = input
                        .iter()
                        .map(|v| (v / q.scale).round() as i32 + q.zero_point)
                        .map(|v| v.clamp(-128, 127) as i8)
                        .collect();
                    (api.copy_in)(tensor, codes.as_ptr().cast(), codes.len())
                }
                _ => -1,
            };
            if copied != 0 {
                return Err(failed("input"));
            }
            if (api.invoke)(self.interpreter) != 0 {
                return Err(failed("inference"));
            }
            let tensor = (api.output)(self.interpreter, 0);
            if tensor.is_null() {
                return Err(failed("output"));
            }
            let copied = match (api.tensor_type)(tensor) {
                FLOAT32 if (api.byte_size)(tensor) == logits.len() * 4 => {
                    (api.copy_out)(tensor, logits.as_mut_ptr().cast(), logits.len() * 4)
                }
                INT8 if (api.byte_size)(tensor) == logits.len() => {
                    let q = (api.quantization)(tensor);
                    let mut codes = vec![0i8; logits.len()];
                    let status = (api.copy_out)(tensor, codes.as_mut_ptr().cast(), codes.len());
                    for (value, code) in logits.iter_mut().zip(codes) {
                        *value = (code as i32 - q.zero_point) as f32 * q.scale;
                    }
                    status
                }
                _ => -1,
            };
            if copied != 0 {
                return Err(failed("output"));
            }
        }
        Ok(logits)
    }
}

impl Drop for Letters {
    fn drop(&mut self) {
        // SAFETY: the handles came from this library, which is still loaded,
        // and are released exactly once: the interpreter before the delegate
        // it uses and the model it reads.
        unsafe {
            (self.api.interpreter_delete)(self.interpreter);
            (self.api.xnnpack_delete)(self.xnnpack);
            (self.api.model_delete)(self.model);
        }
    }
}
