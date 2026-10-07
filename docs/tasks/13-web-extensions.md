# Task 13 — Web toolbar: forward navigation and CSS/JS extensions

Depends on task 10. Follow the rules in [README](README.md); the login-automation restriction applies in full.

The school web view (`src/pages/web/WebPage.tsx`, `backend/src/school_web.rs`) already has back (postback-safe `PageHistory`), reload, zoom and an address bar. Customization today is one CSS + one JS string per site (`customization.sites[site]`) gated by `settings.siteConsents[site]`, edited in Settings → Site styles & scripts (a Developer options page).

Extensions are **for styling and small page tweaks only**. They do not get any API to app data; no hook bridge is exposed to page scripts.

## Build

### Forward

- Add a forward button next to back. Extend `PageHistory` with a forward stack: back moves the current entry to it, a new navigation clears it. Forward must skip WebForms postback entries the same way back does (`history_entry`). New command `school_web_forward`; expose `canGoBack` / `canGoForward` to the front end so the buttons disable correctly.

### Extensions model

- Replace per-site customization with a list stored in settings:
  `Extension = { id, name, matches: string[], css: string, js: string, enabled: boolean, runAt: "document-start" | "document-end" }`.
  `matches` are URL globs (e.g. `https://lms.s.isct.ac.jp/course/*`) whose host must be one of the official school hosts in `siteForAddress` (`WebPage.tsx`); reject anything else. Keep the 256 KiB size limit per field.
- Migration: each existing non-empty `customization.sites[site]` becomes one extension matching that site's host, `enabled` = the old `siteConsents[site]`. Remove `siteConsents` and `customization.sites` after migration (schema version bump, round-trip through export/import).
- Injection: on every page load, inject all enabled extensions whose `matches` fit the URL, in list order. CSS as a `<style data-isct-ext="{id}">`; JS wrapped in an IIFE with try/catch that logs errors to the app's activity log with the extension name. Extractors and extensions stay independent, as today. Toggling an extension re-applies on the current page (remove its style element; JS changes apply on reload).

### Toolbar menu

- Add an extensions button to the web toolbar: a popover listing the extensions that match the current URL, each with an on/off switch, plus `Manage…` which opens Settings → Site styles & scripts.

### Editor (functional; styling is done separately)

- Settings → Site styles & scripts: list with add / duplicate / delete / reorder / enable, and an editor for name, matches, run-at, CSS and JS.
- Use the existing CodeMirror 6 setup; add `@codemirror/lang-css` and `@codemirror/lang-javascript` for highlighting and basic autocomplete. No TypeScript type checking.
- Import / export a single extension as JSON.

## Out of scope

Any data API or message channel from page scripts to the app. Remote extension sources or auto-update.

## Acceptance

- Back → forward returns to the same page; a new navigation after back disables forward; postback pages are skipped in both directions.
- An old workspace with portal CSS enabled migrates into one enabled extension and the portal still looks the same.
- Disabling an extension from the toolbar removes its CSS from the current page immediately.
- A `matches` pattern for a non-school host is rejected.
- Rust tests for the forward stack and pattern matching; front-end test for migration.
