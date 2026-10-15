# Shared Core API and shell integration

This reference describes the implemented Rust boundary. Tauri and
the desktop MCP adapter use it now. Swift/Kotlin bindings remain Phase 6 work;
these names are not promises that generated foreign bindings already exist.

## Shared protocol and transport

Desktop and future native mobile shells share Core's domain contract: the same
records, intents, read models, validation, JST date rules, revision conflicts and
operation ownership. Their transport adapters carry that contract:

| Consumer         | Transport / entry                           | Status                                                       |
| ---------------- | ------------------------------------------- | ------------------------------------------------------------ |
| Desktop UI       | Tauri invoke → the app's single `Arc<Core>` | Implemented                                                  |
| Desktop MCP      | Streamable HTTP / JSON-RPC → `Core.agent`   | Implemented, authorized subset of domain operations          |
| Swift / iOS      | Native bindings → Core and platform ports   | Phase 6 deferred; application bindings are not generated yet |
| Kotlin / Android | Native bindings → Core and platform ports   | Phase 6 deferred; application bindings are not generated yet |

The shared contract is defined by Rust, rather than a universal HTTP endpoint.
MCP has its own tool envelopes, permission and audit contract. Mobile bindings
must expose the same business semantics without introducing another state
machine; their concrete foreign signatures will be specified when implemented.
The existing foreign-runtime spike is a fixture, not an application binding.

Sources of truth:

- [Facade](../crates/isct-core/src/facade.rs): service entry points and read-model errors.
- [Workspace](../crates/isct-core/src/workspace.rs): mutation/snapshot envelopes and all intents.
- [Schedule](../crates/isct-core/src/schedule/mod.rs): query/result variants and read models.
- [Finder](../crates/isct-core/src/course_query/mod.rs): finder request/action/result records.
- [Persistence contracts](../crates/isct-core/src/contracts.rs): row and settings schemas.
- [Generated TypeScript](../apps/desktop/src/generated/persistence.ts): workspace and schedule declarations generated from Rust. Finder declarations currently reside in Rust and the thin shell adapter.

JSON records use camelCase fields. Tagged requests use `kind`; query results use
`{kind,value}`. IDs are opaque strings; term IDs are `YYYY-nQ`; workspace revision
is an opaque fingerprint, not a row revision. Row `rev` and `schemaVersion`
belong to persistence. There is currently no negotiated public API version;
`schemaVersion` is not an HTTP or FFI protocol version. API changes must update
Rust contracts, generated declarations, adapters, parity checks and this reference.
JSON `unknown` fields in TypeScript represent Rust `serde_json::Value` payloads;
Core validates their domain shape rather than granting arbitrary record writes.

## Desktop command contract

These are local Tauri commands, restricted to the trusted main WebView. Parameter
objects below are passed to `invoke`; read commands return values directly,
without an additional `{data:...}` wrapper.

| Command              | Parameters                      | Result                                       | Rust service                          |
| -------------------- | ------------------------------- | -------------------------------------------- | ------------------------------------- |
| `workspace_snapshot` | `{}`                            | `WorkspaceSnapshot`                          | `Core.workspace.snapshot()`           |
| `mutate_workspace`   | `{mutation: WorkspaceMutation}` | Committed `WorkspaceSnapshot`                | `Core.workspace.mutate(mutation)`     |
| `schedule_query`     | `{queries: WorkspaceQuery[]}`   | `{revision, values: WorkspaceQueryResult[]}` | `Core.schedule_query(queries)`        |
| `finder_query`       | `{request: FinderRequest}`      | `FinderView`                                 | `Core.finder_query(request)`          |
| `lms_merge_preview`  | `{scope, term}`                 | Review groups and missing detail IDs         | `Core.lms_merge_preview(scope, term)` |

`WorkspaceSnapshot` fields are `revision`, `workspace` and `schoolRecords`.
`WorkspaceMutation` fields are `expectedRevision` and `intent`. A mutation returns
the full committed snapshot; it never returns an optimistic local patch.

```json
{
  "expectedRevision": "<revision from workspace_snapshot>",
  "intent": { "kind": "patchSettings", "patch": { "appearance": "dark" } }
}
```

Intent groups (the generated `WorkspaceIntent` is the complete field schema):

| Group                  | `kind` values                                                                                                                                                     |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Preferences / terms    | `patchSettings`, `setAdmissionYear`, `editAcademicCalendar`, `selectTerm`, `addTerm`, `removeTerm`                                                                |
| Courses / review       | `saveCourse`, `deleteCourseBatch`, `editCourseBatch`, `setCourseOnTimetable`, `addCatalogCourses`, `undoCatalogCourses`, `applyLmsMerge`, `markTimetableReviewed` |
| Events / plans         | `saveEvent`, `deleteEvent`, `savePlan`, `deletePlan`                                                                                                              |
| Calendar / sources     | `saveCalendarException`, `deleteCalendarException`, `saveSubscription`, `setSubscription`, `deleteSubscription`                                                   |
| Overrides / automation | `saveScheduleOverride`, `deleteScheduleOverride`, `saveScheduledTask`, `deleteScheduledTask`, `recordScheduledTaskRun`, `saveIntensiveSchedule`                   |
| Assignment / lesson    | `setAssignmentMark`, `recordLesson`                                                                                                                               |

Save intents take a `draft`; optional `existingId` selects update rather than
creation. Specialized intents have their own fields; callers must use their
schema instead of treating the intent table as a generic CRUD API. Core assigns
identity/revision bookkeeping and enforces constraints in the transaction.

Query groups (all requests are `WorkspaceQuery`):

| Group                         | `kind` values                                                                                                                           |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| Workspace / schedule          | `workspaceOverview`, `planningWeek`, `scheduleView`, `dayAgenda`, `scheduledCourses`, `courseTimeline`, `taskView`                      |
| Calendar / profile            | `subscriptionViews`, `academicYears`, `academicYear`, `calendarState`, `quarterRange`, `validateAcademicProfile`, `academicTermForDate` |
| Editor / export               | `planCivilStart`, `nextPlanOccurrences`, `intensiveDays`, `declaredCredits`, `displayColor`, `quarterIcs`                               |
| Native glance / notifications | `glance`, `notificationPlan`                                                                                                            |

Batch queries contain at most 64 requests and return one revision plus ordered
results. Result kinds equal request kinds except `validateAcademicProfile` →
`academicProfile` and `academicTermForDate` → `academicTerm`. Any query failure
rejects the batch. Optional serde-default fields and omitted result fields are
defined in Rust; nullable and optional are not interchangeable for every record.

`FinderRequest` has `year`, `term`, `keys`, `input`, `remoteIds` and `action`.
Keys/input default empty; remote IDs and action default absent. Actions are
`{kind:"commit",key}`, `{kind:"toggleTime",key,append}` and `{kind:"paste",text}`.
`FinderView` returns normalized keys, query plan, suggestions, rows, clashes,
replacement candidates, credit totals, active slots, paste results and missing
detail IDs. These actions prepare a view; a workspace intent performs writes.

## Minimal desktop call sequence

This TypeScript example uses the generated contract (paths assume a file in
`docs/`). Use a serialized intent dispatcher in the application, as implemented
by [the snapshot cache](../apps/desktop/src/store/workspace.ts).

```ts
import { invoke } from "@tauri-apps/api/core";
import type {
  WorkspaceMutation,
  WorkspaceQuery,
  WorkspaceQueryResult,
  WorkspaceSnapshot,
} from "../apps/desktop/src/generated/persistence";

type ReadModels = { revision: string; values: WorkspaceQueryResult[] };

export async function updateAppearanceAndReadToday() {
  let snapshot = await invoke<WorkspaceSnapshot>("workspace_snapshot");
  const mutation: WorkspaceMutation = {
    expectedRevision: snapshot.revision,
    intent: { kind: "patchSettings", patch: { appearance: "dark" } },
  };
  snapshot = await invoke<WorkspaceSnapshot>("mutate_workspace", { mutation });
  const queries: WorkspaceQuery[] = [
    { kind: "glance", now: new Date().toISOString() },
  ];
  const view = await invoke<ReadModels>("schedule_query", { queries });
  return { snapshot, view };
}
```

On mutation failure, retain the editor, reread the snapshot and ask for explicit
retry. Do not automatically replay a create/save. A read model carries its own
revision; invalidate disposable views after workspace/school changes and discard
responses for superseded requests. A mutation can commit before another writer
changes state, so a subsequent read's revision need not equal the returned one.

Native shells follow this same sequence after bindings exist: initialize one
Core and platform ports, subscribe then recover snapshots, render queries,
forward intents with the observed revision, and drain on shutdown. This reference
does not invent Swift/Kotlin methods for bindings that have not shipped.

## Ownership

`Core::new(DataStore, Arc<SchoolAuthService>, PullNetwork)` constructs one
application owner containing `store`, `auth`, `agent`, `workspace`, `reminders` and `Arc<CoreRuntime>`.
The desktop manages one `Arc<Core>`. Its compatibility `DataStore` and auth state
entries are clones of those same services, not additional coordinators.

`Core::services(events, executor, exporter)` creates operation-scoped ports for
school, references and scheduling. `Core::school_records()` returns the existing
school read model. `Core::attach_school_browser` installs the single Core-owned
login/session/extraction engine over a native `SchoolBrowser` port. Its callback
identity and lifecycle contracts are documented in
[Core school browser](core-school-browser.md).

A native shell supplies paths, credential storage, WebViews, notifications,
file dialogs, openers and cloud transport. Native window bounds and cookie-store
objects stay in the shell. Core owns domain decisions, validation, persistence
and task state. Adapters map typed errors to existing Tauri string responses.

`ports::EventSink` is the single event path. Every `CoreEvent` (workspace,
school login/session/progress/ingest/web/download, pull state and cloud sync
state) carries its UI event name and JSON payload; the desktop sink forwards it
to the main webview and logs delivery failures. Events are hints: each one has
a snapshot command the UI reads after subscribing.

## Service map

| Service                     | Rust entry points                                                                                                                          | Shell responsibility                                           |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------- |
| Workspace                   | `Core::workspace.snapshot/mutate`                                                                                                          | Render snapshots; forward typed intents with expected revision |
| Schedule / calendar         | `Core::schedule_query`, `schedule::{glance,notification_plan}`                                                                             | Supply explicit dates/times; render views and export bytes     |
| Course finder / LMS review  | `Core::finder_query`, `Core::lms_merge_preview`                                                                                            | Input, selection, progress and review confirmation             |
| Reminders                   | `Core::reminder_tick`, `Core::import_legacy_reminders`                                                                                     | Wakeups, migration bytes and `NotificationSink`                |
| School                      | `Core::school_browser()`, `Core::services(...).school`, `Core::school_records`                                                             | Native views, presentation, callbacks and downloads            |
| Pull state                  | `CoreRuntime::{pull_snapshots,subscribe_pulls,cancel_pull,refresh_event}`                                                                  | Forward intents and publish snapshots                          |
| References                  | `Core::services(...).references`                                                                                                           | Supply lifecycle/network facts                                 |
| Scheduler                   | `Core::runtime`: wake, run_scheduled, suspend, resume                                                                                      | Periodic wakeups and scheduled exporter port                   |
| Catalog                     | `catalog::{offerings,finder_offerings,catalog_query,catalog_sync_offering,catalog_sync_offerings,catalog_sync_department,syllabus_course}` | Call through the runtime's owned operation scope               |
| Downloads                   | `Core::runtime.downloads`, `course_materials::CourseMaterials`                                                                             | Cookie acquisition, native URL handlers and opening files      |
| Notes / backups / cloud     | `notes::NoteStore`, `BackupService`, `CloudSync`                                                                                           | Selected root, dialogs, transport and events                   |
| School credentials          | `Core::auth` / `SchoolAuthService`                                                                                                         | `SecretVault`, native login surface                            |
| Encrypted credential backup | `cloud_credentials::{validate_password,encrypt,decrypt,supports_provider}`                                                                 | Fetch/store encrypted bytes; vault access                      |
| Assignment workspaces       | `workspaces::Workspaces`                                                                                                                   | Selected root, Finder/editor opening; desktop feature only     |
| Agent operations            | `Core::agent.call(name,args,AgentPolicy)`                                                                                                  | MCP schema/HTTP transport and local authentication             |
| OCR                         | Separate `isct-ocr` crate                                                                                                                  | Supported platform setup and progress delivery                 |

Catalog caching preserves the bilingual freshness threshold and stored detail
content. Batch fetches are ordered futures held by the calling operation; no
untracked per-offering tasks survive cancellation. The desktop runs that operation
through `CoreRuntime::run`, so dropping a command receiver does not discard
admitted work. The underlying syllabus transport still owns request pacing.

## Workspace transactions and read models

`Core::workspace.snapshot()` returns
`{revision, contentRevision, workspace, schoolRecords}`, all assembled from one
SQLite read transaction. `revision` is an opaque SHA-256 fingerprint of the
user-owned rows and settings intents are checked against; school, catalog and
reference caches and Core bookkeeping (scheduler run fields, subscription fetch
state, the subscription poll clock) are excluded, so a background ingest never
turns a person's edit into a conflict. Intents keep that bookkeeping from the
stored row. `contentRevision` fingerprints the whole snapshot; read-model caches
key on it.

Send `WorkspaceMutation { expected_revision, intent }` to
`Core::workspace.mutate`. Core starts an IMMEDIATE transaction, rechecks the
revision, validates and applies the whole intent, and returns the committed
snapshot. Course replacement, LMS review, plan/event edits, subscriptions,
automations, lesson records and assignment marks use this path. IDs, timestamps,
row revisions and derived changes belong to Core. Shells keep editable form
values and a disposable snapshot cache.

Settings forms send `replaceSettings` with the complete settings object; a key
the form cleared stays cleared. Core validates every enumerated preference
(locale, appearance, density, text size, content language, notification
permission, cloud provider/conflict) and the admission year (2000–9999) on every
settings path, including `setAdmissionYear`, the calendar editor and
`DataStore::put_workspace_setting`, and keeps the fields it owns (subscription
poll clock, agent write history). Shells keep only their option lists.

A stale revision returns `WorkspaceError::Conflict`; reread before retrying or
asking the person to review changed data. Invalid input returns `InvalidIntent`,
I/O failure returns `Storage`. No partial intent is published. The desktop
publishes its returned snapshot and signals cloud work only after commit.
Backup import/reset and school import keep their existing specialized services.

Desktop uses `workspace_snapshot` for workspace and school rows together. The
retired standalone `school_records` Tauri command is removed; the public
`Core::school_records` convenience API remains available to service consumers.
Recurrence preview uses `schedule_query` with `nextPlanOccurrences`; the old
`preview_plan_recurrence` Tauri command is removed.

`Core::schedule_query(Vec<WorkspaceQuery>)` evaluates at most 64 queries against
one workspace/school snapshot and returns `{revision, values}` in request order.
Each result is tagged `{kind,value}`. Use a page-sized request rather than one
request per timetable cell:

```json
[
  { "kind": "workspaceOverview", "term": "" },
  {
    "kind": "scheduleView",
    "term": "2026-3Q",
    "from": "2026-10-12",
    "to": "2026-10-18",
    "includePersonal": true
  },
  { "kind": "glance", "now": "2026-10-14T08:00:00+09:00" }
]
```

The empty overview term asks Core to choose the planning quarter. Its result
includes the today/planning terms, initial finder conditions, timetable IDs,
credits, available terms and stable course colors. Schedule views contain dated
agendas, calendar notices, important dates and cancelled-class context. Other
query variants cover the planning week, academic-year calendar, course timeline,
editor previews, profile validation, task grouping and ICS export. `WorkspaceQuery` in the
generated TypeScript contract is the complete request list.

Dates are strict `YYYY-MM-DD`; clock/wakeup timestamps require an ISO offset.
Plan inputs also accept ISO civil dates and minute/second date-times for editor
and stored-data compatibility, interpreted in JST. Core canonicalizes new plan
writes to RFC 3339 with an explicit offset. RRULE `UNTIL` accepts a date
(through the end of that JST day), a floating date-time (JST) or a UTC `...Z`
value; Core converts it to UTC before evaluation, so nothing depends on the
machine's zone. Every local writer, including MCP, stores only plans whose
start and recurrence evaluate, and schedule overrides whose periods name a class
slot (`InvalidIntent` / MCP `InvalidInput` otherwise). Desktop and MCP share this parser,
recurrence and civil-date exclusions. A stored plan or override that still cannot
be evaluated (for example synced from elsewhere) is skipped, or left unapplied,
without failing the other queries; `scheduleView.problems` lists it for the
person to repair. Academic dates use JST independently of
the device language/timezone. Invalid inputs return
`ScheduleError` through `ReadModelError`; shells display the error without
substituting an independent calendar calculation. The frontend discards responses
for superseded requests and invalidates cached views when the revision changes.

`Core::finder_query(FinderRequest)` owns condition parsing, normalization,
suggestions, remote query planning, local filtering, credit/conflict projections,
replacement candidates and multi-code paste. `commit`, `toggleTime` and `paste`
are read-only finder actions; changing the timetable requires a workspace intent.
`remoteIds = null` uses stored offerings; an empty list represents a pending
remote search. The shell executes the returned syllabus plan and reports progress.

`Core::lms_merge_preview(scope, term)` returns review rows, conflict groups and
missing syllabus detail IDs. The shell fetches those details, requests the final
preview, then forwards the person's choices as `ApplyLmsMerge`. Core recomputes
the same diff from the same catalog rows and applies the chosen changes in its
transaction: chosen LMS courses join as `lms-member` memberships (also in their
quarters outside the reviewed scope), LMS-sourced courses take syllabus names,
URL and credits, chosen common courses take the LMS times with the syllabus
room, unchecked local courses leave that quarter, and LMS assignments and
materials are linked to the matched course.

A course belongs to every quarter it is placed in: its own `termId` plus each
quarter with a live membership or plan item, so a 3Q–4Q course is one course
with a placement in each quarter. Timetables, Today, reminders, ICS, finder
busy slots, task grouping and the course timeline use those placements.
`AddCatalogCourses` plans an offering into each of its quarters when added in
one of them (otherwise into the quarter on screen) and replaces only local
drafts that clash in that quarter. `UndoCatalogCourses` reverses exactly the
last add of the session: it removes the plan items that add placed and brings
back the ones it replaced. `SetCourseOnTimetable` off and `DeleteCourseBatch`
remove a course's placements in every quarter, whatever added them. `catalog::syllabus_course`
returns the localized editor projection, including period times and rooms.

`taskView(now)` returns deadline presentation, the IDs Core treats as
submitted, nearest assignment per course, and per-term
pending/pinned/overdue/this-week/later and lecture groups as ordered IDs. Today,
Lecture Desk and course readers render those IDs and labels; they do not apply
their own submitted rule. Shells pass `now` from the shared minute clock
(`apps/desktop/src/platform/use-clock.ts`), so pages issuing the same query within a minute
share one request. Malformed
or locale-formatted timestamps are excluded from deadline calculations; they
cannot reach an `Intl` formatter as an invalid date.

## Glance and notification delivery

`glance(workspace, now)` returns the academic term/date, today's agenda, next
occurrence, remaining occurrences and nearest open deadlines. Native widget
extensions may render a saved serialized snapshot without loading the runtime;
the shell owns refreshing that saved copy.

`notification_plan(workspace, now, horizon, locale)` produces a pure plan with
stable keys, times, titles, bodies, priorities and deep links. It does not mark
notifications delivered. A native shell applies its platform queue limits.
Desktop and future native shells use the same schedule and reminder rules.
`notification::NotificationContent` owns the title/body size limits for both
planned reminders and shell-requested notifications.

Desktop calls `Core::reminder_tick(sink, now)` from its existing wake loop.
Core owns the 30-second gate, five-minute grace window, three-day fired-log
retention, delivery deduplication and task shutdown. `NotificationSink::deliver`
returns a typed delivery error; failed deliveries release their claim for retry.
The device-local log is written atomically with mode 0600 beside the database
and is excluded from backup/cloud data.

Before the first delivery, the Tauri shell forwards the old
`isct-reminders-fired` localStorage value to `Core::import_legacy_reminders`.
Core merges it once and persists completion. The shell deletes the old value
only after success. Delivery waits for this migration barrier, preventing a
startup tick from racing migration. The wait lasts at most one minute after
the reminder service starts; then Core creates an empty log and delivers, so a
shell without a legacy log need not call the import. A later import still
merges its entries.
Shutdown closes admission and drains reminder tasks.

## OCR input boundary

`isct_ocr::MatrixOcrService::scan(image_bytes, progress)` accepts one encoded
image. The independent OCR crate bounds input/decode allocation, applies image
orientation, composites alpha over white and creates the 2200px and 600px
images. Its callback receives `OcrProgress { completed, total }`; returning false
cancels with `MatrixOcrError::Cancelled`.

Run OCR on a blocking worker. The shell handles HEIC-to-JPEG when its codec
requires it (the desktop webview uses `heic-to`) and forwards progress. Tauri
sends one bounded base64 image because its command/channel transport is JSON;
it checks the payload with `isct_ocr::check_base64_image_len` (derived from
`MAX_IMAGE_BYTES`) before decoding.
Native byte bindings need no base64 layer. Without the inference feature,
including unsupported targets, the same API returns `Unavailable` cleanly.
See [isct-ocr](../crates/isct-ocr/README.md) for limits and supported formats.

## Assignment workspace contract

Enable Core's `desktop-workspaces` feature on desktop targets. `Workspaces::new`
takes the selected root. `find`, `create`, `workspace_path`, `set_submission`,
`prepare`, `delete` and `clean` retain the existing directory/metadata format and
camelCase wire records. `prepare` returns `PreparedSubmission` with a folder or
archive target; the shell opens it. Core never submits school assignments.

These are synchronous filesystem APIs. Native commands execute them on blocking
workers inside `CoreRuntime::run`. An admitted operation retains its captured
root until completion; changing the root affects subsequent operations. It does
not move existing student files. Notes have the stricter stale-handle contract
described in [notes and backup APIs](core-backup-sync.md).

## MCP and desktop lifetime

While Settings → AI agent access allows local agents, desktop runs one
standard Streamable HTTP MCP server at `http://127.0.0.1:47832/mcp`, backed by
the official Rust MCP SDK. The listener and token exist only while access is
on; the wake loop starts, stops and retries the listener to match the stored
setting. Settings → AI
agent access provides the URL and a copy button for the Bearer access token.
There is no separate CLI, stdio adapter, bridge descriptor or headless database
mode. The app must stay open. All 17 tools call the existing `Core.agent`; no
second Core or school runtime is started.

The token lives in the app's local data directory with Unix mode 0600, outside
the database, cloud snapshots and backup exports. It persists across app restarts
so client configuration remains usable. The adapter checks Bearer authentication,
Host and Origin, bounds HTTP work, and lets Core recheck agent permission for
every tool call. Binding failure is reported by Settings without crashing
desktop and is retried every five seconds. Storage and compatibility errors
reach clients as a generic message and are logged locally.

Concurrent plan additions converge on one stable identity. Writes recheck
permission and commit mutations and audit together in one SQLite transaction.
Successful writes emit the existing workspace change event and notify the same
cloud service. Accepted work remains owned after a client disconnect; shutdown
stops HTTP admission and drains work before shutting down Core.

A broken connection can leave a caller uncertain whether a write committed;
clients must query before retrying non-idempotent create operations. The transport
does not automatically replay writes or claim exactly-once delivery.

No MCP tool added by Phase 2 contacts school systems or exposes credentials,
cookies or session data. A future explicitly authorized school tool must use
`Core.runtime`'s existing coordinator through this same service.

## MCP mapping and envelope

Every tool is dispatched through `Core.agent.call(name, args, AgentPolicy)`.
[Agent access](agent-access.md) describes client setup and tool parameters;
`tools/list` returns the current JSON schemas from
[the HTTP adapter](../apps/desktop/src-tauri/src/agent_server.rs).

| Tools                                                         | Shared Core capability                                                         |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `today`, `schedule`, `courses`                                | Shared schedule/calendar/plan rules and local workspace                        |
| `course`, `search_catalog`, `recommended`                     | Local course/catalog read models and admission/department policy               |
| `registration`, `grades`, `gpa`, `requirements`               | Imported school records; no school fetch                                       |
| `plan_add`, `plan_remove`                                     | Local planning mutations using the versioned store                             |
| `plan_event_create`, `plan_event_update`, `plan_event_delete` | Personal plans, shared timestamp/recurrence validation and transactional audit |
| `hide_assignment`, `restore_assignment`                       | Local assignment marks and transactional audit                                 |

After the MCP initialization handshake, a read call has the JSON-RPC envelope:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "schedule",
    "arguments": { "from": "2026-10-14", "to": "2026-10-14" }
  }
}
```

Success returns MCP `content` with JSON text; object results also include
`structuredContent`. Tool failures set `isError` with diagnostic text. Transport
authentication/Origin errors are HTTP errors. MCP writes have their own accepted
arguments and audit/permission checks, rather than accepting arbitrary
`WorkspaceMutation` objects or the UI's `expectedRevision` envelope.

## Shell integration checklist

1. Construct one Core over the app data store and vault; enter a host-owned
   runtime for async services. Keep native handles in platform adapters.
2. Install browser/event/notification/file ports where supported. Correlate
   operation and browser callbacks using the identities in their service contract.
3. Register subscriptions before recovering snapshots; preserve revisions and
   ignore stale callbacks/responses. Recover a snapshot after reconnect/resume.
4. Supply lifecycle/network facts and clock wakeups; Core decides due work,
   admission, state transitions and commits. Ports return typed failures.
5. Stop native admission/wakeups, detach subscriptions and await Core shutdown.
   Caller detachment and explicit domain cancellation are separate operations.

The [school-browser](core-school-browser.md), [downloads](core-downloads.md)
and [notes/backup/cloud](core-backup-sync.md) references define their concrete
ports, operation IDs, callbacks and recovery. The future foreign binding must
preserve these contracts; a transport wrapper cannot weaken commit ownership.

## Shutdown and remaining boundaries

Signal lifecycle facts through the existing runtime. Stop native HTTP/wakeup
admission, then await `Core::shutdown()` (or the same runtime's shutdown) and all
native owned tasks. Never abort an in-flight filesystem/database commit merely
because the caller stopped waiting. Platform authorization dialogs can delay
blocking I/O; they must not occupy the UI thread or async executor.

`SchoolExecutor::execute` returns `SchoolExecutorFuture<'_>` and is awaited by
the coordinator. Implementations must not start a nested blocking runtime.
Credential-vault access runs on a blocking worker before the executor starts.

## Errors and failure recovery

Core service APIs return domain error enums. Match their variants rather than
their `Display` text. Tauri converts errors to the existing display strings at
its command boundary; the MCP transport retains its existing error envelope.
String payloads provide diagnostics, not machine-readable classifications.

School browser errors retain invalid input, authentication, storage, timeout,
stopped-service, changed-page and cancellation categories through the executor
and pull coordinator. Native browser platform failures remain execution errors.
`SchoolError::Coordination(PullError)` retains coordinator failures such as
`RuntimeUnavailable`, `ExecutorPanicked`, `ExecutorStopped` and
`CoordinatorStopped`; callers can inspect the source without parsing text.
Reference refresh similarly preserves validation, network, storage and
cancellation errors, with remaining coordinator errors retained as a typed
`ReferenceError::Coordination` source.

`CoreRuntime::run` and `run_scheduled` return `RuntimeTaskError` for the owned
operation itself, separately from the operation's domain result:

| Variant       | Meaning                                          | Recovery                                                                   |
| ------------- | ------------------------------------------------ | -------------------------------------------------------------------------- |
| `Stopped`     | Admission has closed or its lock is unavailable  | Use an active Core instance; do not retry on a stopped scope               |
| `Unavailable` | No Tokio runtime is entered                      | Enter the host-owned runtime before invoking the operation                 |
| `Panicked`    | The admitted worker unwound                      | Report failure and re-read affected state before deciding whether to retry |
| `Interrupted` | The admitted worker was cancelled by its runtime | Recover the runtime and re-read state before retrying                      |

Dropping a caller does not cancel admitted work. The tracked worker remains
owned until completion, and shutdown drains it. A worker panic is returned as an
error and is never converted into success. This boundary does not roll back
work already committed or guarantee that arbitrary third-party state remains
usable after a panic. Runtime destruction, process aborts and native crashes
cannot be recovered by this task boundary.

Cloud sync retains typed task failures in `CloudSyncError::Task` and
`StorageTask`, with the existing user-facing messages. Its supervised queue
clears the syncing flag and resets interrupted running deliveries to pending;
the saved delivery can retry. Ordinary storage and transport errors keep their
domain categories.

For coordinated pulls, cancellation before `applying` prevents the pending
payload from being committed; an admitted apply finishes. Partial school
results retain successful rows and outcomes while the pull snapshot becomes
`Failed`. Inspect the outcome and current snapshot before retrying. A timeout
does not imply that earlier pages or a separate completed operation rolled back.

Both Core and OCR deny production `unwrap`, `expect`, `panic`, `todo`,
`unimplemented` and `unreachable` through Clippy. Tests may inject panics to
verify failure paths. These checks complement input validation and bounded
indexing; they do not prove that dependency code can never panic.

[Download APIs](core-downloads.md) and [notes/backup/cloud APIs](core-backup-sync.md)
provide detailed errors, cancellation and recovery contracts. Desktop acceptance
is recorded in [verification](verification/core-consolidation.md).

Remaining phases cover foreign bindings and native shell adoption.
The facade and school-browser port do not claim that
Swift/Kotlin bindings or physical-device acceptance are complete.

## Type generation

Regenerate the shared TypeScript contracts with `yarn core:types`. The root
command runs `cargo run -p isct-core --features ts --example export-types` and
formats `apps/desktop/src/generated/persistence.ts`. The exporter belongs to Core
and is not included in desktop application bundles.
