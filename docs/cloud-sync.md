# Cloud sync and recovery

Sync and backup have separate responsibilities. Sync distributes user changes;
recovery points retain earlier content even when deletes have already synced.
School data is a refetchable cache; only user records, overlays, preferences
allowed by `data-ownership.md`, and local notes enter this protocol.

## Triggers

The Backup settings page has an **Auto** switch, off by default, and separate
**Upload** and **Pull** buttons. This is device state.

- Manual: saving, launching, focusing or closing the app does not start a new
  transfer. Upload reconciles reachable cloud copies and publishes the merged
  data. Pull merges cloud changes locally and never creates an upload.
- Auto: each relevant native record/setting/note save immediately journals an
  upload. There is no 30-second debounce and no periodic cloud polling. A
  five-second local comparison also detects external MCP writes; multiple
  external changes between observations can be represented by one snapshot.
- Opening/resuming the app resumes unfinished operations in either mode. Turning
  Auto off prevents new automatic operations; it does not revoke submitted work.
- Auto is change-triggered, not a real-time remote subscription. Another device's
  changes arrive on Pull or the next local change's reconciliation.
- Exit captures pending changes durably and exits without waiting for a network
  round trip. The next launch resumes work. The OS need not grant background time.
- Disabling a provider disconnects only it and cancels its unfinished deliveries.
  Enabling it in manual mode does not upload anything.

## Platform and account boundaries

| Platform | Available providers | Recommended common channel |
| --- | --- | --- |
| macOS / iOS / iPadOS | iCloud enabled by default; optional Drive | iCloud for Apple-only use |
| Windows / Android / Linux | Drive after connecting | Google Drive |
| Mixed platforms | Drive on every device; optional iCloud on Apple | Google Drive |

An Apple device connected to both services can relay changes between them;
iCloud and Drive do not communicate independently. Provider support is compiled
with Rust cfg flags. No user-agent platform detection is used.

OAuth credentials remain independent, device-local vault entries. Cloud content
carries a `sync.workspaceId`. The first identified cloud workspace binds the
installation; a different workspace is rejected before merging or uploading.
Cloud errors do not clear another provider's token or acknowledgement. The
shared cross-platform channel, Drive, is serviced first. Reads/writes have bounded
timeouts; an unavailable provider retains its own retry state.

## Versions and merge priority

There are three different identities, each with one responsibility:

1. **Content version**: `v1-<SHA-256>` of normalized user data and restore
   generation. The same content has the same version on every device/provider.
   Export time, device identity and array ordering do not change it. The UI
   abbreviates the hash; the journal retains it in full.
2. **Task ID**: UUIDv7, created once before network work. Retries retain the ID.
   Every delivery records provider, pending/running/completed, attempts, next
   retry and error. Network reconciliation may refine its target content version.
3. **Provider revision**: the provider-local numeric revision plus its ETag or
   CloudKit change tag, used only for conditional writes. It is not a global
   sequence number and does not determine which user's edit wins.

Within the same restore generation, merge by record ID. Higher record revision
wins; equal revisions compare modification/deletion timestamps, then writer ID,
then canonical content. Writes increment the stored/imported revision, so a
subsequent edit survives a clock moving backwards. Concurrent edits to one
record use deterministic whole-record resolution, not field-level collaboration.
Independent records are all retained; deletes are versioned tombstones.
Singleton preferences/profile/terms use the same rules.

A cloud's name or download order never gives its content priority. Upload first
merges reachable replicas. Conditional writes re-read for concurrent updates.
If an update is discovered during publication, a follow-up task brings lagging
providers to that merged version, including in manual mode. Local edits made
after a manual Upload remain local unless separately submitted.

## Durable queue and acknowledgements

`cloud-sync.sqlite` contains the device configuration, queued snapshots, task
receipts, provider acknowledgements and interrupted local-apply intent. SQLite
commits use `synchronous=FULL`; a failed commit cannot advance the in-memory
journal. Legacy `cloud-sync.json` configuration is imported once.

The sequence is:

1. Save user data locally.
2. Persist a task containing the source snapshot and provider deliveries.
3. Reconcile the clouds; persist the resulting target before uploading.
4. Persist `running` and increment attempts before each delivery.
5. Conditionally write/read, durably stage local application, apply it, then
   persist that provider's completed receipt and acknowledged content version.

A crash between data save and task creation is recovered by comparing saved
content with the journal on launch. A crash after a cloud accepted an upload but
before acknowledgement re-reads it: identical content does not create another
cloud version. Interrupted `running` deliveries become pending on restart.
Retries back off from 30 seconds to a few minutes; an explicit request retries
pending work immediately. New changes queue separately while a transfer runs.

Acknowledgement is per provider. Drive at version B and iCloud at A is partial
completion, not globally "Up to date". The UI shows unfinished tasks and only
shows up-to-date when all enabled providers acknowledge local content and no
delivery remains pending. The ten latest completed receipts are retained.

## Recovery

The cloud format is now version 2. The reader accepts legacy version 1; older
apps reject new envelopes rather than silently bypassing restore generations.
Current data, two immediate predecessors and up to seven daily recovery points
live in one conditionally written envelope. History entries are flat; syncing
unchanged content never rotates history. Daily points represent distinct days
with saved changes, not a guarantee of an upload on idle days. The existing
25 MiB envelope limit is enforced without silently dropping notes or history.

The UI groups cloud versions by content version, listing their storage locations.
Before importing changed cloud content or restoring a version, a content-addressed
local recovery file is saved atomically. These local files are independently
listed for export or restoration and are not automatically removed.

Restoring is an explicit operation affecting all connected devices, in either
mode. Pending transfers must finish or their provider must be disconnected first.
The operation reads enabled providers to determine the generation boundary;
if one is unavailable, restore stops before changing current data. It then:

- saves a local recovery point;
- assigns a new `{ counter, id }` restore generation;
- journals the upload intent before replacing local content;
- replaces user tables and sync metadata in one SQLite transaction;
- replays a durable local-apply intent if notes/table application is interrupted;
- publishes the new generation through the ordinary retryable queue.

A higher generation replaces the old branch rather than merging missing records
back into it. Local backup-file imports use this same restore protocol; clearing
this device instead disconnects the queue and keeps a local recovery file.
An old device saves its local content for recovery before adopting
the new generation. Simultaneous restores compare generation counter and ID
consistently. School caches and device-only settings are not restored from cloud.

## Credentials

Credentials are a separate channel. Apple Keychain synchronization is managed
by the OS. Drive's encrypted credential backup uses the existing explicit setup,
upload and restore flow. Workspace uploads no longer blindly overwrite that
file from possibly stale credentials on another device. Tokens and encryption
passwords never enter workspace snapshots or task payloads.

## Bridge

- `cloud_sync_state` / `cloud-sync-state`: configuration, current content version,
  per-provider acknowledgement, tasks, cloud versions and local recovery points.
- `set_cloud_sync_automatic { automatic }`: change Auto and capture current data.
- `set_cloud_sync_provider { provider, enabled }`: independently connect/disconnect.
- `sync_cloud_now { direction: "upload" | "pull" }`: enqueue an explicit operation.
- `restore_cloud_version { id }`: restore a cloud or registered local recovery point.
- `cloud_recovery_document { id }`: read a registered recovery point for export.

## Verification

Unit tests exercise durable restart, immutable saved source changes, separate
provider acknowledgements, content-version normalization, revision ordering with
clock skew, restore generations, and SQLite replacement rejecting stale uploads.
Frontend tests exercise the switch, separate directions, partial completion,
and mobile single-tap/directional actions. Real provider authentication,
network loss during a native upload, and phone sleep/resume still require signed
device integration testing.
