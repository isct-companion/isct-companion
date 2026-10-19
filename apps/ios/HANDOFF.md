# iOS handoff

State of `feat/mobile-ios` and the iOS work that could not continue on the
machine where it was built (an Intel Mac, with no signing team, no device and no
school credentials available to the agent). Read [README.md](README.md) first
for the build and the layout.

## Done and verified

Verified in the iPhone 17 simulator (iOS 26) against a copy of a real desktop
`school.db`:

- First launch shows the connect card.
- 今日: the next class, its room and the map button, plus deadlines and recent
  items. All of it comes from Core's `WorkspaceQuery.today`.
- 時間割: the real week, with holidays hatched.
- 課題: Core's `assignmentBoard`; hide, undo and restore work, and submitted
  rows show grades. Since the main merge (2026-10-18) the board is the
  quarter task timeline that desktop and Android render: the tab shows
  `today`'s groups and its bottom row opens the full `record`. The Swift side
  was rewritten for that shape and first checked on a device, not the
  simulator.
- Course detail: the map, LMS and syllabus links, which come from
  `CourseDetail.lmsUrl` / `syllabusUrl` in Core.
- The ウェブ tab and pushed school pages load the SSO login page in the resident
  WKWebView.
- Settings sheet: notification settings.

## Blocked here: needs another machine, a device or an account

### 1. Matrix-card OCR import (needs a real card)

Built on 2026-10-18. 設定 › Tokyo Tech offers 撮影, 写真から, 貼り付け and 入力. A
photo goes to Core's `scanMatrix`, which reports progress as `ocr-progress`. The
7×10 review grid is tinted by Core's `matrixConfidence`. The OTP field reads
QR codes the same way, through Core's `importOtp`. The Intel simulator builds
the "unavailable" OCR backend, so only a device exercises recognition.

- Done when a photo of a real matrix card fills the grid, saving it works, and
  the next SSO login answers the matrix challenge on its own.

### 2. Real school sign-in (needs the student's credentials)

The agent may not type passwords, so nothing after the login page was
exercised.

- Save the ID and password in the settings sheet, then pull. `AppModel.pull`
  runs `scienceLms`, then `kyomu`. Check that 今日, 時間割 and 課題 fill from a
  fresh install, with no copied `school.db`.
- Check that the SSO plus matrix second factor completes inside the resident
  WKWebView (`Core/SchoolBrowserHost.swift`).
- Open question: once, the first tap on a course's LMS link opened Safari
  instead of the in-app page, and it could not be reproduced. Watch the
  `app.isctcompanion` / `school-web` log category in Console while you test.

### 3. Signing, entitlements and device-only behaviour (needs the team account)

`project.yml` has no `DEVELOPMENT_TEAM`. Set it to the Mac app's team, because
these items only work in a signed build:

- **iCloud Keychain shared with the Mac app.** Item names come from Core's
  `vaultItem`: service `app.isctcompanion`, account
  `app.isctcompanion.passwords.<suffix>`, and school logins are
  `synchronizable`. The access group `$(AppIdentifierPrefix)app.isctcompanion`
  must match the Mac release build. Check: a login saved on the Mac shows up on
  the phone. This is a step in `NATIVE-DEVICE-FLOWS` in
  `docs/verification/manual-checks.yaml`.
- **Notifications on a device.** `Core/NotificationScheduler.swift` registers
  Core's `notificationPlan`, up to 64 pending. Check permission, delivery
  times and re-planning after a pull.
- **iCloud backup.** Built; see below and `docs/mobile-backup-handoff.md`. The App ID needs the iCloud
  capability with the `iCloud.app.isctcompanion` container, which the Mac app
  already uses, so a signed build with the team can read the Mac's record.
- **Widgets.** These need a widget extension target and an App Group shared
  with the app. The read model should be Core's `today` query, not a second copy.

## Built, not yet checked on a device (2026-10-19)

- ウェブ: sites are switched by long-pressing the tab (`TabBarLongPress`, a
  recognizer on the `UITabBarController`'s bar) instead of the floating
  button. On iOS 26 check that the press lands on the ウェブ item and that the
  bar's own drag does not also switch tabs; if the recognizer cannot attach,
  fall back to `tabViewBottomAccessory` opening the same `SitePickerRows`.
- Settings: notifications (classes, exams, assignment lead times), study
  profile, data updates (cellular switch, last read per site, 今すぐ更新) and
  the ウェブ tab's text size and first site, on the desktop's settings keys.
- バックアップ (`Features/Settings/BackupScreen.swift`, `Core/CloudKitStore.swift`):
  iCloud on/off, automatic or by-hand Upload/Pull, earlier versions, local
  recovery files, and a backup file export/import. `CloudKitStore` only moves
  the record's bytes and change tag; Core names the record (`cloudkitRecord`),
  parses, merges and keeps history, the same code the Mac app runs. The
  entitlements pin the Production CloudKit environment so a development
  build sees what the released Mac app wrote. Check `IOS-ICLOUD-BACKUP`.

## Not blocked, still open

These can be done anywhere:

- Quick Look for downloaded course files. `AppModel` passes `downloads: nil` for now; implement `NativeDownloadSession` (background `URLSession`) first.
- A row for intensive courses (集中講義) in 時間割.
- ログを共有 in 設定 › 情報: Core keeps a log ring (`diagnostics.rs`) but the
  FFI exposes no export yet.

Android (Compose) comes after iOS and uses the same Core queries. Its design
tokens are already generated to `design/dist/android/`.
