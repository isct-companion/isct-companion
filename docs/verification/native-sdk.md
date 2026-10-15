# Native Core SDK checkpoint

The shared facade is `crates/isct-ffi`; native SwiftUI/Compose applications are
the next integration stage. See [SDK integration](../core-ffi.md) and
[shared domain API](../core-api.md) for the implemented contract.

## Local verification — 2026-10-15

| Check                                                                                                     | Result                                                               |
| --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `cargo test --workspace --locked -j2`                                                                     | 378 passed; 4 private OCR image tests explicitly ignored             |
| `cargo test -p isct-ffi --features ocr --locked -j2`                                                      | 11 passed                                                            |
| `cargo clippy -p isct-ffi -p isct-core --all-targets --features isct-ffi/ocr --locked -j2 -- -D warnings` | Passed                                                               |
| Desktop frontend                                                                                          | 47 files / 266 tests passed                                          |
| `yarn desktop:build --debug --no-bundle -- --locked -j2`                                                  | Passed; desktop binary retained                                      |
| Root script tests                                                                                         | 3 files / 9 tests passed                                             |
| Swift 6 and Kotlin/JVM clients                                                                            | Real Core, SQLite, native callback exception recovery and OCR passed |

Rust ownership tests exercise detachment without aborting a commit, cooperative
cancellation, bounded shutdown waits that retain draining work, queue overflow
and snapshot recovery, stale browser callbacks, foreign-port release and worker
panic containment. Both foreign clients recover from an unexpected native vault
exception, then successfully save/delete credentials and continue Core calls.

## Packaged-target CI

[The iOS SDK job](https://github.com/isct-companion/isct-companion/actions/runs/38682438947/job/116249763838)
passed at source checkpoint `3503b8d`, including the Swift package consumer,
iPhone 16 Pro simulator and OCR inference. Its complete Swift SDK is uploaded as
the `isct-core-swift-sdk` workflow artifact. Device SDKs and simulator clients are
built in CI; no iOS simulator runtime is downloaded locally.

[Android AAR/ART acceptance](https://github.com/isct-companion/isct-companion/actions/runs/38682438947/job/116249762920)
passed at checkpoint `3503b8d`: both ABI libraries built, the Kotlin consumer
loaded the actual AAR on Android API 34, and `KOTLIN_CORE_SDK_SMOKE_PASSED`
completed. The AAR and dependency metadata are uploaded as `isct-core-android-sdk`.

The iOS consumer loads the shipped Swift package/XCFramework, compiles in Swift
6 and requires OCR capability before model warmup/inference. Android builds
arm64 and x86_64 AAR libraries, then loads the AAR on an API 34 x86_64 emulator
using ART. That ABI intentionally reports OCR unavailable; arm64 OCR is compiled.

The clients use temporary SQLite stores and fixture vaults. Real school sign-in,
OS authorization UI, cloud-provider writes, full native screen flows and physical
device acceptance belong to native-shell integration.

## Cleanup accounting

The runtime spike was replaced by production-facade tests: 369 lines of isolated
Rust/Swift/Kotlin test code and its runner, a 14-line manifest and a duplicate
769-line Cargo lockfile were removed. These are test/tooling reductions, not
production-code reduction. This stage adds a new native boundary; it does not
claim a net reduction in production logic.

Generated bindings, SDK binaries and build evidence remain ignored. Public
syllabus test excerpts now live in Core's fixture directory so clean CI checkouts
compile without ignored local research artifacts.
