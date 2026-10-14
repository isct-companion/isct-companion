# Notes, backups and cloud synchronization

Phase 2d exposes shared Rust services for desktop and future Swift, Compose and
MCP adapters. These are Rust APIs; foreign bindings and MCP tools are not wired
yet. The desktop commands and serialized documents keep their existing names
and formats.

## Ownership and entry points

| Owner | Interface | Responsibility |
|---|---|---|
| `notes::NoteStore` | `at`, `select_root` | Shared note gate and selected storage root; reject stale handles after a folder change |
| `notes::Notes` | `save`, `read`, `move_note`, `list`, `transaction` | Validated paths, Markdown/sidecar files, revisions and tombstones |
| `backup::BackupService` | `snapshot`, `export`, `validate`, `write_scheduled` | One schema-6 document for local export, scheduled export and synchronization |
| `cloud_sync::CloudSync` | `state`, `changed`, `tick`, `sync`, `resume` | Durable jobs, provider acknowledgements, retry and startup replay |
| `cloud_sync::CloudSync` | `provider_result`, `set_enabled`, `set_automatic` | Provider settings and queue policy |
| `cloud_sync::CloudSync` | `restore`, `restore_document`, `recovery_document`, `prepare_reset` | Recovery checkpoints and generation changes |
| `cloud_sync::document` | `parse_cloud_workspace`, `versions`, `merge_and_push` | Cloud envelope, daily history and conditional-write conflict retries |
| `runtime::CoreRuntime` | `attach_cloud`, `shutdown` | Bootstrap attachment, stop admission and drain owned work |

The native shell supplies paths, OS dialogs, HTTP/CloudKit/OAuth/credential
storage, and events. `CloudServices` supplies a backup service, `CloudTransport`
and the shell's `EventSink` for each operation; state changes arrive as
`CoreEvent::CloudSyncState` and applied documents as `WorkspaceChanged`
(`cloud-sync` / `cloud-restore`). `WorkspaceTransport` is the lower-level
conditional document read/write port. A shell should construct fresh services
after changing the notes folder. Core retains no Tauri handles.

The native one-second wake supplies an opportunity to work. Core admits a
cloud pass at most every five seconds, applies a 45-second remote timeout and
serializes queue execution. Only one periodic pass may remain in flight, so a
slow scan cannot accumulate background workers. In async cloud flows, note
scans, backup apply and recovery-file I/O run on blocking workers; they never
occupy async executor threads while waiting on a filesystem. The lower-level
`BackupService`, `Notes` and `recovery_document` APIs are synchronous; async
adapters must dispatch them to blocking workers. `changed(CloudServices)` is async and waits for its owned
capture. Callers may drop their futures: admitted work stays
owned until completion or runtime shutdown. Event delivery is best effort;
`state()` remains the authoritative snapshot.

## Delivery transitions

Each delivery persists exactly `pending`, `running` or `completed`. Unknown
persisted values fail journal loading with a compatibility error. Guards retain
the current state when a job is not due, its provider is disabled, or another
worker owns the run lock.

| Event | Pending | Running | Completed |
|---|---|---|---|
| Eligible queue pass | Running, increment attempts | No second worker | Unchanged |
| Worker success | No independent callback accepted | Completed, update provider acknowledgement | Unchanged |
| Transport failure/timeout | Unchanged until a worker starts | Pending with error and backoff | Unchanged |
| Manual retry | Clear retry deadline | Existing worker continues | Unchanged |
| Shutdown or startup recovery | Pending | Pending, clear retry deadline | Completed |

Completion/failure is consumed by the owned worker, rather than an externally
callable state mutation. Backoff is 30, 60, 120, then 240 seconds for subsequent
attempts. Shutdown returns unfinished work to pending without recording a
network failure. Worker panic is supervised and leaves retryable work.
Disabling a provider removes its deliveries from queued jobs; re-enabling with
automatic sync captures current state. Reset checkpoints the current workspace before clearing
the queue and provider bindings.

## Applying data and recovering failures

The journal uses SQLite and commits a cloned candidate before publishing it to
memory. Existing JSON journals migrate through the same loader. A local apply
first persists `pending_apply`, then applies notes and SQLite data, then clears
the intent. A failure leaves that intent available for replay.

`BackupService::snapshot` holds the note gate while collecting notes and one
SQLite read transaction. Applying data re-reads current local rows and notes;
SQLite merge uses an IMMEDIATE transaction and rechecks workspace generation.
This preserves edits made during a remote request and prevents an old
generation from resurrecting data removed by a restore.

Root selection metadata has a separate short lock: constructing a note handle
never waits for a scan. Selecting a new root still waits for current note I/O
before publishing it. Native exports do not hold the ordinary workspace-command
gate while waiting on the notes volume.

The filesystem and workspace database are separate commits. Their recovery
guarantee is idempotent replay through the journal, not an all-or-nothing
filesystem/database transaction. Shells must use `restore_document` for import
and `restore` for history recovery. `BackupService::apply` is the lower-level
replay primitive and must be used with durable intent.

Note export scans all notes and explicitly rejects more than 1,000 notes or
32 MiB of note content; it never silently truncates the traversal. A note is
limited to 1 MB. Scheduled exports retain the 32 MiB document limit and
`workspace-latest.json` filename. Replacement uses a unique sibling file,
flush/sync and rename; Unix temporary files are created with mode 0600.

## Source organization

`src/features/lms/` owns the shared LMS workspace, previews, note editor and
their helpers/tests. Pages compose this feature instead of importing shared
code from another page's private subtree. This replaces the old
`src/pages/today/lms/` directory without changing routes or presentation.

`backend/src/commands/backup.rs` contains backup command adapters and native
file dialogs. `backend/src/commands/school_auth.rs` runs potentially interactive
vault operations on blocking workers, independently of workspace database
commands. School page-load callbacks also read credentials off the UI thread
and consume a current-page token before resuming on the main thread.
`backend/src/cloud_sync.rs` contains native ports and command
forwarders. Business rules live under `crates/isct-core/src/{notes,backup,cloud_sync}`.
Other frontend features can follow this boundary as their domain logic moves
into Core; this checkpoint does not reorganize unrelated UI files.
