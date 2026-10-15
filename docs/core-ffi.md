# Native Core SDK

`crates/isct-ffi` projects `isct-core` into Swift and Kotlin using pinned UniFFI
0.32.1. Core has no UniFFI, Tauri or mobile-framework dependency. The adapters
share the domain contract in [core-api.md](core-api.md); transport naming follows
each generated language rather than introducing a mobile HTTP server.

## Build and integration

| Command               | Output                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------- |
| `yarn sdk:generate`   | Swift/Kotlin sources under ignored `target/native-sdk/generated/`                           |
| `yarn sdk:test`       | Real Core calls from Swift 6 and Kotlin/JVM                                                 |
| `yarn sdk:test --ocr` | Same clients with optional scanner preparation/inference                                    |
| `yarn sdk:ios`        | Device + Apple Silicon simulator XCFramework and a local Swift package in `bindings/swift/` |
| `yarn sdk:android`    | Android arm64/x86_64 AAR, dependency POM and generated Kotlin source in `bindings/kotlin/`  |

Builders default to release; `ISCT_SDK_PROFILE=dev` selects a faster debug SDK
and is recorded in `artifacts/sdk-build.json`. Generated sources and binaries
are ignored. Swift/Kotlin packages must be regenerated together with the Rust
library: UniFFI checks signatures at load time. The native-sdk workflow runs
both device builds and simulator clients, and uploads the complete SDK folders
as explicitly named `dev-ci` acceptance artifacts. These debug artifacts are not
an app-store or Maven release; release packaging is a separate workflow concern.
The workflow intentionally runs for matching pull requests and again when a
matching SDK/Core path lands on `main`. The latter packages and exercises the
exact merged revision; path filters keep unrelated main-branch changes out.

The Swift package supports iOS 17 and newer. It does not declare macOS support:
the host smoke links generated source directly to a host dynamic library and is
only a binding-contract check. Swift requires Xcode and installed iOS
device/simulator SDKs. The simulator smoke additionally requires an installed
runtime, supplied by CI. Android needs
`ANDROID_HOME`, `NDK_HOME`, Android API 34/build tools 36, Java and a Kotlin
compiler. `scripts/bindings/android-smoke.mjs` loads the AAR's Kotlin and native
library on Android ART using a running emulator. It does not use a school account.

Android API 34 is the current product `minSdk`, independently of the API 34 CI
emulator used for packaged acceptance. The build scripts keep the native Clang
target, AAR manifest and D8 output on that single floor. Android's
[NDK build documentation](https://developer.android.com/ndk/guides/other_build_systems)
defines the API suffix on a Clang target triple as `minSdkVersion`, while the
[D8 documentation](https://developer.android.com/tools/d8) defines `--min-api`
as the output DEX compatibility floor. A lower product floor needs a complete
AAR test on that API, including JNA and the arm64 OCR runtime; the API 34 value is
therefore retained pending that decision. The `ort` Cargo feature `api-24` selects
ONNX Runtime 1.24's C API and is unrelated to Android API level 24.

The Swift wrapper target uses Swift 5 language mode because this pinned UniFFI
version's async foreign-callback template fails Swift 6 region-isolation
checking. The public records are Sendable and the consumer smoke compiles in
Swift 6. This confines the compatibility setting to the generated SDK module;
SwiftUI applications can use Swift 6. Generated Kotlin may emit harmless
UNUSED_EXPRESSION warnings on newer compilers; sources are not patched.

## One session and explicit operation ownership

Create `Session.open(config, vault, browser?, cloud?, notifications?, downloads?)`
once per app data directory. Opening storage runs on an owned worker. Directory
paths must be absolute and supplied by the native shell. The SDK uses `school.db`
and the existing Core schema; it does not migrate an installed desktop app's
container. The vault implements Core account names and must preserve the app's
Keychain/Keystore identity. Credentials do not appear in workspace snapshots.

All service methods return an `Operation` immediately. Its identity is assigned
before work is admitted. `awaitResult()` returns a typed `OperationResult`;
`status()` recovers the current pending/terminal result. Snapshots retain both
`revision` (user writes) and `contentRevision` (all content/read-model caches).

- A Swift/Kotlin task stopping its wait does not abort admitted work.
- The supplied `value()` helpers detach when their native task is cancelled.
  Swift reports this as `CancellationError`; `SdkError.Cancelled` remains a Core
  cooperative-cancellation result when the Swift task itself is still active.
- `detach()` stops this handle's waiters; the underlying operation continues.
- `cancel()` cooperates with cancellable capabilities such as OCR and returns
  false for transactions/shared pulls. Use `cancelPull(id)` for domain pull
  cancellation. An admitted database/file commit always finishes.
- `shutdown()` closes admission and waits for owned work and native browser
  shutdown. A 15-second wait expiry returns typed Timeout while the drain keeps
  running; call shutdown again to wait. Repeated successful shutdown is safe.
- A native browser shutdown failure remains a typed error on subsequent waits.
- Finalization initiates cleanup off the foreign thread. Applications should
  explicitly await shutdown; a destructor cannot guarantee cleanup at process exit.

## Services

| Surface               | Main methods                                                                                                                            |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Workspace/read models | `snapshot`, `mutate`, `scheduleQuery`, `finderQuery`, `lmsMergePreview`                                                                 |
| Accounts              | `credentialStatus`, `saveCredentials`, `deleteCredentials`                                                                              |
| School                | `openSchool`, `schoolSession`, `schoolLifecycle`, `ensureSchoolSession`, `schoolBrowsing`, navigation/back/forward/reload, `pullSchool` |
| References/pulls      | `refreshReferences`, `pullState`, `cancelPull`                                                                                          |
| Files/notes           | `openMaterial`, `cleanMaterials`, `saveNote`, `readNote`, `listNotes`, `parseLmsResource`                                               |
| Backup/cloud          | `exportBackup`, `applyBackup`, `cloudState`, `syncCloud`, `setCloudEnabled`, `setCloudAutomatic`, `restoreCloud`                        |
| Lifecycle             | `lifecycle`, `suspend`, `resume`, `wake`, `shutdown`                                                                                    |
| Optional OCR          | `capabilities`, `prepareScanner`, `scanMatrix`                                                                                          |

Core decides due work. The shell supplies network/foreground/browser facts,
periodic wakeups, and native background opportunities. Suspend/resume uses the
existing Core coordinator rather than a foreign state machine. Wakeups are not
a guarantee that iOS or Android will permit background execution.

## Contract generation and errors

The build script reads Core's serde records/enums and mechanically generates
UniFFI value types. No independent Swift/Kotlin list of intent/query cases is
maintained. The dynamic `serde_json::Value` fields are represented by `JsonValue`
(valid JSON text); service methods and intent/query/result discriminants remain
typed. Arrays become foreign lists and Core's deserializer rechecks fixed lengths.
Business validation, timestamps, revision checking and commits remain in Core.
Foreign callers supply complete records, including default field values; Core's
custom serde defaults are not automatically foreign constructor defaults.
`openSchool` returns `SchoolWeb { result: SchoolWebResult }`; `pullSchool`
returns `SchoolPull { outcomes: List<IngestOutcome> }`. Dynamic document fields
do not erase these result or outcome discriminants.

`SdkError` distinguishes InvalidInput, Conflict, Storage, Compatibility,
Authentication, Network, Timeout, Busy, Unavailable, Callback, Cancelled, Closed and
Internal. Match variants, not diagnostic text. Unexpected native vault callback
exceptions retain Callback; denied vault access remains Unavailable. Browser and
cloud service errors retain their existing Core platform/transport categories.
After an Internal failure, recover a snapshot before deciding whether to retry;
the SDK never automatically replays a mutation. Invalid foreign input maps to
InvalidInput; an impossible Core-to-SDK projection maps to Internal. OCR Busy
means retry after the active scan, while Unavailable means the capability cannot
currently run. Cookie access failures retain network/timeout/cancellation rather
than being mislabeled as missing login. Reminder errors retain storage, input,
compatibility, task and native-delivery categories.

## Subscriptions and native callbacks

Call `subscribe()` before reading initial snapshots, then consume queued
`next()` updates. Events retain Core's existing event names and full payloads,
with a per-session sequence. Workspace writes also emit `workspace-changed`
with source `user-intent`, emitted by Core after a successful commit in every
shell. Delivery failure does not turn that committed mutation into an operation
failure. Local `applyBackup` emits source `local-restore`; cloud recovery retains
`cloud-restore`. Pull and OCR progress use the same typed `CoreEvent` envelope.
Snapshots are authoritative; events are hints.

The bounded queue holds 128 events. A slow consumer receives `SnapshotRequired`
and must reread relevant snapshots. `unsubscribe()` wakes pending next calls and
returns Closed; it does not free the Kotlin object handle, which also has the
generated `close()` resource method. Session shutdown closes subscriptions.

Native ports are mechanisms:

- `NativeVault`: blocking Keychain/Keystore operations, called on workers.
- `NativeBrowser`: promptly enqueue UI work. Install the SDK's `schoolBrowserInitialization()` scripts at document start in each
  view/frame; those assets come from Core, including Android frame support.
  `BrowserEvents` reports completion
  using browser generation, page sequence and action ID. Never synchronously
  wait for the main thread from a callback. Shutdown disables native admission.
- `NativeDownloadSession`: async authenticated cookie access for Core downloads.
- `NativeNotifications`: async system notification delivery.
- `NativeCloud`: async provider transport; Core owns merge/queue/recovery policy.

`BrowserEvents` holds only a weak engine listener. Stale generations are rejected
by Core, and callbacks retained beyond shutdown return Closed. Native browser
presentation, OAuth UI and authorization prompts are implemented when each shell
lands; no Tauri adapter is imported into the SDK.

## OCR

`ocr` is an optional Cargo feature; a base SDK works without model/runtime code.
Scanner initialization is lazy. `prepareScanner()` loads the model and performs
an inference warmup, which also allows device-side verification without private
images. Model failure affects only this capability. Scan progress arrives as
`ocr-progress` with the operation ID; the shell converts HEIC to a supported image format before calling.

The pinned ONNX distribution supports Android arm64 and iOS device/simulator.
Intel macOS and Android x86_64 build the unavailable backend, including when the
SDK's OCR feature is selected. Consumers use `capabilities().ocr` rather than
checking operating-system strings. Android emulator tests cover this fallback;
iOS simulator tests exercise model preparation and inference.

## Verification boundary

The real-facade Rust tests cover revision conflicts, typed conversion, committed
writes after caller detachment, queue overflow/recovery, subscription disposal,
cooperative cancellation, timeout without aborting a commit, callback failures,
shutdown failures, stale callback rejection and native-port release.

The foreign smoke clients use a temporary real SQLite store and fake platform
ports. They exercise writes, conflicts, read models, parser/backup access,
subscriptions, optional OCR, suspension/resume and repeated shutdown. Device-side
CI uses the packaged native targets. The Swift client cancels a task after its
operation wait begins, confirms that the waiter detaches, and verifies that the
admitted write still completes. Real school sign-in, real cloud-provider
mutations and complete native UI flows remain native-shell acceptance work.

Commands and packaged-target CI evidence are recorded in the
[native SDK checkpoint](verification/native-sdk.md).
