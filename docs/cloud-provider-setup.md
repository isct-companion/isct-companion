# Cloud provider registration

The registered OAuth clients live in `config/oauth-clients.yaml` (see Google
Drive below). Google Drive is available on desktop and iOS; iCloud is available
on signed Apple builds. OneDrive remains a separately registered provider.

The native app uses this exact loopback redirect:

```text
http://127.0.0.1:42813/oauth/callback
```

Use these names consistently when registering providers:

- product / OAuth application name: `IsctCompanion`;
- native application identifier and Keychain service prefix: `app.isctcompanion`;
- iCloud container identifier: `iCloud.app.isctcompanion`;
- workspace file: `IsctCompanion.json`;
- optional encrypted school-credential file: `IsctCompanion.credentials.json`.

## Google Drive

1. Create or select a Google Cloud project and enable the Google Drive API.
2. Configure the OAuth consent screen. While the app is internal development,
   keep it in Testing and add each developer account as a test user.
3. Create an OAuth client ID with application type **Desktop app**.
4. Copy the client ID into the app. Desktop clients support loopback redirects;
   the console does not require this URI to be entered as an authorized web
   redirect.

The registered IsctCompanion client IDs (desktop, iOS, Android) are committed
in `config/oauth-clients.yaml`. Vite compiles in the one matching Tauri's
`TAURI_ENV_PLATFORM`, and `scripts/tauri.mjs` writes the derived iOS callback
scheme into `backend/Info.ios.plist` (generated, ignored), which Tauri merges
into the iOS app's Info.plist. OAuth client IDs are public and appear in every authorization URL. `VITE_GOOGLE_DRIVE_CLIENT_ID` and the Settings field remain
available only as development overrides. Google's token endpoint also
requires the Desktop client's secret, even with PKCE; set it as
`google-drive.desktop-secret` in the YAML (or `ISCT_GOOGLE_DESKTOP_CLIENT_SECRET`
in the environment). `yarn tauri …` compiles it into the native backend, which
sends it only with the registered desktop client ID. Google documents this
secret as non-confidential for installed apps; it ships only inside desktop
binaries. Rust target conditions exclude the desktop OAuth flow and secret from
iOS/Android, while the iOS Google Sign-In plugin is an iOS-only dependency.
`scripts/verify-platform-bundle.mjs` fails release builds if another platform's
OAuth configuration leaks into an artifact.

The app requests only `https://www.googleapis.com/auth/drive.appdata`, the
non-sensitive scope for the hidden application-data folder. It also requests
offline access so the OS credential vault can retain a refresh token.

Official references: [OAuth for installed apps](https://developers.google.com/identity/protocols/oauth2), [loopback redirects for desktop clients](https://developers.google.com/identity/protocols/oauth2/resources/loopback-migration), [Drive app data folder](https://developers.google.com/workspace/drive/api/guides/appdata).

Registration: [enable Google Drive API](https://console.cloud.google.com/apis/library/drive.googleapis.com), then [create the OAuth client](https://console.cloud.google.com/auth/clients).

## Microsoft OneDrive

1. Create an app registration in Microsoft Entra ID.
2. Choose the account audience needed by the project. For both university and
   personal OneDrive accounts, choose organizational directories plus personal
   Microsoft accounts.
3. Under **Authentication**, add **Mobile and desktop applications** and the
   exact loopback redirect shown above, then enable **Allow public client
   flows**.
4. Under **API permissions**, add the delegated Microsoft Graph permission
   `Files.ReadWrite.AppFolder`. No application permission and no client secret
   are needed.
5. Copy the Application (client) ID into the app.

The app additionally requests `offline_access` so its refresh token can remain
in the OS credential vault. The Graph permission is limited to this app's
OneDrive folder.

Official references: [desktop public-client setup](https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration), [redirect URI rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url), [OneDrive app folder](https://learn.microsoft.com/en-us/graph/onedrive-sharepoint-appfolder), [Graph permission reference](https://learn.microsoft.com/en-us/graph/permissions-reference).

Registration: [Microsoft Entra app registrations](https://entra.microsoft.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade).

## iCloud

iCloud uses the private CloudKit database in `iCloud.app.isctcompanion`; it does
not use OAuth or expose the user's files in iCloud Drive. Workspace data and
user preferences travel in the same private backup document. Device-only state
stays local. School credentials use the Apple Keychain with synchronizable
items enabled, so Apple devices signed into the same iCloud Keychain can share
them without placing credentials in the CloudKit document.

The App ID, entitlements and provisioning profile must all contain the same
iCloud container. The App Store profile must permit CloudKit production and
have `beta-reports-active` for TestFlight.
