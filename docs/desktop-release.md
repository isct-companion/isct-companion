# Desktop distribution

The source repository builds the app. The public repository
`isct-companion/Isct-Companion-App` holds only README and license documents;
Releases contain the three desktop installers, signed Android APK and signed
iOS/iPadOS IPA. Third-party notices and
licenses are bundled in the app.
The two repositories have independent Git histories.

## Preview builds

CI uses the Yarn version pinned in `package.json`, enabled through Corepack,
and installs dependencies with `yarn install --immutable`.

`.github/workflows/desktop-release.yml` builds Apple Silicon Mac, Intel Mac
and Windows x64. Pushes to `main`, version tags, and manual dispatches trigger
the build. Use **Desktop release → Run workflow**, with `publish` left
unchecked, to download private build artifacts. To publish an existing successful build without rebuilding, enable `publish`
and enter its Actions run ID in `source_run_id`. The workflow checks that
the source is a successful Desktop release run in this repository. During the
public preview, `publish_target=source` publishes in the source repository.
Select `publish_target=distribution` to use `Isct-Companion-App` and its
`PUBLIC_RELEASE_TOKEN`; that mode also updates its distribution documents.

macOS requires 14 or later. Windows installers target Windows 10/11 and install
WebView2 through its bootstrapper if needed. Intel Mac currently excludes
campus-card OCR because ONNX Runtime has no prebuilt Intel macOS library.
These are unsigned, unnotarized previews; build success does not establish
runtime compatibility on every supported OS.

## Enable automatic public releases

1. Create a fine-grained GitHub personal access token for resource owner
   `isct-companion`, selecting **only** `Isct-Companion-App`, with **Contents:
   read and write**. Approve it in the organization if required.
2. Save it as `PUBLIC_RELEASE_TOKEN` in the **private** repository's Actions
   secrets. Do not paste it into source files or chat. Set an expiration and
   rotate the secret before it expires.
3. Merge the release workflow into the branch used for releases. Update the
   matching versions in `package.json`, `backend/Cargo.toml` and
   `backend/tauri.conf.json`.
4. Push a version tag such as `v0.1.1`. Alternatively, dispatch the workflow
   with `publish` enabled. The version guard rejects mismatched tags.

All desktop and mobile builds, including the TestFlight upload, must succeed
before publication. Only `.dmg`/`.exe` installers, `.apk` and `.ipa` packages
are uploaded as Release assets. See [mobile releases](mobile-release.md) for
mobile signing and TestFlight setup.
The job creates a draft, uploads the complete set, then publishes it as a
prerelease. An existing published version is protected by default. An explicit
manual dispatch with `replace_existing=true` can refresh that version's assets.
Source-repository releases point their version tag at the verified build commit.
Distribution-repository releases keep their independent document history; no
source code is copied there, and their automatic source archives contain only
the distribution documents.

The public repository must already have a `main` branch. The initial README
and licenses establish it. Public document sources are
`docs/distribution/README.md` and `docs/legal/EULA{,.ja}.md`.

## OAuth and signing

The desktop wrapper reads OAuth client IDs from `config/oauth-clients.yaml`.
CI supports `ISCTCOMPANION_GOOGLE_CLIENT_ID` and
`ISCTCOMPANION_GOOGLE_DESKTOP_CLIENT_SECRET` as overrides. Native desktop
OAuth configuration is included in the application binary and cannot be
kept confidential from its recipients; use the provider's desktop-client
registration rather than a confidential server credential.

### Apple Developer signing and notarization

For direct DMG downloads, use a paid Apple Developer Program membership and
an exportable **Developer ID Application** certificate with its private key.
Export the identity from Keychain Access as a password-protected `.p12`.
Encode it using `openssl base64 -A -in certificate.p12 -out certificate-base64.txt`.

Add these Actions secrets to the **private** repository:

| Secret                       | Value                                                   |
| ---------------------------- | ------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64 contents of the `.p12` file                      |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12`                 |
| `APPLE_SIGNING_IDENTITY`     | Full `Developer ID Application: Name (TEAMID)` identity |
| `APPLE_MACOS_PROVISIONING_PROFILE` | Base64 contents of the Developer ID provisioning profile |
| `APPLE_API_KEY_ID`           | App Store Connect team API key ID                       |
| `APPLE_API_ISSUER`           | App Store Connect issuer ID                             |
| `APPLE_API_PRIVATE_KEY`      | Contents of the matching `.p8` team API private key     |

The provisioning profile must be a macOS **Developer ID** profile for
`app.isctcompanion` and authorize both `iCloud.app.isctcompanion` and the
`FB7AL944GK.app.isctcompanion` Keychain access group. Encode it with
`openssl base64 -A -in profile.provisionprofile -out profile-base64.txt`.
The build validates the profile and embeds it as
`Contents/embedded.provisionprofile`; this is required for CloudKit and
synchronizable Keychain access in the distributed desktop app.

Select `sign_macos` when dispatching a new build. For automatic signed tag
releases, also set the repository Actions variable `MACOS_SIGNING_ENABLED`
to `true`. A signed build fails if any required secret is missing; it never
falls back to unsigned. Tauri signs, notarizes and staples each app. The workflow also submits and
staples each DMG, then verifies the app signature, Gatekeeper assessment
and stapled tickets before collecting artifacts. Reusing a source run preserves that
run's signing status and cannot add signatures to its existing installers.
Windows signing remains unconfigured.

References: [Apple certificate creation](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/),
[Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/),
[Apple app-specific passwords](https://support.apple.com/en-us/102654).
