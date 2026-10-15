# Native Core SDK checkpoint

The shared facade is `crates/isct-ffi`; native SwiftUI/Compose applications are
the next integration stage. See [SDK integration](../core-ffi.md) and
[shared domain API](../core-api.md) for the implemented contract.

## Local verification — 2026-10-15

| Check                                                                                                     | Result                                                               |
| --------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `cargo test -p isct-core -p isct-ffi -p isct-companion --features isct-ffi/ocr --locked -j2`              | 373 passed, including 23 desktop integration tests                   |
| `cargo clippy -p isct-ffi -p isct-core --all-targets --features isct-ffi/ocr --locked -j2 -- -D warnings` | Passed                                                               |
| Desktop frontend                                                                                          | 47 files / 267 frontend tests passed                                 |
| `yarn desktop:build --debug --no-bundle -- --locked -j2`                                                  | Passed; desktop binary retained                                      |
| Root script tests                                                                                         | 3 files / 9 script tests passed                                      |
| Swift 6 and Kotlin/JVM clients                                                                            | Real Core, SQLite, native callback exception recovery and OCR passed |

The JavaScript/TypeScript total is 276 tests: 267 desktop frontend tests plus
9 root script tests. The rows keep those suites separate so the frontend count
is not mistaken for the combined total.

Rust ownership tests exercise detachment without aborting a commit, cooperative
cancellation, bounded shutdown waits that retain draining work, queue overflow
and snapshot recovery, stale browser callbacks, foreign-port release and worker
panic containment. Both foreign clients recover from an unexpected native vault
exception, then successfully save/delete credentials and continue Core calls.

The review-fix checkpoint additionally covers admission-year catalog eligibility,
atomic rejection of an ineligible course batch, single Core-owned user-intent
events, local-restore event semantics, independent wake-service outcomes and
native event-port release.

## Study-profile and review follow-up — 2026-10-15

Targeted checks for the subsequent study-profile, event-recovery and SDK fixes:

- Core: 78 tests across study profile, finder, workspace, agent, persistence,
  events and facade; FFI: 15 tests, including typed study-history round-trip
  and invalid-input rollback.
- Desktop: 6 affected test files / 76 tests; TypeScript and production frontend
  build passed. The existing large-chunk advisory remains.
- Core/FFI strict Clippy, Rust formatting and desktop `cargo check` passed.
- Newly generated Swift 6 and Kotlin host clients passed. Swift cancellation
  now deterministically covers an active waiter detaching; a separate real-Core
  check proves the admitted write completes after detachment.

The checks cover legacy profiles and backups, graduate/transfer admission,
historical stages, class suffixes, authoritative catalog lookup, atomic
multi-quarter eligibility, paste diagnostics and cross-client event recovery.
These are local targeted results; the packaged-device CI evidence below is the
earlier SDK checkpoint.

Change size, separating embedded Rust test modules: production +1,076 / -145
lines; tests +1,063 / -55; generated types +34 / -1. This follow-up adds study
history and its editor, so it is a net addition rather than a reduction phase.

## Graduate compatibility and teaching periods — 2026-10-15

The [shared API](../core-api.md) now describes source-linked graduate planning
rules, explicit professional-degree identity, page availability, and the typed
`teachingPeriods` query. School registrations, grades and requirement results
remain authoritative; missing approval or prerequisite facts produce `unknown`.

Targeted checks at this checkpoint:

- Core: 177 distinct tests across study profile, schedule, workspace, agent,
  finder, catalog, ingestion/parsers, browser orchestration, refresh, scheduler,
  subscriptions and persistence. This includes empty/unavailable preservation,
  duplicate-grade rollback, stable graduate row identity, cross-quarter rules,
  evening classes, custom clocks, ICS and reminders.
- FFI: 16 tests, including professional identity, configured teaching periods,
  the typed query, malformed foreign pair rejection and atomic rollback.
- Desktop: 13 affected files / 106 tests. Thirteen tests execute the production
  教務 extraction script against DOM fixtures; four exercise course-editor
  preservation of period ranges and explicit custom time.
- TypeScript and production frontend build, desktop `cargo check`, Rust format,
  and Core/FFI strict Clippy passed. The existing Vite large-chunk advisory and
  warnings in UniFFI-generated Kotlin remain.
- Newly generated Swift 6 and Kotlin/JVM clients passed against the real Core,
  including the default `teachingPeriods` query and 11–12 evening block. This
  host run uses the default SDK features.

The graduate DOM fixture is derived from the public 教務 student manual linked
by the school, whose PDF was produced in 2017. It exercises the manual's separate
graduate/undergraduate grade tables with synthetic data; it does not establish
the current live DOM. No graduate account was available. Unknown or incomplete
structures are rejected before page reconciliation. Without a server-provided
manifest, a wholly absent table cannot be distinguished from an account-specific
page shape; recognized sections must each have rows or explicit scoped empty
evidence.

Line accounting separates embedded Rust test modules and fixture files:
production +1,969 / -470; tests +1,322 / -16; generated types +77 / -3. This phase
adds graduate support and stricter import contracts, so it is a net addition.
It removes independent clock tables and the mirrored private rule enum.

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
