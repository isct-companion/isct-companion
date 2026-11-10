# LiteRT

The letter model's runtime on Android and Windows (`src/engine/letters/litert.rs`).

- Android: `com.google.ai.edge.litert:litert:2.3.2` from Google Maven; the app
  packages its `libLiteRt.so` (5.6 MB) and excludes the `litert-api` Kotlin
  module, which needs a newer Kotlin and is never called. Core also opens
  1.x's `libtensorflowlite_jni.so`, but 1.4.2 hands only 18 of the model's
  126 nodes to XNNPACK. 2.3.2 crashes on an x86_64 emulator's ARM
  translation while creating the interpreter; phones run it.
- Windows x86_64: Google's prebuilt `libLiteRt.dll` 2.3.1,
  https://storage.googleapis.com/litert/binaries/2.3.1/windows_x86_64/libLiteRt.dll
  (SHA-256 `30fcb695ce8c2d8da5416337b6f4dc4407f04823b331cf6782a4894a82ee2ebb`),
  fetched and checked by `build.rs` and installed beside the executable. It
  links the C runtime statically; no Visual C++ redistributable is needed.
  An offline build passes the file as `ISCT_LITERT_DLL`.

Both libraries export the TFLite C API, which Core opens at run time.
License: Apache-2.0 (`LICENSE`, from https://github.com/google-ai-edge/LiteRT).
