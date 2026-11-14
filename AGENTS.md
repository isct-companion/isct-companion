# Agent guidelines

Rules for any coding agent (Codex, Claude Code, …) working in this repository.

## 1. Use existing libraries before writing your own

If a dependency already in the project, or a well-maintained library, solves
the problem, use it instead of hand-rolling an equivalent. Examples: routing is
react-router, notifications are Radix Toast, page keep-alive is
React's `<Activity>`, UI text goes through i18next. Write custom code only when
no suitable library exists, and say why in a short comment.

## 2. Extract shared logic behind one interface

When two places (desktop and mobile, iOS and Android, two pages) need the same
behaviour, put it in one module with one interface and let each side call it.
Do not copy the logic. Example: `crates/app-core/src/school_session` signs in
over HTTP for every platform, and `crates/app-core/src/school_browser` decides
when a school page recovers, for every school-web host.

## 3. Isolate native differences at compile time, not with runtime checks

Behaviour that differs because of the native platform is separated in source
at build time, like `#ifdef`: Rust `#[cfg(...)]` (`target_os`, `matrix_ocr`),
per-platform files (`notifications_macos.rs`, `school_webkit.rs`), separate
native apps (`apps/ios`, `apps/android`), and the values the binary injects at startup (`__ISCT_COMPANION_NATIVE_TARGET__`,
`__ISCT_COMPANION_NATIVE_OS__`, read via `nativeOs()`). Never detect the
platform at runtime from `navigator.userAgent`, `navigator.platform`, touch
points or similar heuristics.

The same holds for anything else a platform fixes in advance:

- Code only one platform runs is compiled only there, including protocol
  clients and their fallbacks. A fallback for another OS's behaviour is dead
  code.
- One executable per role. A program must not work out at runtime which of
  several jobs it was started for, from `argv[0]`, a subcommand or its
  environment. Build each role as its own binary.
- A platform's own paths and tools are `cfg` constants. Do not check whether
  files exist, walk a candidate list, or try another OS's paths. Ask the system
  at runtime only for what really varies within one platform (the layout a
  service reports about itself, software the person installed). When the
  answer is missing, fail with a clear error instead of guessing.

The product targets are macOS (Apple silicon and Intel), Windows, iOS/iPadOS
and Android arm64-v8a only. Linux, HarmonyOS, 32-bit Android (armeabi-v7a) and
Android x86/x86_64 are not supported: do not add branches, dependencies,
fallbacks or checks for them. Linux stays only as a build host (CI runners, NDK
host tools, model training), and `--with-emulator` and the x86_64 SDK smoke stay
as CI infrastructure on x86_64 runners, not a product target. The desktop crate
refuses to compile for any other system.

Allowed exceptions:

- iOS vs iPadOS. One iOS binary runs on both, so telling them apart at runtime
  is fine.
- Layout by size. Adapting a layout to the window or pane size (CSS media
  queries, an element's measured width) is a layout decision, not native
  behaviour. The desktop TS frontend serves only the desktop app: there is no
  phone layout to switch to, and iOS and Android are their own native apps.

## 4. Consolidate domain logic and state in Rust

`crates/app-core` owns domain decisions, mutations and read models. Shells
render, forward intents and implement platform ports; they do not maintain
parallel business state machines. Keep platform handles out of core and use
compile-time platform adapters. The contracts are decisions in slotledger
(`goal/core-decides-shells-render` and the `approach/` keys that build on it).

New public core/FFI boundaries use typed errors. Remove non-test panic paths
as modules migrate; enable `clippy::unwrap_used`, `expect_used` and `panic`
denies for the completed core/FFI boundary. Release profiles use unwind, which
alone is not crash recovery.

Shells provide lifecycle facts and one timer for Core's `WakeDirective`;
core decides what is due and when (`approach/core-deadline-plan`), and no
service keeps a clock of its own.
Bounded task waits require explicit ownership and shutdown semantics. Expose
coarse services through FFI, with correlated callbacks and snapshot recovery.

Preserve active desktop behaviour and stored-data compatibility at each tested
checkpoint, then commit. Remove superseded implementations and state owners;
record production-code reduction separately from tests and generated code.
Mobile shells will be redesigned; old mobile UI parity is not required.

## 5. Keep every test in one place, and pick the layer before writing it

All JavaScript/TypeScript tests live under `apps/desktop/tests/`, grouped by the behaviour
they protect. Do not add a `*.test.ts(x)` beside the module it exercises, and do
not add a suite for a new file. Read `apps/desktop/tests/README.md` before writing one: it
lists the suites, the shared harness, and the rule that `unit/` never mounts a
React component while `e2e/` mounts the real shell over a scripted host.

Rust test code lives in `src/tests/` for each crate. A test module stays a child
of the module it tests — that is what keeps private items reachable — but the
file itself lives in that one folder.

Prefer the real thing over a mock: a suite that replaces the code under test
with a mock proves nothing about the app. Reach for the harness first, and mock
only a dependency with no headless mode, with a comment saying why.

Never build hand-made fake data, fixtures or mocks of school pages or Isct-Data
files: verify against real data. Checks on the data itself (format, references,
coverage) live in the repository that holds it, Isct-Data, as Python scripts.

Then decide whether the behaviour belongs in a test at all:

- Automatable and cheap to run every commit: put it in `apps/desktop/tests/unit`
  or `apps/desktop/tests/e2e`.
- A lasting product risk that needs real credentials, a consent screen,
  hardware, a signed bundle or a native shell: add a check to
  `docs/verification/manual-checks.yaml`, not a test. Each check names a
  persona, states why each step exists, says where to observe the result, and
  justifies in `why_manual` why automation cannot reach it. Run
  `yarn verify:manual stale` to see which checks a change invalidated.
- A newly added feature working at all (it opens, it shows, a tap does what it
  should): no check and no e2e script. The person tries new features
  themselves; manual checks are only for lasting product risks the suites
  cannot reach. Logic behind the screen still gets its test at the layer that
  owns it, usually Core.

One-time migration, refactor cutover, and upgrade acceptance checks do not enter
the long-lasting manual checklist. Remind the user to perform them in the
current conversation; retain checks for ongoing product risks.

A check whose covered paths are all already exercised by the suites is rejected,
and so is a step with no reason: "the manual table" must not become the place
where hard tests go to be forgotten.

## 6. A new test has to earn its place, and an old one has to keep it

Tests are a cost that compounds, and a suite that only grows stops being read.
Before adding a case, name the behaviour it protects and the bug it would catch.
If an existing case already covers that behaviour, extend it rather than adding a
sibling; if nothing covers it, add it.

The same bar removes:

- A test whose body says nothing beyond the line it calls: one `assert_eq!` over
  an inline literal, a getter round-trip, a constant restated. Delete it.
- A test a sibling already covers, in the same file, crate or suite. Fold in
  whatever it asserted that the survivor does not, then delete it.

Every removal needs a named survivor, or the statement that nothing covered it
and it asserted only a literal. If you cannot point at one of those, keep the
test and say why in your report. Dropping an assertion you cannot justify is
worse than keeping a redundant one.

Two things this is not. It is not licence to delete a boundary case, a rejection
path or a safety invariant because it looks small — an ignored test that needs a
private image is not redundant with a synthetic one. And it is not compression
for its own sake: consolidating a scattered suite into fewer, larger files will
usually add lines while raising coverage, and that is the intended outcome. The
thing to drive down is the number of places a reader has to look, not the number
of assertions.

## 7. Keep project state in slotledger only

Decisions, plans, tasks, handoffs and notes live in slotledger, through its MCP
tools (set up with `npx slotledger init`; usage in
`.claude/skills/slotledger/SKILL.md`). Do not
write handoff, PLAN, TODO or notes files, including notes between sessions, and
do not keep project state in agent memory (Claude's auto memory, Codex
memories). Hand work over with the task's `handoff`; record what you would
otherwise remember with `record` or `task`. `STATUS.md` is generated from
`.project/events.jsonl`; never edit either by hand.

## 8. Preserve human authorship when squashing

Every squash merge must preserve attribution for all human contributors
in the squashed commits: collect their commit authors and existing human
`Co-authored-by` trailers, deduplicate them, and include everyone other than
the final commit's author as `Co-authored-by: Name <email>` trailers in the
final commit message. Preserve their recorded identities; use an email
associated with their GitHub account (including their GitHub no-reply
address), and never invent an email. Verify the final message before merging
or pushing; do not rely on the squash interface to preserve attribution.

Use the repository's configured git author as is. Never add a
`Co-authored-by` trailer for an AI (Claude, Codex, …) or any other AI
attribution line to commit messages or PR descriptions.
