# Desktop updater: source repository and local verification

Desktop detection and release manifests target `isct-companion/isct-companion`.
The desktop workflow is manual-only; pushing source or configuring secrets does
not request a desktop build. No repository visibility or licence was changed.
The source repository was already public when inspected.

The updater's public key is stored in Tauri configuration. The runtime and the
Tauri bundler therefore use the same key. CI can override that configuration
with the repository variable `ISCT_UPDATER_PUBLIC_KEY`; encrypted signing material
and its password are separate Actions secrets. Local encrypted key files are
ignored, and the local password is held in Keychain. No private key or password
is committed. `requireSignedVersion` binds the signed bundle to its announced
version. The current version is Tauri's package version, including local build
configuration overrides, rather than the Cargo manifest version.

A missing stable release is reported as unpublished, not as up to date. A debug
build can explicitly select a signed fixture service bound to `127.0.0.1`, with
artifact URLs restricted to that same origin and `/bundles/`. This fixture
selection is compiled out of release builds. It never disables signature checks.

## Local findings, 16 October 2026 (JST)

- Developer-ID-signed base `1.0.0` detected signed fixture `1.0.1-dev.1`.
- Changing one byte in the downloaded archive caused rejection; no install action
  was offered and the installed version stayed `1.0.0`.
- The unmodified archive passed signature/version verification and reached
  **Update ready to install**.
- Automatic updates saved correctly, but the original native Cmd-Q path exited
  without installing. Inspection of Cocoa's predefined Quit implementation
  showed that it calls `terminate:` directly, bypassing `ExitRequested`.
- The app menu now retains the native menu but routes Quit / Cmd-Q through
  `app.exit(0)`, which enters the owned save/shutdown/install path. Installation
  results are recorded in `updater-install-result.json` in app data.
- Both corrected local app bundles compiled and signed successfully. The corrected
  base is installed for the final retest. macOS locked before the final native UI verification could be observed,
  so the corrected automatic installation/reopen check remains pending.

Native screenshots and rollback copies are under ignored
`artifacts/updater-test/`. `rejected-update.png` and `update-ready.png` show the
completed stages. A consistent database backup and original application are
retained. Integrity checks passed; the compared records remained 63 stored
courses, 62 LMS courses, 49 grades, two plans and one course override.

No new Actions run, release publication, mobile build or notarization was
requested. This local development payload is not a published stable release.
Windows launch and signed macOS installation have separate manual checks; no
Windows result is inferred from a Mac run.

The shared repository did receive a separate push by `OHNope` at 22:15 JST,
which triggered completed run `38906471351`. This session did not push or
request that run. The manual-only workflow changes remain local until they are
explicitly pushed; they have not changed the remote workflow yet.

Code accounting for this source/signing follow-up (documentation excluded): production +187 / −74 lines; tests +42 / −0 lines; generated +0 / −0 lines. No test was removed. The settings suite now protects the unpublished-channel message; its scripted host supplies the native updater snapshot instead of an invalid null response.
