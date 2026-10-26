# Core consolidation

The contracts that keep application logic in shared Rust services, which the
desktop Tauri shell and future native mobile shells (SwiftUI, Jetpack Compose)
call through one interface. Read `AGENTS.md` first; every rule there applies.
The migration this file originally planned is complete; its acceptance record
is [`verification/core-consolidation.md`](verification/core-consolidation.md).

## Goal

```text
crates/isct-core   domain logic and state: parsing, acquisition, sync, scheduling, storage
crates/isct-ocr    optional matrix recognition, independent of core and platform frameworks
crates/isct-ffi    UniFFI facade over isct-core for Swift and Kotlin
apps/desktop/src-tauri  Tauri desktop shell: commands, platform ports, lifecycle
apps/desktop/src        React UI: renders state from core, sends user intents to core
apps/ios, apps/android  Native mobile shells over the Core FFI (crates/isct-ffi)
```

- `isct-core` has no dependency on `tauri`, `wry`, `uniffi` or any UI crate.
- OCR is an independently assembled capability. Core does not depend on ONNX,
  image processing or model assets. An unsupported or disabled OCR backend
  returns a typed unavailable result without disabling other app services.
- Shells render, forward intents, and implement platform ports. They contain no
  domain decisions.
- Desktop behaviour is the acceptance baseline after every change. Mobile UI
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
  `apps/desktop/src/generated` stays the desktop's type source.

## State ownership and compatibility contracts

School sync (hints in, one snapshot out) is described in [`school-refresh.md`](school-refresh.md).

| Layer               | Owner       | Responsibilities                                                                |
| ------------------- | ----------- | ------------------------------------------------------------------------------- |
| Intent and policy   | Core        | launch checks, TTL, academic year, debounce, admission policy                   |
| Pull coordination   | Core        | exact-key joining, priority, lanes, cancellation, retry, snapshots              |
| Domain executors    | Core        | reference materialization, school session/extraction orchestration, persistence |
| Platform mechanisms | Shell ports | WebView operations, secure storage, CloudKit, lifecycle facts, OS wakeups       |
| Presentation        | Shell       | rendering, layout, localization, user intents, cached core read models          |

Keep domain-specific state machines for school sessions/navigation and cloud
sync where they describe different resources and outcomes. They belong in
Core and communicate through explicit contracts; do not force every operation
into a single giant pull machine. The pull reducer has 11 states and 14 events
(154 defined pairs), but its queue, waiters, run IDs and resource ownership are
extended runtime state. Total transitions alone do not prove runtime safety.
`Yielding` is currently reserved; real visible-browser preemption lives in the
school executor. Moving it must be explicit and tested, not inferred from the
presence of that enum variant.

Preserve these contracts:

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

1. **Core decides, shells render.** A conditional in TS, Swift or Kotlin that
   decides _what_ the data means (due, stale, merged, valid) belongs in core.
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
   superseded shell policy, state and queues. A temporary adapter needs a
   stated removal point, not a second permanent source of truth.

## Target structure

```text
Cargo.toml                 [workspace] desktop shell + crates/*
rust-toolchain.toml        pinned toolchain + ios/android targets
crates/isct-core/
  src/lib.rs               Core::new(ports) -> Core; service accessors
  src/ports.rs             all platform traits
  src/events.rs            CoreEvent
  src/...                  domain services; tests in src/tests/
crates/isct-ocr/
  src/lib.rs               independent scan API, typed errors and unavailable backend
  models/paddle/           recognition model
crates/isct-ffi/
  src/lib.rs               #[uniffi::export] facade
  tests/                   Swift and Kotlin binding smoke tests
apps/desktop/src-tauri/
  src/lib.rs               Tauri setup only
  src/commands/*.rs        #[tauri::command] wrappers, one file per service
  src/core_ports.rs        Tauri implementations of core ports
```

Build-script cfg values are crate-local: audit every consuming crate rather
than assuming a core cfg propagates to the shell.

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

| Service                    | Current commands (examples)                                                                                                                    |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace`                | `load_workspace`, `put_user_record`, `delete_user_record`, `put_workspace_setting`, `export_data`, `import_data`                               |
| `reset`                    | `reset_caches`, `reset_caches_preview` (`Core::reset`: one lease, admission gate, generation)                                                  |
| `catalog`                  | `catalog_*`, `preview_plan_recurrence`                                                                                                         |
| `sync`                     | `sync_hint`, `sync_snapshot` (the planner decides every school and reference read)                                                             |
| `school`                   | `school_records`, `connect_school`, credentials, `set_kyomu_guide`                                                                             |
| `school_browser`           | `open_school_site`, `navigate/back/forward/reload_school_web`, `set_school_web_zoom`, `resolve_school_url` (layout commands stay in the shell) |
| `course_files`             | `course_storage_info`, `*_course_note*`, `open_course_*`, `clean_course_materials`                                                             |
| `workspaces` (desktop cfg) | `assignment_workspace`, `*_workspace*`                                                                                                         |
| `cloud`                    | `cloud_*`, `connect/disconnect_cloud`, credential sync                                                                                         |
| `scheduler`                | `run_scheduled_task`, `write_scheduled_export`, `tick`                                                                                         |
| `ocr`                      | `recognize_matrix_image`                                                                                                                       |
| `agent`                    | `agent_command`                                                                                                                                |

Desktop-only system integrations stay in `apps/desktop/src-tauri` as shell code:
`school_webkit`, `set_school_web_bounds`, `hide_school_web`,
`save_file`, `open_external`.

## Decisions

- OCR stays in an optional independent crate. Typed failures isolate normal
  scan errors from domain state; native abort isolation would require a
  separate process and is not provided by a crate boundary.
- Release builds use `panic = "unwind"` on every platform.
- Campus Wi-Fi will ship on mobile later. When it does, its policy moves to
  core and the macOS calls in `campus_wifi.rs` become the desktop port.
- Desktop stays Tauri + React: it depends on web components (CodeMirror,
  Milkdown, PDF.js, KaTeX) with no native equivalents. Native desktop shells
  (SwiftUI, .NET) are out of scope.
