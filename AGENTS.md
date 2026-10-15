# Agent guidelines

Rules for any coding agent (Codex, Claude Code, …) working in this repository.

## 1. Use existing libraries before writing your own

If a dependency already in the project, or a well-maintained library, solves
the problem, use it instead of hand-rolling an equivalent. Examples: routing is
react-router, notifications are `@mantine/notifications`, page keep-alive is
React's `<Activity>`, UI text goes through i18next. Write custom code only when
no suitable library exists, and say why in a short comment.

## 2. Extract shared logic behind one interface

When two places (desktop and mobile, iOS and Android, two pages) need the same
behaviour, put it in one module with one interface and let each side call it.
Do not copy the logic. Examples: `crates/isct-core/src/school_browser` holds the
login scripts and relays used by every school-web adapter; the route table in
`apps/desktop/src/app/routes.ts` serves both the workstation and compact desktop layouts.

## 3. Isolate native differences at compile time, not with runtime checks

Behaviour that differs because of the native platform is separated in source
at build time, like `#ifdef`: Rust `#[cfg(...)]` (`desktop`, `mobile`,
`target_os`), per-platform files (`school_web_ios.rs`, `school_web_android.rs`),
and the values the binary injects at startup (`__ISCT_COMPANION_NATIVE_TARGET__`,
`__ISCT_COMPANION_NATIVE_OS__`, read via `nativeOs()`). Never detect the
platform at runtime from `navigator.userAgent`, `navigator.platform`, touch
points or similar heuristics.

Allowed exceptions:

- iOS vs iPadOS. One iOS binary runs on both, so telling them apart at runtime
  is fine.
- Layout by viewport. Choosing the compact or workstation layout from the window
  size (`apps/desktop/src/app/form-factor.ts`) is a layout decision, not native behaviour.

## 4. Consolidate domain logic and state in Rust

`crates/isct-core` owns domain decisions, mutations and read models. Shells
render, forward intents and implement platform ports; they do not maintain
parallel business state machines. Keep platform handles out of core and use
compile-time platform adapters. See `docs/core-consolidation.md` for contracts.

New public core/FFI boundaries use typed errors. Remove non-test panic paths
as modules migrate; enable `clippy::unwrap_used`, `expect_used` and `panic`
denies for the completed core/FFI boundary. Release profiles use unwind, which
alone is not crash recovery.

Shells provide lifecycle facts and periodic wakeups; core decides what is due.
Bounded task waits require explicit ownership and shutdown semantics. Expose
coarse services through FFI, with correlated callbacks and snapshot recovery.

Preserve active desktop behaviour and stored-data compatibility at each tested
checkpoint, then commit. Remove superseded implementations and state owners;
record production-code reduction separately from tests and generated code.
Mobile shells will be redesigned; old mobile UI parity is not required.
