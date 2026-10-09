# Task 15 — Native cloud sync

Rust owns OAuth, provider requests, workspace export/import, version checks and
credential encryption. The WebView only starts high-level operations and shows
their status.

## Workspace file

Google Drive `appDataFolder` and the OneDrive app root contain one readable
workspace file named `IsctCompanion.json`:

```json
{
  "format": "IsctCompanion",
  "formatVersion": 1,
  "revision": 1,
  "updatedAt": "2026-10-06T00:00:00.000Z",
  "updatedBy": "device-id",
  "document": { "schemaVersion": 6, "kind": "IsctCompanion.local-backup" }
}
```

The document is a schema-v6 local export. Rust validates both layers before
restore. A backup that differs from the remote document returns an explicit
conflict until the user chooses the device or cloud version.

## Optional credential file

When enabled, Rust reads the validated school credential bundle from the OS
credential vault and uploads `IsctCompanion.credentials.json`. Its payload is
encrypted and authenticated with AES-256-GCM; the key is derived with
PBKDF2-SHA256 from the user's 8–12 character password. The password has its own
device-local vault entry and is never uploaded or returned to the WebView.

Cookies, OAuth tokens, raw HTML, diagnostics and downloaded files are never
included. The UI states that the app cannot guarantee credential security
beyond the described encryption.

Provider registration is documented in
[`docs/cloud-provider-setup.md`](../cloud-provider-setup.md). iCloud transport
is not implemented; native Keychain synchronization is a separate mechanism.
