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
| `cloud_sync::document` | `parse_cloud_workspace`, `versions`, `merge_and_push` | Cloud envelope, age/minimum-copy history retention and conditional-write conflict retries |
| `cloud_sync::document::DocumentCloud` | `new(transport, device_name)` | The queue's `CloudTransport` over any `WorkspaceTransport`; every shell shares its parse, history and merge loop |
| `cloud_sync::document::TaggedWorkspace` | `new(store)` | `WorkspaceTransport` over an `ObjectStore` that only reads and writes bytes with a change tag (CloudKit on macOS and iOS) |
| `cloud_sync::document::Providers` | `with(provider, store)` | One `WorkspaceTransport` routing each provider to its own store (CloudKit and Drive on Apple devices) |
| `cloud_sync::google_drive::GoogleDrive` | `new(authorization)`, `read_file`, `write_file` | Drive `appdata` REST for the workspace envelope and the credential file, over a `DriveAuthorization` |
| `cloud_sync::google_drive::Pkce` | `begin`, `finish`, `authorized` | PKCE sign-in and refresh-token storage in the shell's `SecretVault` (desktop and iOS) |
| `runtime::CoreRuntime` | `attach_cloud`, `shutdown` | Bootstrap attachment, stop admission and drain owned work |

The native shell supplies paths, OS dialogs, HTTP/CloudKit/OAuth/credential
storage, and events. `CloudServices` supplies a backup service, `CloudTransport`
and the shell's `EventSink` for each operation; state changes arrive as
`CoreEvent::CloudSyncState` and applied documents as `WorkspaceChanged`
(`cloud-sync` / `cloud-restore`). `WorkspaceTransport` is the lower-level
conditional document read/write port; shells wrap it in `DocumentCloud`
rather than implement `CloudTransport`. A provider that stores one object
with a change tag implements only `ObjectStore` (read bytes and tag, write
bytes if the tag still matches) and goes through `TaggedWorkspace`, which
pairs each envelope revision with the tag it was read with. The FFI's
`NativeCloud` is that byte port, so Swift and Kotlin never parse or merge.
The CloudKit record (container, record type, name, asset key) is named once
in Core and exported as `cloudkitRecord`.

Google Drive is one Core module for every shell, `cloud_sync::google_drive`.
It owns the `drive.appdata` REST calls (lookup, sibling files, ETag
conditional writes, the encrypted credential file) and implements
`WorkspaceTransport`; a shell only supplies an access token through the
`DriveAuthorization` port:

| Shell | OAuth client | How the token is obtained |
|---|---|---|
| Desktop | Desktop client (with its non-confidential secret) | Core's PKCE flow; the shell opens the browser and runs the loopback listener |
| iOS / iPadOS | iOS client | Core's PKCE flow; the shell presents `ASWebAuthenticationSession` and returns the callback URL |
| Android | Android client (package + signing SHA-1) | Google Identity `AuthorizationClient`; the shell returns a short-lived access token, silently after the first consent |

Core's PKCE flow keeps the refresh token in the shell's `SecretVault` under
the existing `google-drive` account, so a desktop's saved authorization keeps
working. The FFI exports the port as `NativeDriveAuthorization`, and
`SessionConfig.cloudProviders` lists Drive on both mobile shells. A shell should construct fresh services
after changing the notes folder. Core retains no Tauri handles.

The native one-second wake supplies an opportunity to work. Core admits a
cloud pass at most every five seconds, applies a 45-second remote timeout and
serializes queue execution. The five-second pass is a local scheduler wake,
not an upload frequency: automatic uploads require a changed content hash and
one hour since the provider's last successful sync. First copies and explicit
manual uploads can run immediately. Pending snapshots coalesce before execution,
keeping record/tombstone merges, completed receipts and retry budgets.
Only one periodic pass may remain in flight, so a
slow scan cannot accumulate background workers. A pass with nothing due
writes and publishes nothing: unchanged journal updates skip the fsync'd
write, and `cloud-sync-state` is emitted only when the state changed.
`provider_result` does not wait for a running queue pass. In async cloud flows, note
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
attempts. A provider's pending failure cools down all its queued operations;
new local saves cannot bypass it. Explicit manual sync clears the deadline.
Shutdown returns unfinished work to pending without recording a
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

`apps/desktop/src/features/lms/` owns the shared LMS workspace, previews, note editor and
their helpers/tests. Pages compose this feature instead of importing shared
code from another page's private subtree. This replaces the old
`apps/desktop/src/pages/today/lms/` directory without changing routes or presentation.

`apps/desktop/src-tauri/src/commands/backup.rs` contains backup command adapters and native
file dialogs. `apps/desktop/src-tauri/src/commands/school_auth.rs` runs potentially interactive
vault operations on blocking workers, independently of workspace database
commands. School page-load callbacks also read credentials off the UI thread
and consume a current-page token before resuming on the main thread.
`apps/desktop/src-tauri/src/cloud_sync.rs` contains native ports and command
forwarders. Business rules live under `crates/isct-core/src/{notes,backup,cloud_sync}`.
Other frontend features can follow this boundary as their domain logic moves
into Core; this checkpoint does not reorganize unrelated UI files.
