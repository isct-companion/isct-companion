# Native Core SDK checkpoint

The shared facade is `crates/isct-ffi`; native SwiftUI/Compose applications are
the next integration stage. See [SDK integration](../core-ffi.md) and
[shared domain API](../core-api.md) for the implemented contract.

## Local verification — 2026-10-15

| Check                                                                                                     | Result                                                               |
| --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `cargo test -p isct-core -p isct-ffi -p isct-companion --features isct-ffi/ocr --locked -j2`              | 373 passed, including 23 desktop integration tests                   |
| `cargo clippy -p isct-ffi -p isct-core --all-targets --features isct-ffi/ocr --locked -j2 -- -D warnings` | Passed                                                               |
| Desktop frontend                                                                                          | 47 files / 267 tests passed                                          |
| `yarn desktop:build --debug --no-bundle -- --locked -j2`                                                  | Passed; desktop binary retained                                      |
| Root script tests                                                                                         | 3 files / 9 tests passed                                             |
| Swift 6 and Kotlin/JVM clients                                                                            | Real Core, SQLite, native callback exception recovery and OCR passed |

Rust ownership tests exercise detachment without aborting a commit, cooperative
cancellation, bounded shutdown waits that retain draining work, queue overflow
and snapshot recovery, stale browser callbacks, foreign-port release and worker
panic containment. Both foreign clients recover from an unexpected native vault
exception, then successfully save/delete credentials and continue Core calls.

The review-fix checkpoint additionally covers admission-year catalog eligibility,
atomic rejection of an ineligible course batch, single Core-owned user-intent
events, local-restore event semantics, independent wake-service outcomes and
native event-port release.

## Packaged-target CI

[The iOS SDK job](https://github.com/isct-companion/isct-companion/actions/runs/38689356250/job/116271429048)
passed at source checkpoint `769a99b`, including the Swift package consumer,
iOS simulator and OCR inference. Its dev-profile Swift SDK is uploaded as
the `isct-core-swift-sdk-dev-ci` workflow artifact. Device SDKs and simulator clients are
built in CI; no iOS simulator runtime is downloaded locally.

[Android AAR/ART acceptance](https://github.com/isct-companion/isct-companion/actions/runs/38689356250/job/116271428805)
passed at checkpoint `769a99b`: both ABI libraries built, the Kotlin consumer
loaded the actual AAR on Android API 34, and `KOTLIN_CORE_SDK_SMOKE_PASSED`
completed. The AAR and dependency metadata are uploaded as `isct-core-android-sdk-dev-ci`.

The current workflow names new uploads `isct-core-swift-sdk-dev-ci` and
`isct-core-android-sdk-dev-ci`. Both use the dev profile for CI speed; they are
verification evidence rather than release builds.

The iOS consumer loads the shipped Swift package/XCFramework, compiles in Swift
6 and requires OCR capability before model warmup/inference. Android builds
arm64 and x86_64 AAR libraries, then loads the AAR on an API 34 x86_64 emulator
using ART. That ABI intentionally reports OCR unavailable; arm64 OCR is compiled.

The clients use temporary SQLite stores and fixture vaults. Real school sign-in,
OS authorization UI, cloud-provider writes, full native screen flows and physical
device acceptance belong to native-shell integration.

## Cleanup accounting

The review fix removes the separate desktop/FFI pull-event forwarding loops
and moves credential notification and periodic domain wake orchestration behind
Core. New eligibility checks and boundary contracts add production code; the
regression tests and generated bindings are not counted as production reduction.

The runtime spike was replaced by production-facade tests: 369 lines of isolated
Rust/Swift/Kotlin test code and its runner, a 14-line manifest and a duplicate
769-line Cargo lockfile were removed. These are test/tooling reductions, not
production-code reduction. This stage adds a new native boundary; it does not
claim a net reduction in production logic.

Generated bindings, SDK binaries and build evidence remain ignored. Public
syllabus test excerpts now live in Core's fixture directory so clean CI checkouts
compile without ignored local research artifacts.
