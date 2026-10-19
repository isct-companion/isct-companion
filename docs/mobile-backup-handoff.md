# Mobile iCloud backup — handoff for gliese

Date: 2026-10-19. Code on main: `a9b4cbf` (feat(mobile): iCloud backup on iOS
through Core's shared cloud transport).

The iPhone app can now back up to and restore from the same iCloud record the
Mac app writes, so either device loads the other's data. All of the logic is in
Core. The iOS shell only moves the record's bytes. The work is **built and
unit-tested but has never run on a device**: nobody on the team could sign it
yet. What is left is mostly signing and team setup, plus three code follow-ups.

| # | Item | Who | Status |
|---|------|-----|--------|
| 1 | Find out which team can sign `app.isctcompanion` with iCloud | Team owner | **Blocking** |
| 2 | Enable iCloud on the iOS App ID, register devices | Team owner / Admin | Blocked by 1 |
| 3 | Install on a phone (Xcode or TestFlight) and run `IOS-ICLOUD-BACKUP` | gliese | Blocked by 2 |
| 4 | Check the macOS iCloud path on a Developer ID build | gliese | Open |
| 5 | Compile-time switch so Personal Team builds don't crash | Code | Open |
| 6 | Android cloud backup (Google Drive) | Code | Open, larger |
| 7 | Remove stale OneDrive code and its manual check | Code | Open, small |

---

## What landed

- **Core** (`crates/isct-core/src/cloud_sync/document.rs`):
  - `DocumentCloud` turns any `WorkspaceTransport` into the queue's
    `CloudTransport`. Every shell now shares one envelope parse, history listing
    and merge-and-push loop. Before this change, desktop had its own copy and the
    FFI port asked Swift/Kotlin to write a third.
  - `ObjectStore` + `TaggedWorkspace` are for a provider that stores one object
    with a change tag (CloudKit). The store reads bytes and a tag, and writes
    bytes only if the tag still matches. Core pairs each envelope revision with
    the tag it was read with.
  - The CloudKit location is named once: container `iCloud.app.isctcompanion`,
    record type `IsctCompanionBackup`, record name `IsctCompanion.json`, asset
    key `payload`. The FFI exports it as `cloudkitRecord()`.
- **FFI** (`crates/isct-ffi`):
  - `NativeCloud` is now a byte port: `deviceName`, `read`, `write(bytes,
    expected)`, `disconnect`.
  - `applyBackup` imports through `CloudSync::restore_document`, as desktop
    does. It saves a recovery point, starts a new generation and makes the apply
    replayable. The journal exists even when no cloud store is installed.
- **Desktop**:
  - macOS `icloud.rs` is now an `ObjectStore`. Conflicts are detected by the
    CloudKit record change tag instead of by parsing the payload's revision.
  - Google Drive is unchanged, apart from going through `DocumentCloud`.
- **iOS**:
  - `Core/CloudKitStore.swift` is the CloudKit implementation of the byte port.
  - The entitlements add iCloud/CloudKit with the **Production** container
    environment. Development-signed builds would otherwise use CloudKit's
    separate Development database and never see what the released Mac app
    wrote.
  - 設定 › バックアップ (`Features/Settings/BackupScreen.swift`) has the iCloud
    switch, automatic or manual Upload/Pull, earlier versions, local recovery
    files and backup-file export/import.

Tests: `tagged_writes_carry_the_tag_read_with_the_expected_revision` (core) and
`sessions_sharing_one_cloud_record_load_each_others_data` (FFI). In the FFI
test, two sessions share a fake CloudKit record, load each other's data, and
survive a write that loses the conditional save.

---

## 1. Which team signs the iOS app (blocking)

What we know:

- The Mac release is signed by team **`FB7AL944GK`**
  (`scripts/desktop/macos-signing.mjs`). The iCloud container
  `iCloud.app.isctcompanion` therefore belongs to that team.
- The developer's Xcode shows a team named **"Zean Miao"**, with role
  *Developer*. Building with `DEVELOPMENT_TEAM=FB7AL944GK -allowProvisioningUpdates`
  fails with `No Account for Team "FB7AL944GK"`. Either the Zean Miao team has a
  different ID, or the account cannot sign for it.
- If the Zean Miao program is an **Individual** enrollment, invited users get
  App Store Connect access only. They cannot create signing certificates or
  profiles, so they cannot sign. Only an **Organization** enrollment lets
  members sign with their own Apple IDs.

To do:

1. In developer.apple.com → Membership details, check the Team ID and the
   enrollment type.
2. Make sure the iOS app is signed by **the same team as the Mac release**. A
   container belongs to one team, so a phone signed by a different team cannot
   read the Mac's backup.
3. If the program is Individual, use TestFlight (§3b) for device testing. The
   developer keeps the Personal Team for UI work once §5 lands.

## 2. App ID and devices (team owner or Admin)

- In Certificates, Identifiers & Profiles → Identifiers → `app.isctcompanion`,
  turn on **iCloud** with **CloudKit** and tick the
  `iCloud.app.isctcompanion` container. The Mac app should already have this;
  check that it applies to the iOS App ID too.
- Register the test iPhone. Ask the developer for its UDID. A *Developer* role
  may not be allowed to register devices; an Admin can, or can grant "Access to
  Certificates, Identifiers & Profiles".
- In CloudKit Console → `iCloud.app.isctcompanion` → **Production** schema,
  check that record type `IsctCompanionBackup` exists with an asset field
  `payload`. It is created the first time a released Mac app uploads to iCloud.
  If no released Mac has ever done that, deploy the schema from Development
  first. Otherwise Production rejects the iPhone's first write.

## 3. Install and verify

**a. Direct install**, with a team that can sign:

```bash
ISCT_SDK_PROFILE=dev node scripts/bindings/build-ios.mjs
```

```bash
xcodegen generate --spec apps/ios/project.yml
```

```bash
xcodebuild -project apps/ios/IsctCompanion.xcodeproj -scheme IsctCompanion -destination 'id=<iPhone UDID>' -configuration Debug -allowProvisioningUpdates DEVELOPMENT_TEAM=<team id> build
```

Then install the `.app` with `xcrun devicectl device install app`, or run from
Xcode. Do not commit `DEVELOPMENT_TEAM`.

**b. TestFlight**, if the program is Individual: archive with the owner's
account, upload, and add the developer as an internal tester.

Then run **`IOS-ICLOUD-BACKUP`** in `docs/verification/manual-checks.yaml`:
upload from the Mac and pull on the phone, then the reverse, then concurrent
edits on both, then restore and file import. Record the result with
`yarn verify:manual record`.

## 4. macOS iCloud on a Developer ID build

`apps/desktop/src-tauri/src/icloud.rs` changed: conflicts now use the record
change tag. macOS debug builds never touch iCloud: `account_available` returns
early, because unsigned CloudKit traps. This path therefore only runs in a
signed release build. It is part of `IOS-ICLOUD-BACKUP`. Before release, also
check that an existing Mac with an iCloud backup still uploads and pulls.

## 5. Personal Team builds crash (code)

The Personal Team cannot carry the iCloud entitlement. `CloudKitStore()` is
created at launch, and CloudKit throws without the entitlement. So the old
Personal Team flags (`DEVELOPMENT_TEAM=<personal> PRODUCT_BUNDLE_IDENTIFIER=…`)
now produce a build that **crashes on launch**.

Fix at compile time, per AGENTS.md §3:

- Add a build configuration or xcodegen setting that drops the three iCloud
  entitlement keys.
- Set a Swift compilation condition, for example `ICLOUD`, only when the
  entitlement is present.
- Under `#if ICLOUD`, pass `cloud: CloudKitStore()` and
  `cloudProviders: [.icloud]`. Otherwise pass `nil` and `[]`.
- The Backup page then shows only the backup file (Core's `cloud()` reports
  unavailable). Import still goes through the journal.

## 6. Android cloud backup (code, larger)

Android passes `cloud = null`. The Google Drive transport lives in the desktop
shell (`apps/desktop/src-tauri/src/cloud.rs`, reqwest, with sibling-file and
ETag handling). Writing it again in Kotlin would duplicate it. Instead:

- Move the Drive `WorkspaceTransport` into a shared Rust module.
- Have the native side supply only an OAuth access token, through a port.
- Android then needs a Google OAuth client for its package and signing key.

Docs: `docs/cloud-sync.md` (protocol), `docs/core-backup-sync.md` (layers).

## 7. Stale OneDrive (code, small)

Desktop only offers Google Drive and iCloud (`cloud_sync.rs` `supported`). The
OneDrive branches in `cloud.rs` (OAuth endpoints, read/write) and the
`CLOUD-ONEDRIVE-WRITE` manual check describe a provider the app does not offer.
Delete them, or decide to bring OneDrive back.
