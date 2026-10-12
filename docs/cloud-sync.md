# Automatic cloud sync

Backup becomes sync: it runs on its own, merges per record, and keeps two
earlier versions for rollback. The Backup settings page
(`src/pages/settings/BackupSettings.tsx`) is already built against the bridge
contract below. The native side implements it.

## Providers

| Device | Default | Optional |
| --- | --- | --- |
| macOS, iOS, iPadOS | iCloud (CloudKit private DB), on without setup | Google Drive, for people who also use Windows or Android |
| Windows, Android, Linux | none until the person connects | Google Drive |

- iCloud is enabled on first launch when the iCloud account is available. If
  it is not, it reports `available: true, enabled: true, error: "…"` and
  retries silently; it never asks.
- Several enabled providers are all kept in sync: pull and merge from each,
  then push the merged result to each.
- Pick the provider set with `#[cfg(target_vendor = "apple")]` /
  `#[cfg(...)]`, never at runtime (see AGENTS.md rule 3). iCloud is not
  compiled in elsewhere, so it is absent from `providers` there.
- Sync configuration (enabled providers, tokens, last sync) is device state in
  native storage, not part of the workspace document. The workspace's
  `settings.cloud` and `syncStatus` become legacy: migrate an existing Drive
  connection once, then stop reading them. Keep the developer `clientId`
  override.

## Fix first: providers must not sign each other out

Today `connect`, `connect_with_native_token` and `connect_icloud` in
`backend/src/cloud.rs` delete every other provider's stored token, because
only one provider was allowed. On an Apple device, any iCloud connect wipes
the Google Drive token, so Drive asks to sign in again. This is a design
defect, not a rebuild artifact. Keep each provider's token independently;
disconnect only removes its own.

Also check token validity: a stored token is ignored when its `client_id`
no longer matches the build's configured client (per-target IDs from
`config/oauth-clients.yaml`, or the developer override). Prefer keeping the
token and refreshing with the client it was issued to, and report a clear
`error` when that fails, instead of silently showing "not connected".

## When it runs

- On launch and when the app returns to the foreground: pull, merge, push.
- After local changes: push about 30 s after the last change (debounced).
- When the app goes to the background or the window closes: push right away
  if there are unpushed changes.
- In the background scheduler, alongside the existing scheduled tasks.
- One run at a time. A run that overlaps another is folded into it.

## Merge: newest change wins, per record

- Records with an `id` (courses, assignments, plans, events, plan items,
  memberships, intensive schedules, …) merge by `id`. The side with the newer
  `updatedAt` wins. Ties go to the higher `rev`, then to the larger device
  id, so every device reaches the same result.
- Deletions are tombstones (`deletedAt`). They take part like any other
  change, so a delete newer than an edit wins and an edit newer than a delete
  revives the record.
- Singleton blocks without ids (synced settings, academic profile, terms)
  use the same rule as a whole block, with the block's own updated time.
  Device-only settings (window layout, density, local permissions, sync
  configuration) never leave the device.
- Merging is a pure function in Rust with unit tests, shared by every
  platform (AGENTS.md rule 2). Replace the current conflict prompt and the
  `conflict` setting; no dialog is shown.

## Versions

- Before each push that changes the cloud copy, the previous copy becomes
  `previous-1` and the old `previous-1` becomes `previous-2`. Only two are
  kept.
- Restoring a version replaces local data with it and pushes it as the new
  current copy, which keeps the replaced copy as `previous-1`.

## Login credentials

- With iCloud enabled on Apple devices: store school credentials as
  synchronizable Keychain items (`kSecAttrSynchronizable`), so iCloud
  Keychain carries them with no extra password.
- With Google Drive: keep the existing encrypted credential file and its
  8–12 character password flow. The settings page keeps that UI unchanged.

## Bridge contract

Types and methods are in `src/platform/desktop-bridge.ts`
(`CloudSyncState`, `cloudSyncState`, …). Commands:

| Command | Payload | Returns |
| --- | --- | --- |
| `cloud_sync_state` | none | `CloudSyncState` |
| `set_cloud_sync_provider` | `{ provider, enabled }` | `CloudSyncState` (enabling Drive runs its OAuth connect first) |
| `sync_cloud_now` | none | `CloudSyncState` after the run |
| `restore_cloud_version` | `{ id }` | `CloudSyncState` |

Event `cloud-sync-state` carries a `CloudSyncState` whenever any field
changes (run started or finished, error, versions updated).

```ts
type CloudSyncState = {
  syncing: boolean;
  providers: {
    provider: "icloud" | "google-drive";
    enabled: boolean;
    lastSyncedAt?: string; // ISO time of the last successful run
    error?: string; // last failure, cleared by a success
  }[];
  versions: {
    id: string;
    provider: "icloud" | "google-drive";
    savedAt: string;
    device: string; // device name shown to the person
  }[]; // newest first, at most two per provider
};
```

## Acceptance

- Rust unit tests for the merge rules: newer edit wins, tie-breaks, delete
  vs edit both ways, device-only settings untouched, deterministic on both
  sides.
- Rust tests for version rotation: never more than two, restore pushes and
  rotates.
- Two devices editing different records offline both keep their edits after
  syncing.
- No sync UI prompts. Failures show only as the provider's `error` on the
  Backup page.

## Native implementation

`cloud_sync.rs` owns device configuration (`cloud-sync.json` in app data),
serialized runs and lifecycle flushing. It samples user data every 5 seconds,
then waits 30 seconds after the last observed change; unchanged remote data is
checked every 5 minutes. This also observes writes from the separate MCP
process. Launch, foreground, background and orderly app exit request a run.

`data/cloud_merge.rs` merges the same user-owned tables used by local backups,
including tombstones and versioned `terms`, `academicProfile` and
`userPreferences`. School/reference caches remain refetchable data as defined
in `data-ownership.md`. SQLite imports merge inside an IMMEDIATE transaction
so edits committed during a network request are retained. Note files carry
modification clocks and writer metadata; deletions remain in `deleted-notes`.

The current cloud envelope embeds two flat earlier envelopes in `previous`
(newest first). This is the physical representation of `previous-1` and
`previous-2`: current and history are committed together, avoiding partially
rotated history. Restoring stamps the selected content as a fresh edit and
creates tombstones for records introduced since that version. Drive writes
use ETag conditions; CloudKit writes check the fetched revision and retain the
record change tag. Concurrent-write conflicts are retried without a dialog.

Google authorization uses the client that issued the stored token. Build-time
client IDs are also supplied to native mobile builds; the developer override
is used for new connections. Android uses Google Identity AuthorizationClient
and only opens consent for an explicit enable request; background renewal
reports an error if additional consent is needed. Apple school credentials
continue to use the existing synchronizable Keychain implementation.

Unit tests cover merge ordering, tombstones, offline records on two SQLite
stores, concurrent local writes, device setting isolation, history rotation,
rollback and persisted provider selection. Live provider authorization and
phone WebView behavior require signed-device integration testing.
