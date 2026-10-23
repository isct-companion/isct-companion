# Mobile distribution

A pushed `v<version>` tag starts two workflows beside the Desktop release:

| Workflow | Output | Where it goes |
|---|---|---|
| `android-release.yml` | `IsctCompanion_<version>_android.apk` | Attached to the GitHub release once the Desktop release workflow has published it; also kept as a run artifact |
| `ios-release.yml` | `IsctCompanion_<version>_<build>.ipa` | Uploaded to TestFlight; also kept as a run artifact |

Both build the Core SDK with the release profile from the tagged commit, take
the version from `package.json`, and can be started by hand (workflow dispatch)
to build without a tag.

## Android

Without signing secrets the workflow builds a debug-signed APK named
`…_android-debug.apk`. It installs, but every run signs it with a different
key, so it cannot update an earlier install. For releases, create one release
key and keep it forever: Android refuses updates signed by another key.

```bash
keytool -genkeypair -v -keystore isctcompanion-release.keystore -alias isctcompanion \
  -keyalg RSA -keysize 4096 -validity 36500
base64 -i isctcompanion-release.keystore | gh secret set ANDROID_KEYSTORE
gh secret set ANDROID_KEYSTORE_PASSWORD
gh secret set ANDROID_KEY_ALIAS --body isctcompanion
gh secret set ANDROID_KEY_PASSWORD
```

Back up the keystore and its passwords outside the repository.

The `versionCode` is derived from the version (`1.1.8` → `10108`), so each
release updates the previous one.

## iOS

Uploading is off until the repository variable is set:

```bash
gh variable set IOS_TESTFLIGHT_ENABLED --body true
```

Until then the job only checks that the app compiles for a device.

The workflow signs with Xcode's automatic signing for team `FB7AL944GK` and
reuses the App Store Connect API key the desktop notarization already reads
(`APPLE_API_KEY_ID`, `APPLE_API_ISSUER`, `APPLE_API_PRIVATE_KEY`). The key needs
the App Manager role (or Admin) to manage profiles and upload builds.

Optionally add the Apple Distribution certificate, so signing does not depend
on cloud-managed certificates (which need an Admin key). Export it from
Keychain Access as a `.p12` (*Apple Distribution: Zean Miao (FB7AL944GK)*,
certificate and private key), then:

```bash
base64 -i distribution.p12 | gh secret set IOS_DISTRIBUTION_CERTIFICATE
gh secret set IOS_DISTRIBUTION_CERTIFICATE_PASSWORD
```

Build numbers are the UTC time of the run (`YYYYMMDDHHMM`), so each upload is
newer than the last. The marketing version is the release version.
