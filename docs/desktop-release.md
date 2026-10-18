# Desktop distribution

The source repository `isct-companion/isct-companion` builds and publishes desktop
Releases for macOS 14+ (Apple Silicon and Intel) and Windows 10/11 x64. Mobile
release pipelines remain separate and are not part of this workflow.

## Build and publish

CI uses the Yarn version pinned in `package.json`, enabled through Corepack,
and installs dependencies with `yarn install --immutable`.

The Desktop release workflow runs on version tag pushes and by manual dispatch. Update the matching versions
in `package.json`, `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml`,
`Cargo.lock` and `apps/desktop/src-tauri/tauri.conf.json`, including its macOS
`bundleVersion`. Add user-facing notes in `docs/releases/<version>.md`. After review,
commit and push the changes, then push a matching version tag:

```sh
git tag -a v1.1.2 -m "IsctCompanion 1.1.2"
git push origin v1.1.2
```

Any maintainer with permission to push that tag can start a release. Actions uses
repository secrets for signing and its scoped `GITHUB_TOKEN` for publication;
maintainers do not need the owner's local credentials. Existing repository tag
rules still apply. A tag must match the application version. Tests and type checks
run before the three installer builds; every build must succeed before publication.
Tags pushed by another workflow's `GITHUB_TOKEN` do not trigger this workflow;
use a human maintainer's normal Git authentication or a GitHub App token.

Manual dispatch remains available with `publish=true`. Leave
`replace_existing=false` for new versions. To prepare a review build, dispatch
with `publish=false` and push the version tag only after approval.

To build without publishing, leave `publish=false`. To publish an existing successful
Desktop release build without rebuilding, set `source_run_id` to its Actions run ID.
The workflow validates that run before using its artifacts.

All three desktop builds must succeed before publication. Publication uses the source
repository's `GITHUB_TOKEN`: it creates a draft, uploads installers and signed updater
bundles, then publishes the complete release. Stable versions are marked Latest;
versions with a prerelease suffix remain prereleases. Release notes record the source
commit. Existing published releases require explicit `replace_existing=true`.

When `ISCT_UPDATER_PUBLIC_KEY` is configured, every platform must supply a signed
update bundle using `TAURI_SIGNING_PRIVATE_KEY`. The complete set produces
`latest.json`, consumed by desktop update checks from this source repository.

Mac signing and notarization are controlled by `sign_macos` or
`MACOS_SIGNING_ENABLED`. Windows signing remains unconfigured. Intel Mac currently
excludes campus-card OCR because ONNX Runtime has no prebuilt Intel macOS library.
Build success does not establish runtime compatibility on every supported OS.

Installer document sources are `docs/distribution/README.md` and
`docs/legal/EULA{,.ja}.md`. Third-party notices are bundled in the app.

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

GitHub references: [tag push events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#push),
[token permissions](https://docs.github.com/en/actions/security-for-github-actions/security-guides/automatic-token-authentication).
