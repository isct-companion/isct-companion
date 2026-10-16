# Core consolidation

Execution brief for consolidating application logic in shared Rust services that the
desktop Tauri shell and future native mobile shells (SwiftUI, Jetpack Compose)
call through one interface. Read `AGENTS.md` first; every rule there applies.

## Goal

```text
crates/isct-core   domain logic and state: parsing, acquisition, sync, scheduling, storage
crates/isct-ocr    optional matrix recognition, independent of core and platform frameworks
crates/isct-ffi    UniFFI facade over isct-core for Swift and Kotlin
backend/           Tauri app (desktop today, Tauri mobile until native shells ship)
src/               React UI: renders state from core, sends user intents to core
```

- `isct-core` has no dependency on `tauri`, `wry`, `uniffi` or any UI crate.
- OCR is an independently assembled capability. Core does not depend on ONNX,
  image processing or model assets. An unsupported or disabled OCR backend
  returns a typed unavailable result without disabling other app services.
- Shells render, forward intents, and implement platform ports. They contain no
  domain decisions.
- Desktop behaviour is the acceptance baseline after every phase. Mobile UI
  and shells will be redesigned; preserving the old Tauri mobile design is
  not an acceptance requirement. Core portability to SwiftUI and Jetpack
  Compose remains a design requirement.
- This is a consolidation and cleanup, not a file relocation. Core owns the
  authoritative domain state, mutations and derived read models. Shells must
  not reconstruct business state through their own flags or decision rules.
- Refactoring and removal of unused features, dead code, redundant abstractions
  and duplicate implementations are authorized. Preserve active desktop user
  workflows and stored-data compatibility; establish usage before deleting a
  feature. Do not retain an unused layer merely because it existed before.
- Stability comes first. Reduce production code and independent state owners
  by removing duplication, rather than shortening code at the expense of
  explicit contracts and meaningful tests.

## Non-goals

- Building the SwiftUI or Compose shells. This brief ends at a tested FFI layer.
- Redesigning desktop UI or changing the persisted data/sync contract. Internal
  models and APIs may be simplified behind compatibility adapters.
- Replacing `ts-rs`. Core types keep deriving `TS` behind a `ts` feature so
  `src/generated` stays the desktop's type source.

## Inventory baseline (2026-10-13)

The counts below are an initial inventory, not a dependency graph or proof that
a module can move independently. Recount production code, tests and generated
files separately at each checkpoint. The OAuth script edits are deferred in a
separate Git stash and are outside this migration.

Backend: 34 top-level files in `backend/src`, ~21k lines, plus `data/`,
`school_parser/`, `syllabus/`, `matrix_ocr/`.

Tauri coupling by file (count of `tauri` mentions):

| File | Lines | Refs | What couples it |
|---|---|---|---|
| `lib.rs` | 1908 | 280 | ~90 `#[tauri::command]`s, state wiring, `require_local_main` |
| `lms_files.rs` | 1162 | 28 | commands, `AppHandle` for paths, opener |
| `workspaces.rs` | 517 | 20 | commands, `AppHandle` paths, `async_runtime` |
| `cloud_sync.rs` | 923 | 19 | `emit`, `Manager::state`, `async_runtime::spawn` |
| `school_refresh.rs` | 338 | 9 | `AppHandle` |
| `campus_wifi.rs` | 414 | 9 | commands (macOS system integration) |
| `scheduler.rs` | 323 | 8 | 30 s `tokio` interval owned by core, `emit` |
| `school_web_shared.rs` | 3444 | 6 | owns WebView handles, eval, bounds, downloads |
| `pull_service.rs`, `reference_refresh.rs`, `cloud.rs`, `secrets.rs`, `school_web*.rs`, `lms_downloader.rs`, `school_webkit.rs`, `lms_protocol.rs`, `icloud.rs` | | 1–5 | `AppHandle`, events, plugins |

Candidates with no direct Tauri imports: `syllabus.rs`, `subscriptions.rs`,
`school_extract.rs`, `school_auth.rs`, `pull_coordinator.rs`, `policy.rs`,
`plans.rs`, `matrix_red_crops.rs`, `lms_ingest.rs`, `lifecycle.rs`,
`kyomu_ingest.rs`, `kyomu_guide.rs`, `contracts.rs`, and the `data/`,
`school_parser/`, `syllabus/`, `matrix_ocr/` modules.

Move dependency-closed groups. For example, `school_auth` depends on the
`SecretVault` trait currently defined alongside a Tauri implementation; move
the trait first. Embedded JS, fixtures, model paths and generated TS paths
also belong to the dependency audit.

Other facts that shape the plan:

- `[profile.release] panic = "abort"`. A panic across FFI kills the app.
- 395 `Result<_, String>` signatures; ~548 `unwrap`/`expect` (including tests).
- 9 `app.emit(...)` calls: `workspace-changed` (with a `source`),
  `cloud-sync-state`, pull progress.
- No `rust-toolchain.toml`. `ort` is pinned to `=2.0.0-rc.12`.
- Matrix OCR is required on mobile. `MatrixOcrService::scan` takes two base64
  canvases produced in JS; HEIC is decoded in JS (`src/core/matrix-ocr.ts`).

Frontend logic that belongs in core (non-test lines):

| TS module | Lines | Logic |
|---|---|---|
| `src/storage/repository.ts` | 552 | export validation, schema checks, record collections |
| `src/store/workspace.ts` | 541 | workspace mutations (keep the zustand store as a view cache only) |
| `src/core/schedule.ts` | 448 | calendar state, quarter ranges, scheduled courses |
| `src/core/lms-merge.ts` | 344 | LMS ↔ catalog diff and merge |
| `src/platform/subscription-polling.ts` | 233 | poll due rules, reference kinds |
| `src/platform/reminders.ts` | 194 | due reminders, fired log |
| `src/platform/school-session.ts` | 173 | sign-in timeouts, focus re-check policy |
| `src/core/user-schedule.ts` | 135 | plan occurrences, intensive days |
| `src/core/course-workflows.ts` | 108 | batch edit/delete rules |
| `src/platform/school-freshness.ts` | 50 | TTLs, stale source detection |
| `src/core/*` (rest) | ~800 | academic calendar, ICS, credits, course query, text |
| `src/core/matrix-ocr.ts` | — | image decode/resize before OCR |

Stays in TS: rendering, layout (`school-web-bounds.ts`, timetable layout,
course colours), routing, i18n, and `desktop-bridge.ts` as a thin invoke layer.

Some listed code is already superseded: `subscriptionPollDue`,
`pollReferenceSubscriptions` and `staleSources` have no production call sites
at this baseline. Retain useful regression cases in the authoritative Rust
implementation and remove the unused implementation instead of porting it
again. Preserve any still-used exports in their appropriate shell/core layer.

## State ownership and compatibility contracts

The existing pull migration at `4a32ccf` is the starting point; see
[`school-refresh.md`](school-refresh.md). Consolidation must cover the whole
application, not stop once the pull modules are moved.

| Layer | Owner | Responsibilities |
|---|---|---|
| Intent and policy | Core | launch checks, TTL, academic year, debounce, admission policy |
| Pull coordination | Core | exact-key joining, priority, lanes, cancellation, retry, snapshots |
| Domain executors | Core | reference materialization, school session/extraction orchestration, persistence |
| Platform mechanisms | Shell ports | WebView operations, secure storage, CloudKit, lifecycle facts, OS wakeups |
| Presentation | Shell | rendering, layout, localization, user intents, cached core read models |

Keep domain-specific state machines for school sessions/navigation and cloud
sync where they describe different resources and outcomes. They belong in
Core and communicate through explicit contracts; do not force every operation
into a single giant pull machine. The pull reducer has 11 states and 14 events
(154 defined pairs), but its queue, waiters, run IDs and resource ownership are
extended runtime state. Total transitions alone do not prove runtime safety.
`Yielding` is currently reserved; real visible-browser preemption lives in the
school executor. Moving it must be explicit and tested, not inferred from the
presence of that enum variant.

Preserve these contracts through every applicable phase:

- One owner for each domain state and write path; UI caches cannot authorize
  or duplicate domain transitions. Reads expose a coherent revision/snapshot.
- Exact-key callers share the real outcome. Full LMS and course-detail pulls
  are distinct because they acquire different data. School has one execution
  lane; reference HTTP has two. Manual priority and equal-priority FIFO remain.
- Cancellation before Applying prevents writes. Once Applying begins, writes
  retain their existing commit boundary; running cancellation retains resource
  ownership until the executor actually exits. 教務Web may commit per page.
- Navigation/extraction callbacks carry an execution identity. Old, duplicate
  and preempted callbacks cannot write or settle a newer run. Visible browsing
  preserves the interrupted caller's completion when work resumes.
- Partial success retains successful pages and per-page outcomes even when
  the job snapshot is Failed. Typed errors must not discard that payload.
- Attach state notifications before reading the current snapshot; stale reads
  cannot overwrite newer events. Detach/reconnect and dropped notifications
  have a snapshot recovery path.
- Startup reference checks repair missing quarter data despite retained hashes.
  Valid empty feeds remain distinguishable from absent materialization after
  restore. Source edits during a fetch win over stale metadata writes.
- Dates remain locale-independent: strict civil dates, offset-aware timestamps
  and the established JST academic calendar rules. Language changes cannot
  change persisted values or crash startup.
- Move existing admission rules unchanged first. Automatic school and native
  reference jobs currently have different network policies; consolidating
  code does not implicitly authorize changing those desktop behaviours.

## Rules for the consolidated code

Add these to `AGENTS.md` in phase 0.

1. **Core decides, shells render.** A conditional in TS, Swift or Kotlin that
   decides *what* the data means (due, stale, merged, valid) belongs in core.
2. **Platform capabilities are ports.** Core defines a trait; each shell
   implements it. Core never names a shell type.
3. **Native differences at compile time.** Inside core use `#[cfg(...)]`;
   otherwise put the code in the shell that needs it.
4. **Typed errors at the boundary.** No `Result<_, String>` in public core or
   FFI APIs.
5. **No panics in non-test code.** `clippy::unwrap_used`, `expect_used`,
   `panic` are denied in `isct-core` and `isct-ffi`.
6. **Shells provide wakeups; Core decides what is due.** Periodic work is an
   entry point the shell calls (`Core::tick`). Desktop drives it with an
   interval. Core may own bounded task timeouts under an explicit runtime
   lifecycle; mobile must not depend on a continuously running timer.
7. **Coarse FFI surface.** Expose services, not one function per Tauri command.
8. **Remove the replaced owner.** Once a core service is authoritative, remove
   superseded shell policy, state and queues. Temporary adapters need a stated
   removal phase, not a second permanent source of truth.

## Target structure

```text
Cargo.toml                 [workspace] members = ["backend", "crates/*"]
rust-toolchain.toml        pinned toolchain + ios/android targets
crates/isct-core/
  src/lib.rs               Core::new(ports) -> Core; service accessors
  src/ports.rs             all platform traits
  src/error.rs             CoreError and per-service errors
  src/events.rs            CoreEvent
  src/...                  moved modules
crates/isct-ocr/
  src/lib.rs               independent scan API, typed errors and unavailable backend
  models/paddle/           moved from backend/models/paddle
crates/isct-ffi/
  src/lib.rs               #[uniffi::export] facade
  tests/                   Swift and Kotlin binding smoke tests
backend/
  src/lib.rs               Tauri setup only
  src/commands/*.rs        #[tauri::command] wrappers, one file per service
  src/ports/*.rs           Tauri implementations of core ports
```

Moving to a root workspace moves `Cargo.lock` and `target/` to the repo root.
Update `.gitignore`, `package.json` scripts (`--manifest-path` keeps working),
`scripts/tauri.mjs`, CI, and the `matrix_ocr` cfg. Build-script cfg values are
crate-local: audit every consuming crate rather than assuming the core cfg
propagates to backend. Keep Tauri build setup in `backend/build.rs`.

### Ports

Signatures below are sketches, not a complete browser/FFI contract. Adjust
during implementation but keep each
trait narrow and object-safe (`Arc<dyn Port>`), with `Send + Sync`.

Introduce a coarse school-executor port before migrating `pull_service` and
`school_refresh`; the existing Controller implements it temporarily. This
breaks the migration dependency on the later browser split. Eliminate the
temporary adapter once the executor itself lives in Core.

Before implementing `SchoolBrowser`, specify navigation and extraction IDs,
load-versus-completion semantics, failed loads, correlated result delivery,
preemption, cancellation and listener disposal. The current URL-only callback
sketch below is insufficient for those guarantees. A `Send + Sync` port handle
must dispatch native view work to the shell's required thread; it does not make
a native WebView itself thread-safe.

```rust
pub trait Paths: Send + Sync {
    fn data_dir(&self) -> PathBuf;     // replaces app.path().app_data_dir()
    fn cache_dir(&self) -> PathBuf;
    fn course_storage(&self) -> Option<PathBuf>;
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);  // replaces all app.emit calls
}

pub trait SecretStore: Send + Sync {   // replaces keyring + secure-storage split
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str) -> Result<(), PlatformError>;
}

pub trait Opener: Send + Sync {        // open file / URL / reveal in Finder
    fn open_path(&self, path: &Path) -> Result<(), PlatformError>;
    fn open_url(&self, url: &Url) -> Result<(), PlatformError>;
}

/// Headless view of an authenticated school browser. The shell owns the
/// WebView, its layout and visibility; core only drives it.
#[async_trait]
pub trait SchoolBrowser: Send + Sync {
    async fn load(&self, site: SchoolSite, url: Url) -> Result<(), BrowserError>;
    async fn eval(&self, site: SchoolSite, script: &str) -> Result<serde_json::Value, BrowserError>;
    async fn cookies(&self, site: SchoolSite) -> Result<Vec<Cookie>, BrowserError>;
    async fn current_url(&self, site: SchoolSite) -> Result<Option<Url>, BrowserError>;
    fn set_listener(&self, listener: Arc<dyn SchoolBrowserListener>);
}

pub trait SchoolBrowserListener: Send + Sync {
    fn page_loaded(&self, site: SchoolSite, url: Url);
    fn download(&self, site: SchoolSite, download: DownloadEvent);
}

pub trait CloudDrive: Send + Sync { /* iCloud / Google Drive record IO */ }

/// OS Wi-Fi access for campus network setup. Core owns the policy (which
/// networks, when to connect, how to verify); shells own the OS calls
/// (CoreWLAN on macOS, NEHotspotConfiguration on iOS, Wi-Fi suggestions on
/// Android).
#[async_trait]
pub trait CampusWifi: Send + Sync {
    async fn status(&self) -> Result<WifiStatus, PlatformError>;
    async fn connect(&self, profile: &WifiProfile) -> Result<(), PlatformError>;
}
```

`require_local_main` stays in the Tauri command layer; it is a Tauri security
check, not core logic.

### Events

```rust
pub enum CoreEvent {
    WorkspaceChanged { source: ChangeSource },
    CloudSyncState(SyncState),
    PullStateChanged(Vec<PullSnapshot>),
    SchoolPullProgress(SchoolPullProgress),
    SchoolSessionChanged(SchoolSessionEvent),
    OcrProgress { done: u32, total: u32 },
}
```

The Tauri `EventSink` maps these to the existing event names and payloads so
the React side needs no change.

Keep snapshot subscription distinct from transient progress. Specify event
revision/order and subscription disposal at the boundary. Preserve the full
existing JSON payloads, including partial results, rather than serializing
only a state enum.

### Runtime and lifecycle

Core owns task handles and their domain state. Define who creates/enters the
async runtime, how blocking work is joined, and how `resume`, `suspend` and
`shutdown` settle or retain pending callers before exposing the FFI facade.
The synchronous Tauri boundary must not be copied blindly into foreign UI
threads.

Cover the scheduler's 30-second wakeup, school refresh's 1-second wakeup,
detail debounce, delayed/retry pulls and sign-in timeouts. Use timestamps for
freshness/persistence and monotonic elapsed time for bounded waits; tests must
control time. A resume rechecks current facts and due work. A suspension or
dropped foreign future must not leave an occupied lane with no owner or allow
an old callback to write into a replacement run.

The host [foreign runtime spike](verification/ffi-runtime.md) pins UniFFI
0.32.1 and exercises Swift and Kotlin callbacks, errors, cancellation and
shutdown. Its Swift binding does not propagate `Task.cancel()` into Rust:
the facade needs explicit operation IDs and cancellation, allocated before
waiting. Caller detachment and shared job cancellation remain distinct.
Non-`Result` Swift exports generate `try!`, so unwind alone is insufficient;
all fallible FFI operations must return typed errors, including disposal.
This fixture validates binding mechanics, not the final application lifecycle.

### Errors

`thiserror` enums per service, plus one umbrella:

```rust
pub enum CoreError {
    NeedsSignIn(SchoolSite),
    Network(String),
    SiteChanged { site: SchoolSite, detail: String }, // parser no longer matches
    Storage(String),
    Busy(&'static str),                               // e.g. OCR already running
    InvalidInput(String),
    Platform(String),
}
```

Tauri commands convert to the current `String` errors (so React is unchanged)
until the frontend is updated to read the typed form.

### Service facade

Group the ~90 commands into services on `Core`. Tauri commands and UniFFI
exports both call these.

| Service | Current commands (examples) |
|---|---|
| `workspace` | `load_workspace`, `put_user_record`, `delete_user_record`, `put_workspace_setting`, `reset_workspace`, `export_data`, `import_data` |
| `catalog` | `catalog_*`, `preview_plan_recurrence` |
| `school` | `pull_school`, `pull_state`, `cancel_pull`, `school_records`, `login_school`, `ensure_school_session`, credentials, `set_kyomu_guide`, `test_school_connection` |
| `school_browser` | `open_school_site`, `navigate/back/forward/reload_school_web`, `set_school_web_zoom`, `resolve_school_url` (layout commands stay in the shell) |
| `references` | `refresh_reference_subscriptions` (source selection, acquisition and cache policy are internal) |
| `course_files` | `course_storage_info`, `*_course_note*`, `open_course_*`, `clean_course_materials` |
| `workspaces` (desktop cfg) | `assignment_workspace`, `*_workspace*` |
| `cloud` | `cloud_*`, `connect/disconnect_cloud`, credential sync |
| `scheduler` | `run_scheduled_task`, `write_scheduled_export`, `tick` |
| `ocr` | `recognize_matrix_image` |
| `agent` | `agent_command` |

Desktop-only system integrations stay in `backend/` as shell code:
`school_webkit`, `set_school_web_bounds`, `hide_school_web`,
`save_file`, `open_external`.

## Phases

Use small dependency-closed commits/PRs; a phase may require several. Every
completed checkpoint must preserve desktop behaviour. After workspace setup,
run at the repo root:

```bash
cargo clippy --workspace --all-targets --locked -- -D warnings
```

```bash
cargo test --workspace --locked
```

```bash
yarn test
```

```bash
yarn test:types
```

Desktop manual smoke test: launch, sign in, pull LMS and 教務Web, open Today,
refresh subscriptions, exercise active workspace/file workflows, and run OCR
on supported desktop targets. Preserve backups and sync compatibility.

The smoke test is the part automation cannot reach. Since the test
consolidation it is recorded as a row in
[`verification/manual-checks.yaml`](verification/manual-checks.yaml) rather than
prose here: each check names a persona, states why each step exists, says where
to observe the result, and records its outcome with evidence. Run
`yarn verify:manual stale` after a change to see which checks it invalidated, and
`yarn verify:manual sheet <ID>` to get a run sheet to follow. Automated tests
live under `apps/desktop/tests/`; see
[apps/desktop/tests/README.md](../apps/desktop/tests/README.md).

Cross-target checks validate core/FFI portability to `aarch64-apple-ios`,
`aarch64-apple-ios-sim` and `aarch64-linux-android`. Existing CI already checks
Tauri iOS device and Android; extend the relevant checks rather than duplicating
them. The old mobile UI is not a behavioural acceptance gate.

Replay the same input/event traces against the baseline contract and migrated
service. Compare executed requests, write effects, emitted snapshots and caller
outcomes, not only final rendered data. Cover join/priority, full/detail,
cancel/apply races, preemption, stale callbacks, partial success, source edits,
empty caches, restore, locale changes and suspend/resume. Treat expected fixes
to inconsistent/crashing edge cases as explicit regression cases.

At each checkpoint report production LOC separately from tests/generated code,
removed modules and removed duplicate state owners. Expect a net simplification
across the completed migration; early ports/tests may temporarily add lines.

### Phase 0 — Guardrails

- Record desktop acceptance traces, dependency boundaries and code-size baseline.
- Add `rust-toolchain.toml` pinned to the toolchain verified for this repository.
- Create the root workspace with `backend` as the only member; fix paths.
- Switch every release profile to `panic = "unwind"` (decided: desktop and
  mobile alike), then test the FFI panic boundary. Do not treat `unwind` alone
  as crash recovery.
- Adapt the existing CI checks to the workspace and add missing coverage.
- Add the rules section above to `AGENTS.md`.
- Validate a minimal foreign callback/runtime/disposal boundary early, before
  committing the whole service API to an untested FFI design.

Done when: builds and tests are green from the root; no code moved yet.

### Phase 1 — Core crate and pure modules

- Create `crates/isct-core` with `ports.rs`, `events.rs`, `error.rs` (types
  only) and the `ts` feature for `ts-rs` derives.
- Move dependency-closed groups from the candidate inventory, including the
  pull reducer/coordinator, with their tests and embedded assets. Split traits
  from shell implementations first where needed. Move OCR/model paths together
  into the separate `isct-ocr` crate, not into Core.
- Give new public boundaries typed errors immediately; avoid creating more
  public String-error APIs to rewrite in phase 4.
- `backend` depends on `isct-core` and re-exports where needed. Keep
  `src/generated` output byte-identical.

Done when: moved modules have zero `tauri` imports; generated TS unchanged.

### Phase 2 — Ports replace `AppHandle`

- Implement `Paths`, `EventSink`, `SecretStore`, `Notifier`, `Opener`,
  `CloudDrive` in `backend/src/ports/`.
- Introduce `Core` holding `Arc` services; Tauri manages one `Arc<Core>`.
- Introduce the temporary school-executor port implemented by the existing
  Controller, then move `pull_service` and `school_refresh` policy into Core.
- Move the domain portions of scheduler, reference refresh, cloud sync,
  credentials, files/downloads and assignment workspaces. CloudKit/Keychain
  implementations, Tauri protocol handlers and native thread dispatch remain
  shell adapters. Replace `AppHandle` lookup with explicit service dependencies.
- Preserve the compile-time development Keychain fallback and CloudKit policy
  in the platform implementation, including the startup regression coverage.
- Core exposes due-work/lifecycle entry points; move periodic wakeups to
  `backend` and define bounded task timeout ownership as above.
- Split `backend/src/lib.rs` commands into `backend/src/commands/<service>.rs`.
- Route desktop MCP tools through the same Core service facade. Keep the MCP
  tool allowlist, authorization and audit boundary, while removing duplicate
  validation, mutation and orchestration logic from the MCP executable.
  Operations requiring the live school session or pull coordinator must reach
  the desktop's existing Core runtime through a local bridge, rather than
  creating another runtime in the MCP process. Document which headless
  operations can run independently with database revision/transaction checks.

Done when: only `backend/` imports `tauri`; event names and payloads seen by
React are unchanged (add a test that serialises each `CoreEvent` through the
Tauri sink and compares with the old JSON).

### Phase 3 — Headless school web

Largest and riskiest phase; design review before coding.

- Split `school_web_shared.rs` into core logic (site policy, login scripts,
  session checks, extraction orchestration, relays) and shell code (WebView
  creation, data store selection, bounds, visibility, native download hooks).
- Core drives the browser only through `SchoolBrowser`. Bounds, hide/show and
  zoom never enter core.
- Write fake-browser traces for the identity/preemption/cancellation contracts
  first. Move the executor into Core and remove the temporary adapter; do not
  leave parallel queues or session decisions in the shell.
- Tauri implements `SchoolBrowser` for desktop, iOS (`school_web_ios.rs`) and
  Android (`plugins/school-web`).
- Login scripts and `lms_*.js` stay as shared assets in core
  (`include_str!`), so every shell injects the same scripts.

Done when: `isct-core` contains no WebView type; sign-in, pull and downloads
work on desktop; core has trace tests for the session/extraction state machines
using a fake `SchoolBrowser`, including late and duplicate callbacks.

### Phase 4 — Typed errors and panic hygiene

- Replace `Result<_, String>` in all public core APIs with typed errors.
- Remove `unwrap`/`expect` from non-test core code; turn on the clippy denies.
- Commands map typed errors back to the strings React expects.

Done when: clippy denies are on for `isct-core` with no `allow` escapes except
justified, commented ones.

### Phase 5 — Move frontend logic into core

Order by dependency; port tests together with code (TS tests become Rust
tests with the same cases).

1. Remove superseded freshness/polling code after transferring useful test
   cases; move still-active policy such as `reminders.ts` into Core.
2. `schedule.ts`, `user-schedule.ts`, `academic-calendar.ts`, `calendar-ics.ts`,
   `declared-credits.ts`, `course-query.ts`
3. `lms-merge.ts`, `course-workflows.ts`
4. `storage/repository.ts` validation and the mutation logic in
   `store/workspace.ts` (zustand becomes a cache of core read models)
5. `school-session.ts` timing policy
6. Matrix OCR input in `isct-ocr`: `scan` takes raw image bytes, decodes and produces the
   2200 px and 600 px images itself, runs on `spawn_blocking`, reports
   `OcrProgress`. HEIC → JPEG is a shell duty (ImageIO on Apple, `ImageDecoder`
   on Android, `heic-to` on Windows desktop); document this on the API.
7. Course colour: `displayColor` (`src/core/course-colors.ts`) becomes
   `display_color` in core. Keep the FNV hash and palette byte-identical so
   existing courses keep their colours; port the TS cases as fixtures. The
   palette itself is a design token (`design/tokens/course.json`), generated
   into `schedule/course_palette.rs`.
8. Mobile read models on top of the moved schedule/reminder logic (see
   `docs/ux/mobile.md`):
   - `today(now)`: the Today phase (before/between classes, in class, after
     classes, no classes), its hero and sections, and the day status. Shells
     render it in order and do not decide what is due.
   - `glance(now)`: next class, rest of today, nearest deadlines; a small
     serialisable snapshot that widget extensions read without loading core.
   - `notification_plan(now, horizon)`: every planned notification ordered by
     priority, with time, title, body and deep-link target. Platform limits
     (iOS 64 pending) are applied by the shell, not core.

Preserve strict civil-date/timestamp parsing and JST academic semantics when
moving calendar functions. Delete obsolete TS policy and mutation paths once
Rust serves the corresponding read model or intent.

Done when: the listed TS modules are deleted or reduced to calls into core;
the UI renders the same results (existing acceptance tests pass).

### Phase 6 — FFI facade

- `crates/isct-ffi` with UniFFI proc-macros exporting `Core` services as async
  functions, records and typed errors; ports become `#[uniffi::export(with_foreign)]`
  traits.
- Build an XCFramework (iOS device + simulator) and an Android AAR/so set.
- Smoke tests: a Swift test and a Kotlin test that construct `Core` with fake
  ports, load a workspace, run one parser fixture and one OCR fixture.

Done when: both smoke tests pass in CI; OCR runs on the iOS simulator.

### Final deliverable — shared API reference and shell integration guide

Document the implemented Core facade for all four consumers: Tauri, Swift/iOS,
Kotlin/Android and desktop MCP. Build the reference from the real exported
services and records; clearly distinguish implemented contracts from remaining
migration plans. Internal helpers are not public shell API.

- Cover request/response records, typed errors, identifiers, timestamp rules,
  revision conflicts, state/event snapshots, subscriptions and recovery.
- Specify initialization, required platform ports, execution/thread ownership,
  operation IDs, cancellation, suspend/resume and draining shutdown.
- Include minimal Tauri, Swift and Kotlin call examples, plus an MCP tool-to-Core
  mapping and examples. MCP shares domain invariants with the UI while retaining
  its existing authorization and audit contract; exposing Core does not expose
  every method as an MCP tool.
- Keep examples checked against the actual signatures, link the reference from
  the repository README, and update it with public API changes.

Done when: a new shell or MCP adapter can implement the documented ports and
invoke shared services without recreating business rules or state machines.

## Risks

- **School web split (phase 3)**: most behaviour lives in timing between page
  loads and script evals. Write the fake-browser tests first.
- **`ort` on mobile**: compiles for iOS and Android today but has not been run
  on a device. Phase 6 must run OCR on the simulator, and on a device before
  any release.
- **Event payload drift**: guarded by the serialisation test in phase 2.
- **Workspace move**: `target/` relocation breaks cached paths in scripts; grep
  for `backend/target` before merging phase 0.

## Decisions

- OCR stays in an optional independent crate. Typed failures isolate normal
  scan errors from domain state; native abort isolation would require a
  separate process and is not provided by a crate boundary.
- Release builds use `panic = "unwind"` on every platform.
- Campus Wi-Fi will ship on mobile later. Split `campus_wifi.rs` in phase 2:
  policy moves to core, the macOS calls become the desktop `CampusWifi` port.
- Desktop stays Tauri + React: it depends on web components (CodeMirror,
  Milkdown, PDF.js, KaTeX) with no native equivalents. Native desktop shells
  (SwiftUI, .NET) are out of scope.
