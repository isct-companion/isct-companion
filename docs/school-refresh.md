# School sync

Every automatic or requested network read of school and reference data goes
through one Core planner (`crates/isct-core/src/sync_engine`). Shells send
**hints** and render one **snapshot**; they never decide what is due, never
estimate progress and never keep a pull queue of their own. The binding types
are in `.claude/sync-engine/INTERFACES-CP3.md`; the design rationale is
`design.html` §06 (when to update) and §09 (progress and status).

Outside this mechanism there are only course files the person opens, the app's
own update check and cloud sync, each with its own service. The Scheduler keeps
only tasks the person created (for example scheduled exports).

## Hints

| Hint | Sent by |
| --- | --- |
| `wake` | Desktop: the one-second wake loop in `src-tauri/src/core_ports.rs`. Mobile: `Session::wake()` forwards it before running reminders, cloud sync and scheduled tasks. The planner evaluates at most every 30 s on a wake. |
| `foreground` / `background` | Desktop: main-window focus (`lib.rs`) and webview visibility (`useCoreLifecycle.ts`). Mobile: scene/activity lifecycle. |
| `suspend` / `resume` | Desktop: `RunEvent::Resumed`. Mobile: `Session::suspend()` / `resume()` send them. |
| `network` | Desktop: `navigator.onLine` and the Network Information API (`networkHint()` in `platform/sync.ts`); an unreported link is `unknown`. Mobile: `NWPathMonitor` / `ConnectivityManager`, plus `SessionConfig.initialNetwork`. |
| `power-saver` | Mobile: Low Power Mode / battery saver. |
| `signed-in` | Core sends the equivalent itself when a school session becomes active; mobile shells also send it after saving credentials. |
| `open { view }` | A page became visible: Today, Assignments (Lecture Desk without a course), Timetable, Course (a lecture with an LMS record), Finder (the course search for a year), Offering (a syllabus detail panel), Grades (the workstation timetable's academic summary). Desktop sends them from `App.tsx` through `useOpenHint`. Core reads that page's keys at the 15-minute open threshold, at Visible/Interactive priority. |
| `refresh { scope }` | Refresh buttons: the footer and the compact sheet (`everything`), the timetable (`site: science-lms`), the GPA summary (`site: kyomu`), a lecture (`course`), the data-update and calendar settings (`references`), and each failed dataset's Retry. Manual refresh ignores TTL and backoff but keeps single-flight; repeats within 30 s join the first. |

Commands: Tauri `sync_hint(hint)` and `sync_snapshot()`
(`src-tauri/src/commands/sync.rs`); FFI `Session::sync_hint`,
`Session::sync_snapshot` and `decode_sync_snapshot(payload)`.

## Snapshot

`SyncSnapshot { generation, resetting, runs, sources }` arrives as the
`sync-snapshot` event. A shell listens first, then reads `sync_snapshot()`
once; a late read never replaces a newer event (desktop: `onSyncSnapshot` in
`desktop-bridge.ts`; mobile: generation check). After a lagged FFI stream
(`SnapshotRequired`) the shell reads it again.

How the desktop renders it (`platform/sync.ts`, `app/SchoolStatusLine.tsx`,
`app/sync-text.ts`):

- **One progress line**, in the workstation footer and as a small pill at the
  top of the compact layout. The highest-priority working run wins. A
  determinate bar appears only when the run's `total` is known; a run of
  unknown size shows its label; background-only work is a small spinner and
  "Updating in the background". Held work names its reason (offline, waiting
  for Wi-Fi, power saver, in front only, retry time, host paused, sign-in,
  resetting).
- **Idle** shows each source's age from `SourceView.lastUpdated`.
- **Failures** are listed per dataset with their reason and "retries at …",
  each with a Retry that sends `refresh` for that run's scope (`retryScope`).
- **In-page progress**: the lecture page shows "Showing data from N ago ·
  updating this course k / n" from the runs whose scope is that LMS course;
  the page keeps rendering local rows until Core commits the course. The
  timetable, GPA summary and course panels show the same counts on their
  refresh buttons.
- Labels are i18n keys from Core. A key no locale knows yet falls back to the
  dataset's name, so an identifier never reaches the screen.

## Course search

Search is local only: `finder_query` over the stored catalog and its FTS
index. There is no remote search path, page cursor or result cache in the
shell. While the selected year has no stored offerings and its
`catalog.official` run is in the snapshot, the finder says "Downloading the
{year} course catalog"; when the run leaves the snapshot the query is re-read
and the results switch at once.

## Verification

Core's planner, leases, budgets and TTLs are tested in `isct-core`. The
frontend tests cover the bridge contract (`unit/workspace-store`: hint
argument shape, listen-then-read ordering, shape validation), which row of a
snapshot a surface shows (`unit/workspace-store`, "sync snapshot surfaces"),
the footer wiring and hints (`e2e/app-shell`) and the local-only finder with
its download notice (`e2e/planner`). The FFI projection of the renamed sync
types is tested in `crates/isct-ffi/src/tests/crate.rs`; the Tauri event wire
name in `src-tauri/src/tests/core_ports.rs`. How the line, pill and failures
look is a manual check (`DESKTOP-SYNC-STATUS`).
