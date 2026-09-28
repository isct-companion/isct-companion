# Embedded English PaddleOCR

- Model: `en_PP-OCRv5_mobile_rec`, official PaddlePaddle ONNX export.
- Source: https://huggingface.co/PaddlePaddle/en_PP-OCRv5_mobile_rec_onnx
- Revision: `3fafbc3b5dcf93dd72add9f48368be8a3a2cd33b`.
- `en-ppocrv5.onnx`: upstream `inference.onnx`, 7,848,423 bytes, unchanged.
- `characters.json`: that revision's YAML character dictionary, with CTC blank prepended and space appended (438 entries).
- Model license: Apache-2.0 (`PADDLE-LICENSE`).
- Rust wrapper: `ort = 2.0.0-rc.12`, native ONNX Runtime 1.24 API; its prebuilt CPU runtime is 1.24.2 on macOS arm64.
- Runtime license: MIT (`ONNX-RUNTIME-LICENSE`); upstream notices in `ONNX-THIRD-PARTY-NOTICES.txt`.

The model and dictionary are compiled into the native executable via `include_bytes!` / `include_str!`. ONNX Runtime is statically linked; there is no runtime model download, CDN, Python installation or separately located dylib. On a clean build Cargo downloads the platform-specific native runtime. Cross-compiling requires an ort-supported target or a matching custom static ONNX Runtime build; see https://ort.pyke.io/setup/linking.
