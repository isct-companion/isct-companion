# Windows 1.1.3 user reports — handoff

Date: 2026-10-19. Release in question: v1.1.3 (`521cbed`), Windows x64 NSIS
installer `IsctCompanion_1.1.3_x86_64-pc-windows-msvc.exe`.

Windows users reported four problems: the installer is blocked, scraping
fails, auto-update hangs, and the app cannot write files. v1.1.3 contains
**no Windows-specific fix**. Its notes cover the 教務Web course search, macOS
keyboard focus, PDF.js CMaps, course tab counts, missing workspace folders and
the school.db commit path.

| # | Report | Cause | Status (2026-10-20) |
|---|--------|-------|--------|
| 1 | "智能应用控制已阻止" on install | Unsigned installer + Smart App Control | Workflow ready (`sign_windows`); **needs an Azure Artifact Signing account and secrets** |
| 2 | Auto-update hangs / fails | 30 s whole-request timeout on a 34 MB download; errors and install results never shown | Fixed on main `f5ce74f`; window now hides at once on quit. Not released |
| 3 | Cannot write files | Likely Controlled Folder Access blocking an unsigned app in `Documents` | Refused writes now say how to allow them; root fix is signing (1). Still needs a user to confirm the cause |
| 4 | Scraping returns nothing | The extract relay URL can exceed WebView2's 2 MB cap; hidden views throttled | Both fixed in code; **not run on Windows** (`DESKTOP-WINDOWS-SCHOOL-PULL`) |

None of the Windows-only code paths below were compiled locally (no MSVC
target on the Mac). The PR's "Windows backend" platform check compiles them.

Root cause shared by 1, 3 and part of 2: **the Windows installer is not
code-signed**. The release workflow signs only macOS
(`.github/workflows/desktop-release.yml`; the release notes say "The Windows
installer is unsigned").

---

## 1. Installer blocked by Smart App Control

**Symptom.** The dialog reads 智能应用控制已阻止可能不安全的应用. Its only
buttons are OK and "Get an app from Microsoft Store". There is no "Run anyway".

**Cause.** Smart App Control (Windows 11) allows an executable only if it has a
trusted signature or cloud reputation. Ours has neither. It checks every launch,
whatever the source of the file, so these do **not** help: Properties → Unblock
(that only removes Mark-of-the-Web, which SmartScreen uses), copying the file,
renaming it, sending it over chat or USB, or switching to `.msi`.

**Why 1.1.2 "worked".** v1.1.2 and v1.1.3 use the same unsigned NSIS `.exe`
format, published half a day apart. The difference is the machine:

- The user who installed 1.1.2 has Smart App Control Off. Upgraded or older
  installs are often Off.
- Or the same PC moved from *Evaluation* mode (it watches and does not block)
  to *On* in between.

Users can see the state in Windows Security → App & browser control → Smart
App Control (On / Evaluation / Off).

**User workaround.** Turn Smart App Control Off. On older Windows 11 builds it
cannot be turned back on without resetting Windows, so many users will refuse.

**Fix (open).** Sign the Windows installer. Options:

1. **Azure Artifact Signing** (formerly Trusted Signing): about $10/month,
   plugs into Tauri through `bundle.windows.signCommand`. Individual
   eligibility is limited by region; check before relying on it.
2. **OV code-signing certificate**: a few hundred dollars a year, needs a
   hardware token or cloud HSM, and is harder to use from CI.
3. **Microsoft Store (MSIX)**: Microsoft signs the package, and individual
   registration is free. Tauri has no built-in MSIX output, and review takes
   time. A Store submission of a plain exe/msi must still be signed by us.

**Done in the workflow.** `sign_windows` (or the `WINDOWS_SIGNING_ENABLED`
variable) installs `artifact-signing-cli` 0.12.0; `scripts/desktop/tauri.mjs`
passes it to Tauri as `bundle.windows.signCommand`, so the app binary and
the NSIS installer are both signed; the job then requires a `Valid`
Authenticode signature on each. Release notes state each platform's signing
separately. Required secrets and variables are listed in
`docs/desktop-release.md` ("Windows signing"). What is left is the account
itself: an Artifact Signing account with a Public Trust profile (check region
eligibility for an individual publisher), an Entra app registration with the
signer role, and those six values in the repository. Manual check:
`DESKTOP-WINDOWS-SIGNED-INSTALL`.

---

## 2. Auto-update hangs or fails — fixed on main, not yet released

**Causes found in v1.1.3:**

- `desktop_updates.rs` built the updater with `.timeout(30s)`. In
  tauri-plugin-updater 2.14 that timeout is copied onto `Update` and applied as
  a reqwest **whole-request** timeout to the download as well
  (`updater.rs` ~l.849). A 34 MB installer needs about 9 Mbps to finish in
  30 s, so slower links always failed.
- During check and download the About panel showed a fixed "Checking for
  updates…" with no progress.
- On failure it showed "Update check failed" without `status.message`.
- `updater-install-result.json` was written before exiting but **never read**.
  On Windows `Update::install` runs `ShellExecuteW` on the installer and then
  calls `std::process::exit(0)`, so the record always stays `installing`. If
  Smart App Control blocks the installer, the app closes, nothing installs,
  and the next launch says nothing. That is what users perceive as "stuck".

**Fix — `f5ce74f fix(updater): let large downloads finish and report what happened`, pushed to `main`:**

- The download clears the request timeout and fails only after **60 s without
  data** (`DOWNLOAD_STALL`). It records `{downloaded, total}` progress.
- The frontend (`useDesktopUpdates.ts`) polls `app_update_state` every 500 ms
  while a check runs. About shows the native phase, `x / y MB`, and the error
  message.
- Core (`crates/isct-core/src/app_updates.rs`): `InstallRecord`, `LastInstall`
  and `last_install()`, which judge the record against the version that
  launched. The result is `installed`, `incomplete` (the installer was started
  but the old version is still running) or `failed`.
- The shell reads and clears the record at startup (`load_last_install`). The
  app shows a launch notice once (red when the update did not take effect) and
  a line in About. Strings were added to `ja.json` / `zh.json`.
- Tests run: `cargo test -p isct-core app_updates`, `cargo check` for the
  desktop crate, `tsc`, and the locale unit test. **Not run on Windows.**
- Manual check added: **`DESKTOP-WINDOWS-UPDATES`** in
  `docs/verification/manual-checks.yaml` (slow link, disconnect mid-download,
  install with Smart App Control Off and then On).

**Important.** The fix ships only with the next release. Users on 1.1.3 still
run the old updater, so on slow links 1.1.3 → next will fail the same way. Tell
them to download the next installer manually from Releases.

**Quit no longer looks frozen.** Quitting still drains tracked work for up to
`EXIT_TIMEOUT` (15 s), but every window now hides as soon as exit begins
(`lib.rs`, `ExitRequested`). Covered by `DESKTOP-WINDOWS-UPDATES`.

---

## 3. "The app cannot write files" — suspected Controlled Folder Access

**Where we write.** Course files, notes and assignment workspaces default to
`Documents\IsctCompanion` (`apps/desktop/src-tauri/src/lms_files.rs:50`,
`default_root`). App data (`school.db`, settings) lives in
`%APPDATA%\<app id>`, which this feature does not protect.

**Likely cause.** Windows Security → Ransomware protection → Controlled folder
access lets only trusted (signed or reputable) apps write to Documents,
Desktop, Pictures and similar folders. Our unsigned app gets "Access is denied
(os error 5)", and Windows shows "Unauthorized changes blocked". The feature is
off by default, which fits "only some users". Some security suites and
school-managed PCs turn it on.

**How to confirm.** Ask a user to open Windows Security → Virus & threat
protection → Ransomware protection → Controlled folder access, and check
*Block history* for IsctCompanion.

**User workaround.** Choose "Allow an app through Controlled folder access" and
add IsctCompanion, or move the course files location out of Documents in
Settings.

**Done: refused writes explain themselves.** `storage::WRITE_DENIED` is the
one text for a refused course-folder write (Windows names Controlled folder
access; other systems say to check permissions). Notes, workspaces and
materials build their storage errors through `storage::failure_text`. A
refused material download or folder creation is a typed
`FetchError::WriteDenied` / `MaterialError::Denied`, served as reason
`denied`, and the file card shows a localized hint. Nothing remembers the
refusal: after the user allows the app, Retry saves normally (covered by the
core tests). The root fix is still signing (see 1).

---

## 4. Scraping fails on Windows — no regression found, needs logs

A review of v1.1.2..v1.1.3 found no Windows-only change in the school-web
path, with about 80% confidence. `school_web.rs`, `lms_protocol.rs`,
`lms_downloader.rs` and the release workflow are unchanged. The `cfg` cleanups
removed only iOS/Android branches. 7767e11 (enter 教務Web first) and 437ae34
(keyboard focus, macOS only) do nothing platform-specific on Windows.

**Pre-existing suspects (present in 1.1.2 too):**

1. **Extraction relay URL size.** Results return as a navigation to
   `isct-companion-extract://<nonce>/<base64>`. `decide_navigation` accepts up
   to 11 MB (`crates/isct-core/src/school_browser/engine.rs:961`), but
   Chromium/WebView2 caps URLs at 2 MB. An LMS payload over about 1.5 MB of
   JSON would be dropped silently, and the capture would wait out its timeout.
   This fits "refresh never finishes" on full or current-quarter LMS pulls.
   教務Web payloads are small. Unverified on a Windows machine.
2. **Background throttling.** Hidden school views on Windows are 1×1 and
   hidden, without `BackgroundThrottlingPolicy::Disabled`, which only macOS
   sets (`apps/desktop/src-tauri/src/school_web.rs:37`). Chromium's
   hidden-page timer throttling therefore applies.
3. **Title relays** (`engine.rs` ~549, ~789) are capped at 4 KB by Chromium.
   Current payloads are small, so this is unlikely to matter.
4. **cecb98d** (one school.db commit path) holds IMMEDIATE transactions longer.
   On slow Windows disks this could surface as `database is locked`. This is
   speculative, but it is the only candidate that would also break syllabus,
   which uses reqwest rather than WebView2.

**Next step.** Get a 1.1.3 log from a Windows user: Settings → Diagnostics →
copy. Look for:

- `capture start … / capture loaded … target=` (`extraction.rs` ~455, ~476):
  which page each hidden load reached, and whether the relay arrived.
- `database is locked`: points to cecb98d.
- `[school-download] … refused`: download path mismatch.

**Done for suspects 1 and 2:**

- The relay now sends `<run>/<count>/<index>/<data>` parts of at most 1 MiB
  (`RELAY_PART`); Core takes them in order for the active run and asks the
  page for the next one (`__isctRelayPart`). The 11 MiB total cap is
  unchanged. This is platform-neutral, so Android WebView, also Chromium,
  gets the same fix.
- All school webviews, including the course-file downloader, use one profile
  (`school_web::school_profile`). On Windows it passes
  `--disable-background-timer-throttling --disable-renderer-backgrounding
  --disable-backgrounding-occluded-windows` and disables
  `IntensiveWakeUpThrottling`, keeping wry's default disabled features.

Suspects 3 and 4 are unchanged. If a Windows log still shows a failure, look
for `database is locked` first.

---

## Message to send Windows users (draft)

> If Windows says Smart App Control blocked the installer, it is because the
> Windows build is not code-signed yet; we are working on it. Until then the
> only way is to turn Smart App Control off (Windows Security → App & browser
> control).
> If the app says it cannot save files, check Windows Security → Ransomware
> protection → Controlled folder access and allow IsctCompanion.
> If school data does not load, open Settings → Diagnostics, copy the log and
> send it to us.
> Please download the next version manually from the Releases page; automatic
> updates from 1.1.3 may fail on slow connections.

Once the next release ships, the Controlled folder access line can point at
the in-app hint, and the first paragraph can go once the installer is signed.
