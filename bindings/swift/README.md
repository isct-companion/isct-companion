# Swift Core SDK

Run `yarn sdk:ios` at the repository root. It generates `Sources/IsctCore/Generated.swift`
and `artifacts/IsctCore.xcframework`. Add this folder as a local Swift package and
`import IsctCore`. The wrapper target uses Swift 5 language mode; clients may use Swift 6.

Implement `NativeVault` and the platform capabilities your shell supports, open
one `Session`, subscribe before the first snapshot, and forward lifecycle facts.
Call service methods and await the returned operation's `value()` helper. At final
teardown await `session.shutdown()` before releasing it. Widgets read the app's
published glance snapshot; they do not instantiate Core or load OCR.

See [the integration contract](../../docs/core-ffi.md) and the actual Swift client
in `crates/isct-ffi/tests/foreign/Smoke.swift`. The Native Core SDK CI artifact
contains the generated sources, binary XCFramework and this package manifest.
