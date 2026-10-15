# Foreign runtime boundary spike

This is an isolated host fixture, not the application FFI facade. It validates
binding mechanics before service and browser-port contracts are finalized.
It does not establish mobile-target compatibility or application cancellation
semantics. The application must preserve shared pull ownership and the Applying
commit boundary; it must not copy the fixture's unconditional task abort into
those operations.

Run on macOS with Swift 6, Kotlin and a JDK:

```sh
node scripts/bindings/ffi-runtime-smoke.mjs
```

The fixture has its own locked Cargo workspace, pins UniFFI 0.32.1 and uses the
repository's existing `target/`. The runner downloads checksum-pinned JNA 5.17.0
and uses Kotlin's bundled coroutines jar (`KOTLIN_COROUTINES_JAR` can override
its location). Generated bindings and compiler output remain under `target/`.
The fixture is excluded from the production workspace and introduces no app
dependency or exported application API.

Verified on 2026-10-13 with the generated Swift and Kotlin bindings:

- A foreign async call enters an explicitly owned Tokio runtime, sleeps and
  returns the same request ID through its callback and result.
- An unexpected foreign callback exception becomes a typed `Callback` error
  via `From<UnexpectedUniFFICallbackError>` rather than panicking in Rust.
- Explicit cancellation settles the request with the application-level
  `Cancelled` error. Swift's weak listener reference becomes nil, and Kotlin's
  callback handle map returns to zero.
- Kotlin coroutine cancellation drops the foreign future, and the fixture's
  task guard aborts its owned task and removes its registry entry.
- Shutdown settles a pending request, prevents its callback, rejects new work
  with `Closed`, and can be called repeatedly. Kotlin object disposal succeeds.
- An intentionally injected Rust panic through a `Result` export becomes a
  catchable foreign error; both test processes continue and report success.
  The panic hook text in the log is expected for this test.

Findings that constrain the real facade:

1. Swift `Task.cancel()` alone did not cancel the generated async call. The
   generated Swift helper has no task-cancellation handler in this version.
   Provide an explicit operation cancellation method; allocate its identity
   before waiting so cancellation cannot race an unregistered request. Decide
   separately whether a caller detaches or the shared domain job is cancelled.
2. A non-`Result` export generated Swift `try!`. Rust unwind containment alone
   therefore cannot make that call recoverable. All operations that can fail,
   including listener registration/disposal and shutdown, need typed results;
   final destructors must not panic or block a foreign UI/runtime thread.
3. Use `detail` rather than `message` for an error payload field: generated
   Kotlin exception types already define their own `message` property.
4. Keep ownership of runtime, tasks and listeners explicit. The tested fixture
   uses `shutdown_background` in destruction and a task guard on a foreign
   wait; the real Core must additionally honor its persistence and coalescing
   contracts, provide snapshots, and settle every registered operation.

The fixture tests must be replaced or extended with the real facade during
the FFI phase, including callback generation IDs, subscription disposal,
snapshot recovery, suspension/resume and Swift/Kotlin mobile-target checks.
