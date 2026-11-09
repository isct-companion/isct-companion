# ONNX Runtime

- Rust wrapper: `ort = 2.0.0-rc.12`, native ONNX Runtime 1.24 API.
- Runtime license: MIT (`LICENSE`); upstream notices in `THIRD-PARTY-NOTICES.txt`.

ONNX Runtime runs the bundled alphabet classifier (`../alphabet/`) on Linux;
macOS and iOS use the system's Core ML, Android and Windows LiteRT. It is
statically linked; there is no runtime model download, CDN, Python installation
or separately located dylib. On a clean build Cargo downloads the
platform-specific native runtime. Cross-compiling requires an ort-supported
target or a matching custom static ONNX Runtime build; see
https://ort.pyke.io/setup/linking.
