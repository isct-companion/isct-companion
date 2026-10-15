# Kotlin / Android Core SDK

Run `yarn sdk:android` with `ANDROID_HOME` and `NDK_HOME` configured. The builder
produces `artifacts/isct-core.aar` for arm64-v8a and x86_64, a dependency POM and
`artifacts/sdk-build.json`. Keep the generated library and Kotlin source in sync.

Consume the AAR as a local file and declare `net.java.dev.jna:jna:5.17.0@aar`
and `org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2`, plus
`androidx.annotation:annotation-jvm:1.9.1`, or publish the AAR
with its generated POM to a private Maven repository. Generated bindings and the
`Operation.value()` coroutine helper are compiled into `classes.jar`; do not also
compile the generated source in the consuming app. Keep consumer R8 rules.

Create one `Session` with native ports. Subscribe before loading snapshots,
render Core read models and forward intents. At final teardown await
`session.shutdown()`, then call the generated object's `close()` to release its
native handle. `Subscription.unsubscribe()` stops event delivery; `close()` also
releases that object's handle. No domain state machine belongs in a ViewModel.

The x86_64 emulator SDK reports OCR unavailable while the arm64 SDK enables it.
See [the integration contract](../../docs/core-ffi.md) and
`crates/isct-ffi/tests/foreign/Smoke.kt` for a complete client.
