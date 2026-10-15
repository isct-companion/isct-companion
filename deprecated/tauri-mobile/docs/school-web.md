> Archived Tauri mobile implementation. The workflows here are inactive and the
> build procedures below describe the retired shell. Native SwiftUI/Compose
> applications and their release pipelines will be implemented separately.

# Mobile school WebView

The mobile shell uses the desktop command names and result objects. Both adapters
implement the native `SchoolBrowser` port for the one `isct-core` school-browser
engine. Core owns login scripts, authenticated relay nonces, session checks,
ingestion jobs, URL history and restore policy; adapters own native views,
presentation and callbacks. See [Core school browser](../../../docs/core-school-browser.md)
for the dispatch and callback contract. Mobile reports `schoolWeb` and
`schoolImport` as supported.

## Native adapters

- iOS creates a wry child WKWebView on UIKit's main thread, with the existing
  school data-store identifier. Title, navigation, load, and popup callbacks enter
  Core through the native browser port, so native credential calls
  can use the main queue without blocking a WebKit delegate. wry's iOS
  `set_bounds` does not set the child frame, so
  the adapter sets its UIKit frame directly. Bounds are converted from the main
  WebView's origin and clipped to the parent's safe area.
- Android uses the local `tauri-plugin-school-web` Kotlin plugin. Each school
  view is an ordinary Android WebView without the app's Tauri IPC bridge. Its
  navigation delegate uses Core's Rust allowlist; Core validates relay nonces.
  Scripts run through `evaluateJavascript` after page load, including existing
  and later same-origin popup frames. Native events enter
  a serialized Rust worker, which can invoke Kotlin without blocking Android's
  UI callback. Bounds include the main WebView scale, overlay origin, system
  bars, cutouts, and keyboard insets.
- Hiding preserves each native view and its document, scroll position, and
  cookies. Showing an existing view resumes it without navigating. Hiding also
  dismisses the native keyboard.

## Native projects

`deprecated/tauri-mobile/gen` is generated and ignored. Initialize both projects from the
repository root, with Xcode and Android's SDK/NDK installed:

```sh
corepack yarn tauri ios init --ci
corepack yarn tauri android init --ci
```

Android builds need `JAVA_HOME`, `ANDROID_HOME`, and `NDK_HOME`. This checkout was
initialized with Java 17 and NDK 28.2.13676358. Its generated app uses compile/target
SDK 36, matching the plugin; if a newer CLI generates SDK 37, install that SDK or
set those two values to 36 in the generated app's `build.gradle.kts`.

## Physical iPhone build

Tauri CLI 2.12.2 checks for an iOS simulator runtime even for device builds. If
only the device SDK is installed, use its Xcode mode and build the device target
directly. This does not install or run a simulator. Keep the first command running
while running `xcodebuild` in another terminal. Replace the team and device IDs:

```sh
corepack yarn vite build
corepack yarn tauri ios dev --open --no-watch \
  --config '{"bundle":{"iOS":{"developmentTeam":"TEAM_ID"}},"build":{"devUrl":null,"beforeDevCommand":""}}' \
  --features tauri/custom-protocol
```

```sh
xcodebuild -project deprecated/tauri-mobile/gen/apple/isct-companion.xcodeproj \
  -scheme isct-companion_iOS -configuration debug \
  -destination 'id=DEVICE_UDID' -derivedDataPath /tmp/isct-device-build \
  -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
  DEVELOPMENT_TEAM=TEAM_ID build
xcrun devicectl device install app --device DEVICE_UDID \
  /tmp/isct-device-build/Build/Products/debug-iphoneos/IsctCompanion.app
xcrun devicectl device process launch --device DEVICE_UDID app.isctcompanion
```

For a personal development team, iOS may require trusting the developer App in
Settings → General → VPN & Device Management before its first launch.

## Verification

```sh
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked
corepack yarn test --maxWorkers=2
cd deprecated/tauri-mobile/gen/android
./gradlew :tauri-plugin-school-web:testDebugUnitTest
```

The bounds tests cover toolbar space, an already inset shell, landscape cutouts,
and keyboard clipping. End-to-end verification needs school credentials entered
inside the app:

1. Save the Science Tokyo and legacy school credentials in Accounts.
2. Open LMS and 教務 Web; confirm built-in login completes and account status
   returns to the app.
3. Navigate within the school site, scroll, and enter a harmless form value.
4. Leave Web: the native view and keyboard must disappear. Return: the document,
   scroll position, and form value must remain.
5. Rotate the device and show the keyboard; the school view must stay below the
   toolbar and inside the usable screen area.
6. Run a school pull and confirm the existing progress and completion events
   update the app's data.

Mobile download/PDF handling is outside this adapter's current interface. It
retains the desktop download callbacks on desktop; these callbacks are not
available on the mobile native views.

## Validation record (2026-10-11)

- Rust desktop tests: 184 passed, 3 ignored, including the existing school login
  and relay tests and backend acceptance suites.
- Frontend suite: 291 passed, including the added Android frame-support
  lifecycle test. TypeScript build and `git diff --check` passed.
- Kotlin plugin compiled; four bounds/safe-area unit tests passed.
- Android arm64 debug APK rebuilt successfully with the automatic-splitting
  fix. No Android device/runtime test has been performed.
- iPhone arm64 debug build and signing verification succeeded; the profile
  contains the connected iPhone. The app was installed and the developer was
  trusted. A production-bundle initialization failure was reproduced on the
  device: manual vendor splitting produced a circular React dependency and
  called its uninitialized export. Using Vite's default automatic splitting
  removed static chunk cycles; the mobile home screen now renders on the device.
  Temporary diagnostic instrumentation was removed, and the clean version
  was rebuilt, installed, and launched. Login, status relay, native bounds, and
  hide/resume still need physical-device verification with credentials entered
  inside the app.
- No simulator was run.
