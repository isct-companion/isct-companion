# Mobile preview builds

Version tags (`v*`) run Desktop release, which calls Android build and iOS and
iPadOS build alongside the desktop jobs. Publication waits for all platforms
and the TestFlight upload to succeed. Both mobile workflows can also be
dispatched manually.

The original app icon is `public/icon.png`. Regenerate native icons with
`yarn tauri icon public/icon.png --output backend/icons`.
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
Keychain access group; see [cloud provider setup](cloud-provider-setup.md).
Public installers, the signed APK and IPA are published together in the
distribution repository. Its README records TestFlight invitation availability.
Unsigned archives remain in the private repository's Actions runs.
