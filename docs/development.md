# Development guide

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
yarn test                              # unit + e2e suites
yarn test:types                        # typecheck the tests
yarn backend:test                      # Rust workspace tests
yarn build
yarn backend:check
yarn desktop:build
```

All JavaScript/TypeScript tests live in one place; see
[apps/desktop/tests/README.md](../apps/desktop/tests/README.md) for the suites, the shared harness, and the
rule that a new test joins an existing suite rather than the file it covers.
Behaviour that cannot be automated — a real school sign-in, a provider consent
screen, a signed bundle, a physical card — is recorded in
[docs/verification/manual-acceptance.md](verification/manual-acceptance.md);
run `yarn verify:manual stale` to see which checks a change invalidated.

If port 4173 is occupied, Vite selects another port. The native development
configuration expects port 4173, so stop the old process before restarting the
desktop app.

### The development app

`yarn desktop:dev` and a regular `yarn desktop:build --debug` build a separate app, `app.isctcompanion.dev`
("IsctCompanion Dev"). It has its own data folder, single-instance lock,
credential-vault service, course folder (`~/Documents/IsctCompanion Dev`), MCP
port (47833) and Windows print queue (port 24762), so it runs beside the
installed release app and never reads, migrates or deletes the release app's
credentials. It starts with an empty workspace: sign in to the school once
(import an existing OTP seed in the sign-in sheet), and copy the release data
folder into the development one by hand if you need its courses, deleting
`course-files-root.json` from the copy. Turn on Google Drive only with a test
Google account: the development app uses the same Drive app folder.

On macOS both `yarn desktop:dev` and the standalone development bundle from
`yarn desktop:build --debug --bundles app` sign with the same Apple Development
identity (a Personal Team works), which keeps their Keychain access stable
across rebuilds. Both commands use the only valid Apple Development identity
automatically. With multiple identities, select one in your shell profile:

```sh
security find-identity -v -p codesigning   # pick an "Apple Development" SHA-1
export ISCT_MACOS_DEV_SIGNING_IDENTITY=<SHA-1>
```

If no Apple Development identity is installed, create one in Xcode under
Settings → Accounts, or use `yarn desktop:build --debug --no-bundle` to compile
without creating an app bundle.

Use the same target, debug flag and signing environment when packaging an
existing binary with `yarn tauri bundle`. For a debug build, also reuse the
`ISCT_DEV_VERSION` printed by the first build so the bundle version matches
the version compiled into the app. For a debug build with the release
identity, see [macOS signing](desktop-release.md#apple-developer-signing-and-notarization).

A debug binary started any other way (such as `cargo run`) would use the
release app's data and credentials, so it refuses to start.

## Repository map

```text
apps/desktop/
  src/                     React application and disposable view caches
    app/                   application composition, routes and lifecycle
    pages/                 desktop screens
    features/              feature UI and document viewers
    platform/              Tauri client, query hooks and platform presentation
    lib/                   desktop presentation helpers and preview fixtures
    generated/             contracts generated from Rust
  src-tauri/               Tauri host, native ports, commands and HTTP MCP
  tests/                   every frontend test: harness, unit, e2e and live
  public/                  desktop web assets
crates/app-core/           authoritative domain state, services and storage
crates/adpt-isct/          stateless school parsers, page scripts and URLs
crates/app-ocr/            independent optional OCR engine and model
crates/app-ocr-adapter/    shared Core vision adapter and session replay tools
crates/app-ffi/            native Core facade and generated contract bridge
apps/ios/                  SwiftUI app over the native SDK
apps/android/              Compose app over the native SDK
bindings/                  Swift package and Kotlin/AAR packaging
resources/locales/         shared UI translation resources
resources/design/          original icon artwork
design/                    design tokens generated for every shell
config/                    cross-platform build configuration (OAuth clients)
scripts/                   desktop, bindings, checks, data and shared build tools
docs/                      runbooks, user pages, legal texts, LMS API and verification evidence
```

The SwiftUI and Compose applications live in `apps/ios/` and `apps/android/`.
Swift/Kotlin SDK packages live under `bindings/`, backed by the shared Rust
facade in `crates/app-ffi/`.

Platform manifests and build configuration stay beside the owning app. The
root Yarn/Cargo workspaces retain one lockfile each and the Rust build output
remains at root `target/`. Root commands delegate to the desktop workspace.
Generate desktop TypeScript contracts with `yarn core:types`; the exporter
belongs to Core and does not require compiling Tauri. Subscription test data
remains in `crates/app-core/tests/fixtures/subscriptions/`.

Desktop releases build independently; platform checks continue to compile
shared Core for iOS and Android.

## Desktop Core checkpoint

Desktop consolidation through Phase 5 is complete. The frontend renders Core
read models, caches per-view query answers (TanStack Query) and forwards typed
intents; Rust owns validation,
revisions and domain state. Academic date calculations use JST, and stored
plan timestamps use explicit offsets. The signed macOS checkpoint and its
verification coverage are recorded in
[Core consolidation checkpoints](verification/core-consolidation.md).

The Core contracts (transport mapping, records, services, ports, read models,
mutations, errors and shutdown) are decisions in slotledger; `STATUS.md` lists
them. See the [documentation index](README.md). The desktop MCP adapter
shares those services at `http://127.0.0.1:47832/mcp` while the app is open; authentication
is configured in Settings → AI agent access.

The native SDK is implemented in `crates/app-ffi`, with generated Swift/Kotlin
packages under `bindings/`. See [native SDK integration](core-ffi.md) for
service operations, cancellation, native ports, build commands and verification
boundaries.

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
- Google Drive stores the readable workspace export as
  `IsctCompanion.json` in the provider's private app folder.
- Optional school-credential sync uses a separate AES-256-GCM encrypted file.
  Its encryption password (12–128 characters for a new one) stays on the local
  device and must be entered separately on another device.
- Site customization runs only inside allowlisted school WebViews and has no
  bridge to workspace data or native secrets.

Cloud provider registration is documented in
[docs/cloud-provider-setup.md](cloud-provider-setup.md). Persistence
boundaries are documented in [docs/data-ownership.md](data-ownership.md);
school import rules are slotledger decisions (`approach/school-parser-boundary`,
`constraint/school-failure-keeps-data`).

## Scope

The app reads school data and helps organize local plans. It may fill an
assignment's LMS draft area, which the user saves; it never submits
assignments, changes enrollment, publishes official calendar data or provides a
server-side polling service. Distribution signing, notarization and public
hosting remain release configuration.

Rust CI uses two caches through the shared `rust-caches` action:
`Swatinem/rust-cache` retains external dependencies, and jobs that opt in also
use sccache for workspace libraries. The latter keys compiled output by content, so checkout timestamps do not force
unchanged OCR algorithms through code generation. Core owns Matrix sessions;
`app-ocr-adapter` connects its vision port to the independent `app-ocr` engine.
Core business code and unrelated Core feature changes therefore do not invalidate
that engine.
Compiler, target, profile, engine source/model and shared dependency changes
still select different compiled output. Final linking and build scripts still
run. Cargo may print `Compiling app-ocr` on a hit; the action's post-job sccache
statistics show whether the compiler output was reused. `cargo clippy` replaces
`RUSTC_WORKSPACE_WRAPPER` with clippy-driver, so lint steps do not use the
compiler cache; the reuse is in `cargo test` and `cargo check` steps.

Only main writes the Rust and Gradle caches. Warm caches uses the quick
jobs for the dependency inputs that changed on main. It also runs both suites
weekly and reports their results directly in Actions. The release suite uses
separate Windows and macOS keys because it lints and tests. Quick Android
checks read the same Rust dependency archive as the release suite, which
populates it for both ABIs. Release builds use neither Rust cache.

Platform checks' route job (`scripts/checks/route-platform-checks.sh`) prints
one `<job>=true|false` line per job for the changed paths and suite mode.
A successful push records its verified plan. A same-repository PR can reuse
the source checks when its merge tree matches that push's tree and its own
plan matches the verified plan. Frontend checks run against the PR's Git
history because the manual acceptance tooling uses it.
