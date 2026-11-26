# Desktop distribution

The source repository `isct-companion/isct-companion` builds and publishes desktop
Releases for macOS 15+ (Apple Silicon and Intel) and Windows 10/11 x64. The same
version tag also starts the Android and iOS release workflows; see
[mobile-release.md](mobile-release.md).

## Build and publish

CI uses the Yarn version pinned in `package.json`, enabled through Corepack,
and installs dependencies with `yarn install --immutable`.

Work on a `release/X.Y.Z` branch from `main` and use
`yarn release:bump X.Y.Z` to update the version files and print the changes
since the previous tag. Add `docs/releases/X.Y.Z.md` with the release notes.

Push the branch and manually run **Release checks** on it in Actions:

```sh
gh workflow run release-checks.yml --ref release/X.Y.Z
```

This checks the version and notes and runs the full desktop, iOS, and Android
suite. After it passes, open the release PR to `main` and squash merge once
`Platform checks` passes. If the PR was already open, rerun its failed Route
job after **Release checks** completes. If the branch content needs to change
to catch up with `main`, rerun **Release checks** on the resulting commit.

**Tag merged release** compares the squash commit's source tree with the
tested candidate, creates `vX.Y.Z` on the squash commit, and starts Desktop,
Android and iOS release jobs on that tag. Each publishing workflow verifies
the tag and successful candidate run before it signs or uploads anything.
Claude Code follows the same flow through the `release` skill.

For a build without publishing, dispatch **Desktop release** with
`publish=false`. To retry publishing select the existing `vX.Y.Z` tag with
`publish=true`. Supply `source_run_id` to reuse a successful Desktop release
build for the same tagged commit.

All three desktop builds must succeed before publication. Publication uses the source
repository's `GITHUB_TOKEN`. The Desktop workflow opens the version's draft after validating the source,
while the desktop builds run, so the iOS and Android workflows can attach their builds
whatever happens to the desktop runners; a desktop failure then needs only a desktop
rerun. Publication writes the notes on that draft (or creates it for a manual
dispatch), uploads installers and signed updater bundles, then publishes the
complete release. The published version is marked Latest. Release notes record the source commit. Existing published releases require explicit `replace_existing=true`.

When `ISCT_UPDATER_PUBLIC_KEY` is configured, every platform must supply a signed
update bundle using `TAURI_SIGNING_PRIVATE_KEY`. The complete set produces
`latest.json`, consumed by desktop update checks from this source repository.

Mac signing and notarization are controlled by `sign_macos` or
`MACOS_SIGNING_ENABLED`; Windows signing by `sign_windows` or
`WINDOWS_SIGNING_ENABLED`. Both Mac builds read the campus card with the
system's Core ML, so the Intel build includes OCR too.
Build success does not establish runtime compatibility on every supported OS.

Installer document sources are `docs/distribution/README.md` and
`LICENSE` (GPL-3.0) with `docs/legal/NOTICE{,.ja}.md`. Third-party notices are bundled in the app.

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

Add these Actions secrets to the source repository:

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

macOS bundles require Xcode and XcodeGen for the WidgetKit extension.
The extension is signed before Tauri places it in `Contents/PlugIns`.
Campus VPN is built for Apple Silicon and Windows x64.
For a standalone development bundle, run
`yarn desktop:build --debug --bundles app`. The app and widget use the same
Apple Development identity as `yarn desktop:dev`. A Mac with one valid
development identity uses it automatically. When several are installed, set
`ISCT_MACOS_DEV_SIGNING_IDENTITY` to the SHA-1 of the chosen certificate.
The app and widget share an App Group within that team, and the bundled
campus VPN helper uses the same signing identity.

To exercise the release app's CloudKit and Keychain in a debug build, set
`ISCT_MACOS_PROVISIONING_PROFILE` to the path of a project-team development
profile that authorizes this Mac and install its matching Apple Development
certificate. Then run `yarn desktop:build --debug --bundles app`. This
explicitly builds `app.isctcompanion` against the release app's data and
credentials. A regular debug build uses the separate development identity.

Select `sign_macos` when dispatching a new build. For automatic signed tag
releases, also set the repository Actions variable `MACOS_SIGNING_ENABLED`
to `true`. A signed build fails if any required secret is missing; it never
falls back to unsigned. Tauri signs, notarizes and staples each app. The workflow also submits and
staples each DMG, then verifies the app signature, Gatekeeper assessment
and stapled tickets before collecting artifacts. Reusing a source run preserves that
run's signing status and cannot add signatures to its existing installers.

### Windows signing (Azure Artifact Signing)

An unsigned Windows installer is blocked outright by Smart App Control on
Windows 11, and Controlled folder access refuses its writes to Documents,
where course files live by default. Sign with an Azure Artifact Signing
account (formerly Trusted Signing) and a Public Trust certificate profile;
check identity-validation eligibility for the publisher's region first.

Create an Entra app registration with the *Artifact Signing Certificate
Profile Signer* role on the account, then add:

| Name | Kind | Value |
| ---- | ---- | ----- |
| `AZURE_CLIENT_ID` | secret | App registration client ID |
| `AZURE_CLIENT_SECRET` | secret | App registration client secret |
| `AZURE_TENANT_ID` | secret | Entra tenant ID |
| `AZURE_ARTIFACT_SIGNING_ENDPOINT` | variable | Account endpoint, e.g. `https://eus.codesigning.azure.net` |
| `AZURE_ARTIFACT_SIGNING_ACCOUNT` | variable | Signing account name |
| `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE` | variable | Certificate profile name |

Select `sign_windows` when dispatching, or set `WINDOWS_SIGNING_ENABLED` to
`true` for tag releases. `scripts/desktop/tauri.mjs` passes
`artifact-signing-cli` to Tauri as `bundle.windows.signCommand`, so the app
binary and the NSIS installer are both signed. The workflow fails if a
required value is missing and verifies both signatures before collecting
artifacts. A new certificate builds SmartScreen reputation over time; Smart
App Control accepts a valid trusted signature immediately.

References: [Apple certificate creation](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/),
[Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/),
[Apple app-specific passwords](https://support.apple.com/en-us/102654).

GitHub references: [tag push events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#push),
[token permissions](https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication).
