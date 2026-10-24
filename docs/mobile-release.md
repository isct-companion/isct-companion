# Mobile distribution

A pushed `v<version>` tag starts two workflows beside the Desktop release:

| Workflow | Output | Where it goes |
|---|---|---|
| `android-release.yml` | `IsctCompanion_<version>_android.apk` | Attached to the GitHub release once the Desktop release workflow has published it; also kept as a run artifact |
| `ios-release.yml` | `IsctCompanion_<version>_<build>.ipa` | Uploaded to TestFlight; also kept as a run artifact |

Both build the Core SDK with the release profile from the tagged commit, take
the version from `package.json`, and can be started by hand (workflow dispatch)
to build without a tag.

A manual iOS run signs, archives and exports the IPA without uploading it.
Select `upload_testflight` to upload that run; version tags upload automatically
when `IOS_TESTFLIGHT_ENABLED` is true.

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

Google Drive authorization matches the APK's package and signing certificate.
The release client in `config/oauth-clients.yaml` is registered for
`app.isctcompanion` with SHA-1
`8D:A3:F4:A1:0E:BC:06:12:74:C7:D6:58:3B:B3:9E:0B:13:8E:A1:8C`.
Keep the separate debug client for local builds. If distributing through Google
Play App Signing, register its app signing certificate as another Android client
before testing the Play-delivered APK. Check `ANDROID-DRIVE-SIGNIN` with the
release APK and an account outside the OAuth test-user list before release.

## iOS

Uploading is off until the repository variable is set:

```bash
gh variable set IOS_TESTFLIGHT_ENABLED --body true
```

Until then the job only checks that the app compiles for a device.

The workflow uses manual signing for team `FB7AL944GK` and three bundle IDs,
the app `app.isctcompanion` and its extensions `app.isctcompanion.share` and
`app.isctcompanion.open`, using these repository secrets:

| Secret | Content |
|---|---|
| `IOS_CERTIFICATE` | Base64 Apple Distribution `.p12`, including its private key |
| `IOS_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `IOS_MOBILE_PROVISION` | Base64 App Store distribution `.mobileprovision` for the app (`app.isctcompanion`) |
| `IOS_SHARE_MOBILE_PROVISION` | Base64 App Store distribution `.mobileprovision` for the share extension (`app.isctcompanion.share`) |
| `IOS_OPEN_MOBILE_PROVISION` | Base64 App Store distribution `.mobileprovision` for the "Open in" action extension (`app.isctcompanion.open`) |
| `APPLE_API_KEY_ID` | App Store Connect API key ID |
| `APPLE_API_ISSUER` | API issuer ID |
| `APPLE_API_PRIVATE_KEY` | Raw `.p8` private key used to upload builds |

All three profiles are made with the same Apple Distribution certificate, and
their App IDs have the App Group `group.app.isctcompanion` (the extensions hand
shared files to the app through it). A profile made before its App ID had the
group lacks the entitlement: regenerate it after ticking App Groups.

The job fails before building if a secret is missing, a profile has expired,
belongs to another app/team, is not an App Store profile or lacks the App
Group, or the certificate does not match all three profiles.
It installs the supplied certificate and profile, then explicitly selects them
for both archive and export. It does not create cloud-managed certificates or
refresh profiles. Renew the existing secrets when their signing materials expire.

After upload, wait for Apple to process the build. Assign it to a TestFlight
group (or enable automatic distribution for an internal group); external testing
also requires Beta App Review. Check `IOS-TESTFLIGHT-RELEASE` in the manual checks.

Build numbers are the UTC time of the run (`YYYYMMDDHHMM`), so each upload is
newer than the last. The marketing version is the release version.
