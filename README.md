# IsctCompanion

IsctCompanion is a local-first desktop workspace for Science Tokyo schedules,
courses, LMS content and school portals. React renders the interface; the
Tauri/Rust backend owns SQLite, school sessions, credentials, imports, files,
scheduling and cloud synchronization. The browser build is a read-only preview.

## Development

Requirements: Node.js, Yarn, Rust and the platform dependencies required by
Tauri.

```sh
yarn install
yarn dev             # browser preview on port 4173
yarn desktop:dev     # native app with the Rust backend
```

Production checks and builds:

```sh
yarn test
yarn build
yarn backend:check
yarn desktop:build
```

If port 4173 is occupied, Vite selects another port. The native development
configuration expects port 4173, so stop the old process before restarting the
desktop app.

## Repository map

- `src/pages`: Today, Courses, Web and Settings screens.
- `src/components`: shared UI.
- `src/store` and `src/storage`: frontend read model and native repository bridge.
- `backend/src`: authoritative Rust services and Tauri commands.
- `backend/subscriptions`: bundled public reference data.
- `docs`: maintained architecture, storage and provider notes.

## Data and security boundaries

- SQLite is the authoritative workspace store. The frontend does not persist a
  second writable workspace.
- School-owned data is refreshed from allowlisted official pages. Failed or
  partial imports keep the last valid rows.
- School credentials, OTP material, OAuth tokens and the optional cloud
  encryption password live in the operating-system credential vault and are
  never returned to the WebView.
- Google Drive and OneDrive store the readable workspace export as
  `IsctCompanion.json` in the provider's private app folder.
- Optional school-credential sync uses a separate AES-256-GCM encrypted file.
  Its 8–12 character encryption password stays on the local device and must be
  entered separately on another device.
- Site customization runs only inside allowlisted school WebViews and has no
  bridge to workspace data or native secrets.

Cloud provider registration is documented in
[docs/cloud-provider-setup.md](docs/cloud-provider-setup.md). Persistence
boundaries are documented in [docs/data-ownership.md](docs/data-ownership.md),
and school import rules in [docs/data-architecture.md](docs/data-architecture.md).

## Scope

The app reads school data and helps organize local plans. It does not submit
assignments, change enrollment, publish official calendar data or provide a
server-side polling service. Distribution signing, notarization and public
hosting remain release configuration.
