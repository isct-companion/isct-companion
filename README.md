# IsctCompanion

A local-first academic workspace with a responsive frontend shared by a Tauri native app and an installable read-only phone PWA. The included sample workspace is visibly separate from school-imported records and is useful before accounts are configured.

## Run locally

```sh
yarn install
yarn dev
```

Open <http://localhost:4173>. Use `yarn build` for the shared frontend checks. The default macOS native commands are `yarn desktop:dev` and `yarn desktop:build`; they use Tauri and the system webview.

The capability handshake keeps unfinished native commands unavailable. Native acceptance is based on the Tauri `.app` and its acknowledged capabilities.

The current native target is macOS. Run `yarn tauri:dev` for development and `yarn tauri:build` to create the local `.app`. Distribution signing and notarization remain release-stage configuration.

If port 4173 is already occupied, Vite prints the next available URL. The original static prototype is preserved at `reference-demo/index.html`.

## Project structure

Screen code lives in `src/pages/{today,courses,web,settings}`, shared UI in `src/components`, and app navigation in `src/app`. The Rust/Tauri native backend is in `backend/`. See [the project structure guide](docs/project-structure.md) for file responsibilities and commands.

## Layout

Every page fits the window without scrolling: a slim left rail (Today / Courses / Web / Settings, `⌘1`–`⌘4`), and each page owns its height.

- **Today**: the week timetable with a live Japan-time marker, plus "Next" and open assignments.
- **Courses**: quarter tabs (add a quarter with `＋`), a search box that filters the list and dims non-matching blocks, on-timetable switches, and full-screen editors for courses and personal events.
- **Web**: Portal / T2SCHOLA / Syllabus embedded directly in the window (Tauri child webview; the browser preview uses an iframe where the site allows it).
- **Settings**: General, Accounts & sync, Notifications, Academic calendar, Data, and Customize (app CSS, per-site CSS/JS).

The browser preview is read-only by policy (it is the phone PWA build). To exercise editing in the browser during development run this once in the devtools console, then reload:

```js
localStorage.setItem("isct-companion-dev-platform", "native-desktop")
```

## Working behavior

- Quarter-isolated academic records, Japan-time deadlines and calendar exceptions, computed conflicts, responsive workflows, course drawers, and JSON export.
- Versioned IndexedDB persistence with a localStorage migration/fallback.
- Installable offline PWA. Academic and cloud writes are denied by the shared action guard on PWA.
- Tauri commands with trusted-window checks, allowlisted external navigation, local notifications, and a persistent official-site session.
- Per-site desktop CSS and opt-in JavaScript run only in an isolated, domain-allowlisted school window. Built-in school refresh and recoverable export schedules run while the native app is open.
- Cache-first T2SCHOLA course/detail crawl with tolerant extraction, login-page expiry detection, staged validation, and atomic cache publication.
- Google Drive `appDataFolder` and OneDrive app-root transports using one readable `IsctCompanion.json` file with optimistic revision checks. Invalid reads are rejected while local data remains intact.
- Public-client OAuth/PKCE components, explicit cloud conflicts, and a single plaintext JSON workspace stored in the provider's private app folder.

## Configuration and current limits

No credentials, OAuth client IDs, signing identities, or provider grants are included. School sign-in happens only in the official site window; this app does not collect passwords, OTP seeds, or cookies. Real school selectors still need verification against an authorized account and sanitized fixtures. A failed, empty, or changed school page cannot replace the last valid cache.

Cloud use requires a provider public client, the configured loopback redirect URI, and user consent; see [cloud provider registration](docs/cloud-provider-setup.md). Google and Microsoft OAuth details differ, so the UI does not claim a connection until a complete provider setup and transfer has succeeded. The cloud workspace is plaintext JSON and relies on the provider account for access security. Remote push is unavailable in this no-backend build. PWA refresh is launch/resume/manual and may be suspended by the phone OS.

Android and iOS packaging are parked. Their central action policy remains in shared code so the PWA stays read-only and future native mobile builds cannot inherit desktop-only advanced controls.

Assignment submission, enrollment changes, official calendar publication, background server polling, automatic updates, code signing, and public hosting are outside this implementation. Sample assignments and materials remain sample records until a successful school import provides validated source records.

## Verify

```sh
yarn test
yarn build
```
