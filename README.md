# IsctCompanion

IsctCompanion is a local-first desktop workspace for Science Tokyo schedules,
courses, LMS content and school portals. React renders the interface; the
shared Rust Core owns SQLite, domain rules and mutations, school sessions,
credentials, imports, files, scheduling and cloud synchronization. Tauri supplies
native platform adapters and the desktop MCP transport. The browser build is a read-only preview.

## Development

Requirements: Node.js 22+, Yarn 4 (via Corepack), Rust and the platform dependencies required by
Tauri.

```sh
corepack enable
yarn install --immutable
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
- `crates/isct-core`: shared domain services, mutations, read models and task ownership.
- `crates/isct-ocr`: independent image preparation and optional OCR engine.
- `backend/src`: Tauri commands, native platform ports and local HTTP MCP.
- `src/generated`: TypeScript contracts generated from Rust.
- `crates/isct-core/tests/fixtures/subscriptions`: frozen reference-data test fixtures.
- `docs`: maintained architecture, storage and provider notes.

## Desktop Core checkpoint

Desktop consolidation through Phase 5 is complete. The frontend renders Core
read models, caches snapshots and forwards typed intents; Rust owns validation,
revisions and domain state. Academic date calculations use JST, and stored
plan timestamps use explicit offsets. The signed macOS checkpoint and its
verification coverage are recorded in
[Core consolidation checkpoints](docs/verification/core-consolidation.md).

[Shared Core API and protocol](docs/core-api.md) documents desktop/native transport
mapping, records, initialization, services, ports, read models, mutations, errors
and shutdown. See the [documentation index](docs/README.md). The desktop MCP adapter
shares those services at `http://127.0.0.1:47832/mcp` while the app is open; authentication
is configured in Settings → AI agent access.

Phase 6 (Swift/Kotlin bindings, native packages and mobile smoke tests) is deferred.
It is not a prerequisite for the current desktop checkpoint. Native mobile UI
redesign and distribution/release checks remain separate work.

Debug builds display the base version plus a UTC build timestamp and Git revision.
`dirty` records uncommitted working-tree changes at build time, including documents.
The displayed version stays fixed after the build is committed.

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
