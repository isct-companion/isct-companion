# Cloud provider registration

The repository does not contain shared OAuth credentials. A project owner must
register the public desktop clients and paste their client IDs into **Settings →
Developer options → Cloud sign-in** (turn on Developer options first). Do not
create or embed a client secret.

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

The registered IsctCompanion desktop client ID is compiled into the app from
`VITE_GOOGLE_DRIVE_CLIENT_ID`. GitHub Actions reads it from the repository
secret `ISCTCOMPANION_GOOGLE_CLIENT_ID`. The Settings field remains available
only as a development override; no client secret is compiled or required.

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

iCloud transport is not implemented and must not be shown as available. Native
Keychain synchronization is separate from the Google Drive/OneDrive encrypted
credential file described above.
