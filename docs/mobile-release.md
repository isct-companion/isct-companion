# Mobile preview builds

Version tags (`v*`) build Android and a universal iOS/iPadOS archive alongside
the desktop prerelease. Both workflows can also be dispatched manually.

The original app icon is `public/icon.png`. Regenerate native icons with
`yarn tauri icon public/icon.png --output backend/icons`.

## Android

`Android build` generates an arm64 release APK for Android 14 or later and
checks its platform-specific OAuth configuration. Download the
`android-aarch64-unsigned` Actions artifact. The APK is unsigned and needs a
persistent Android signing key before it can be installed or distributed.

## iOS and iPadOS

`iOS and iPadOS build` archives the same app for iPhone and iPad (iOS/iPadOS 17
or later), verifies both device families and checks platform isolation. By
default, it uploads an unsigned `.xcarchive.zip` Actions artifact. This
archive must be signed before installation on a device.

To build a signed IPA and upload to TestFlight, dispatch with
`upload_testflight` enabled, or set the repository variable
`IOS_TESTFLIGHT_ENABLED=true` for tagged releases. Configure these Actions
secrets first: `IOS_CERTIFICATE`, `IOS_CERTIFICATE_PASSWORD`,
`IOS_MOBILE_PROVISION`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, and
`APPLE_API_PRIVATE_KEY`. Signed builds fail if a required secret is missing.

The provisioning profile must authorize the app's CloudKit container and
Keychain access group; see [cloud provider setup](cloud-provider-setup.md).
Desktop publication remains in the public distribution repository; mobile
unsigned artifacts remain in the private repository's Actions runs.
