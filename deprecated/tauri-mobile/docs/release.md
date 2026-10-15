> Archived Tauri mobile implementation. The workflows here are inactive and the
> build procedures below describe the retired shell. Native SwiftUI/Compose
> applications and their release pipelines will be implemented separately.

# Mobile preview builds

Version tags (`v*`) run Desktop release, which calls Android build and iOS and
iPadOS build alongside the desktop jobs. Publication waits for all platforms
and the TestFlight upload to succeed. Both mobile workflows can also be
dispatched manually.

The unchanged original app artwork is `resources/design/icon.png`.
`yarn icons:desktop` generates transparent rounded Windows/web icons and an
inset rounded macOS icon using Tauri's SVG renderer. It leaves mobile assets
unchanged: iOS/iPadOS and Android adaptive icons receive their shape from the OS.
Regenerate mobile assets from `resources/design/icon.png`, never the rounded
`apps/desktop/public/icon.png`, with `yarn tauri icon resources/design/icon.png --output apps/desktop/src-tauri/icons`;
then run `yarn icons:desktop` again to restore the desktop shapes.
iOS source PNGs must be opaque RGB without an alpha channel. The project
patcher validates and copies these committed icons into every fresh Xcode
project; convert the generated iOS PNGs to RGB after regenerating them.

## Android

`Android build` generates an arm64 release APK for Android 14 or later and
checks its platform-specific OAuth configuration. Download the
`mobile-android-aarch64` Actions artifact or the APK in the public prerelease.
The workflow signs the APK with the persistent key configured in
`ANDROID_KEYSTORE` (base64), `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_PASSWORD`
and `ANDROID_KEY_ALIAS` repository secrets.

## iOS and iPadOS

`iOS and iPadOS build` archives the same app for iPhone and iPad (iOS/iPadOS 17
or later), verifies both device families and checks platform isolation. By
default on a manual dispatch, it uploads an unsigned `.xcarchive.zip` Actions
artifact. Tagged releases build a signed IPA, upload it to TestFlight and
include the IPA in the public prerelease. The App Store-signed IPA requires
TestFlight or another authorized Apple distribution channel to install.

To build a signed IPA and upload to TestFlight, dispatch with
`upload_testflight` enabled, or set the repository variable
`IOS_TESTFLIGHT_ENABLED=true` for tagged releases. Configure these Actions
secrets first: `IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD`,
`IOS_MOBILE_PROVISION`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, and
`APPLE_API_PRIVATE_KEY`. Signed builds fail if a required secret is missing.

The provisioning profile must authorize the app's CloudKit container and
Keychain access group; see [cloud provider setup](../../../docs/cloud-provider-setup.md).
Public installers, the signed APK and IPA are published together in the
distribution repository. Its README records TestFlight invitation availability.
Unsigned archives remain in the private repository's Actions runs.
