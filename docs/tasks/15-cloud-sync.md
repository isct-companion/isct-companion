# Task 15 — Plaintext cloud workspace sync

## Goal

Synchronize the task-1 export through a provider-owned private app folder. The
feature is for convenience between the user's devices, not an additional
security boundary.

## Security and storage contract

- Store one readable JSON file named `IsctCompanion.json`. Do not encrypt its
  contents and do not ask for or retain a recovery phrase.
- State this clearly in Settings: the app does not guarantee confidentiality
  of the cloud copy. Access control, account recovery and storage security are
  supplied by Google Drive, OneDrive, or a future iCloud transport.
- OAuth access and refresh tokens remain in the operating-system credential
  vault. School credentials, cookies, OTP material, raw HTML, diagnostics and
  downloaded course files are never included.
- This is an internal-development format. Do not migrate the former encrypted
  `backup.bin` / `_meta.json` pair; it may be discarded.

## File format

The provider transport atomically reads or replaces one document:

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

`document` is the exact local export. `revision`, `updatedAt` and `updatedBy`
are transport metadata used for optimistic conflict detection. Validate the
entire wrapper and export before importing or replacing a remote copy.

## Providers

- Google Drive: `appDataFolder`, OAuth scope `drive.appdata`.
- OneDrive: app root, OAuth scope `Files.ReadWrite.AppFolder`.
- iCloud: use the same file format when a native ubiquity-container transport
  is added. It is not exposed as available until that transport exists.

Provider-owner registration steps are in
[`docs/cloud-provider-setup.md`](../cloud-provider-setup.md).

## Conflict behavior

- A client records the revision it last pulled.
- A push to an existing file without a prior pull is a conflict.
- A push fails if the remote revision changed after the pull.
- The existing Keep this device / Use latest / Export first UI resolves the
  conflict; invalid remote data never replaces local data.

## Acceptance

- A first push writes one valid, human-readable `IsctCompanion.json` file.
- Pull accepts the current schema-v6 export and rejects malformed or obsolete
  cloud files without changing local data.
- A stale or blind push raises an explicit conflict.
- No encryption, cipher, recovery-key UI, `backup.bin`, or `_meta.json` code
  remains.
- TypeScript build and cloud adapter tests pass.
