# Core school browser

The school browser is one logical service in `isct-core`. Core owns login,
session, extraction, history, popup, restore and scheduling decisions. A native
shell owns the actual WebViews, their presentation and platform callbacks.

This split is used by desktop now and is the boundary intended for later iOS and
Android shells. It does not make WebKit or Android WebView failures recoverable
inside the Rust process.

## Ownership

`Core::attach_school_browser(browser)` installs one `SchoolBrowserEngine`.
The browser uses the event sink supplied once to `Core::with_events`; a headless
`Core::new` uses a no-op sink. `Core::school_browser()` returns that same engine;
shells must not create a second login queue or session state machine.

Core owns:

- the resident logical view for each `SchoolSite`;
- official-origin and query validation for automation and persisted restore URLs;
- login attempts, saved-credential renewal and session state;
- the hidden extraction lane, priority, preemption, cancellation and apply
  boundary;
- safe Back and Forward history, same-site popup return and restore records;
- login, session, progress, ingest, URL and cross-site-open events;
- the embedded login, link, popup and Android support scripts.

The shell owns:

- creating, showing, hiding, sizing and closing native WebViews;
- executing a dispatched navigation, reload or script on the required UI
  thread;
- forwarding page, title, navigation, popup, failure and close callbacks;
- native downloads, file reveal and external-browser opening;
- credential storage through `SchoolAuthService`'s vault port.

Window visibility is a shell fact. `user_browsing(Some(site))` yields hidden
school work to the visible page; `user_browsing(None)` lets queued work resume.
Every hidden step (session check, sign-in, pull navigation and extraction)
re-checks visibility after it is admitted to the lane and refuses to start while
a page is shown, so work that waited for the lane never navigates a page the
person has just opened.
The shell must keep this fact aligned with native presentation.

## Interactive browsing versus automation

The official-origin allowlist is an automation boundary, not a browsing sandbox.
User-initiated open/navigation preserves HTTP(S) query parameters and fragments;
external search and linked forms stay in the visible resident workspace. Official
service links can still select their own service view. Back/Forward retains the
full interactive address. External URLs are not written to the school restore
record, and official restore URLs retain their existing sanitization.

Empty `about:blank` / `about:srcdoc` frames are admitted so rich editors such as
TinyMCE can create their inherited-origin editing document. Desktop's Tauri
navigation callback does not expose a frame flag; the portable listener's flag
cannot reliably distinguish these on desktop, so empty frame URLs are admitted
without relying on that flag. These pages receive no school automation scripts.

While a view is visible, ordinary external web navigation is allowed. Hidden
extraction and login keep their strict official-origin validation. An external
page load only updates interactive location/history: it receives no credential,
extraction, submission or school customization injection. Remote web content has
no application IPC grant; the Tauri capability remains local-shell-only.

Native IME, focus, paste and live TinyMCE behavior are covered by the existing
`SCHOOL-ASSIGNMENT-UPLOAD` manual check. Automated engine tests cover navigation
admission, external-page script isolation, complete URLs, history and the switch
back to hidden restrictions. Shell tests cover address submission and the reader
showing the actual destination hostname rather than an incorrect official label.

## Port and dispatch contract

The shell implements `SchoolBrowser` and receives `BrowserCommand` values.
Returning `Ok` from `ensure` or `dispatch` means the command was accepted for
native dispatch. It does not mean a page loaded, a script ran successfully or a
login completed.

Completion and failure return through `SchoolBrowserListener`:

- `page_loaded(PageLoaded)` and `load_failed(LoadFailed)` advance or fail work;
- `title_changed` carries authenticated login and progress relays;
- `decide_navigation` returns `Allow` or `Cancel` for every request;
- `new_window` returns `SameView`, `OpenSchool`, `OpenExternal` or `Ignore`;
- `closed` invalidates that logical browser generation.

A shell reports `closed` when it disposes a view, cannot create one, or receives
a command for a view that no longer exists; returning `Unavailable` from
`dispatch` has the same effect. Core then forgets that generation, so the next
request creates a new view. Work waiting in a closed view fails with a platform
error naming the closed view; only engine shutdown reports `Stopped`.

Callbacks carry a `BrowserId { site, generation }`. Core rejects callbacks from
an older resident view. A `PageId` adds the native view's monotonic load
sequence, and Core rejects page loads that do not advance it. `ActionId`
identifies dispatched commands and dispatch failures. Current native WebView
APIs do not expose a trustworthy request ID for successful page-load callbacks,
so a successful `PageLoaded` can have no `cause`. Core also checks the expected
site or extraction URL and uses view-scoped relay prefixes plus per-run nonces.
Adapters must not label a late load with the newest action merely to fill this
field.

Relay navigation is a complete decision path: malformed, oversized, stale or
wrong-run payloads are cancelled and ignored. An accepted extraction relay is
consumed once. Login title relays are bounded, validated against the active
profile and run, and cleaned before they become UI messages.

## Login and session state

`SchoolSessionState` is `pending`, `active`, `failed` or `not-configured`.
`session_snapshot()` returns both profiles with one revision. Every accepted
state change emits `SchoolSessionEvent { profile, state, revision }` and advances
that revision. Revisions are scoped to one Core instance; a shell that replaces
its bridge must accept the new instance's revision epoch.

The frontend subscribes before reading the snapshot. It applies revisions per
profile, so an event received during the snapshot read wins for that profile
without preventing the snapshot from initializing the other one. `pending` has
no status badge; the existing UI continues to display active, failed and
not-configured.

The shell reports `{ kind: launch | focus, visible }`. Core owns launch
coalescing, the 90-second session deadline and the two-minute focus gate. A
visible page preempts a background session check; the owned check resumes after
the page hides. A manual sign-in paused by a visible page resumes the same way
rather than failing. Every exit of a sign-in attempt clears its run, so a
finished or abandoned attempt never blocks automatic sign-in for its profile. Credential save and deletion call `credentials_changed`, rather
than letting the frontend infer state.

A session timeout settles to `failed` and releases the lane for a later retry.
Automatic credential reads are accepted only while their original page and
login owner remain current. The owner is the session check, the visible page, or
a hidden pull whose navigation landed on the identity provider. A hidden pull
signs in with saved credentials and, once sign-in succeeds, Core loads the
pull's target again. When the profile has no saved credentials, its automatic
sign-in is blocked after a failure, or the attempt fails, the pull returns
`reconnect-required` at once instead of waiting for the page timeout. Page identity comparisons use the same sanitized
URL representation on both sides, including identity-provider query URLs.

`SchoolLoginEvent` remains separate from session state. It reports automation
steps, user-required steps, typed failures and the final connected result used
by the Accounts UI.

## Extraction lifecycle

School pulls enter the shared `PullCoordinator`, then the browser's single
hidden lane. Manual work has priority over waiting automatic work. A caller that
joins a live job of the same key can promote it to manual and ask it to sign in
first; the engine reads both facts again at every lane admission, and signs in
under the same lane admission as the extraction that follows. Opening a
visible school page invalidates an armed extraction relay; hidden work that
observes the visibility change before its apply boundary is requeued after the
page hides. Cancellation is checked while waiting for visibility, lane
admission and native callbacks.

An extraction result is accepted only for its browser generation, page, target
and run nonce. Core then asks `PullContext` to enter `applying`. Cancellation
before that transition discards the payload. Once the transition succeeds,
parsing and the database transaction finish as owned work; dropping the
original native caller does not abort the commit. 教務Web commits each page
under its own gate and returns to `running` afterwards, so Cancel stops the pull
between pages; a shutdown during a page commit cancels the remaining pages once
that commit finishes. An accepted payload must identify a supported page shape,
set `complete: true`, and include every collection required by that shape. An
empty collection is authoritative only when the page contains the extractor's
scoped no-data marker. A missing table paired with a visible group heading,
unknown headers and partial rows are `error`; they do not enter the apply
transaction or mark older rows missing.
教務Web does not publish a manifest of the grade groups or requirement tables
an account should receive, so Core does not hard-code undergraduate counts for
graduate accounts. After a fully loaded page, the extractor validates every
candidate table that is present. If the authority removes a whole candidate
from the DOM, that is indistinguishable from an intentional account-specific
shape until the site publishes another completion signal.

Outcomes are `ok`, `unavailable`, `reconnect-required` or `error`.
`unavailable` requires an explicit not-offered message on an approved school
page. Core records that capability observation for refresh freshness and UI
display, but leaves the page's existing rows untouched. It is a completed pull
outcome rather than a coordinator failure. `pageAvailability` in the school
read model distinguishes retained older rows from the latest authoritative
capability observation.

Visible LMS and 教務 pages use the same extractor and apply functions. This
keeps page-triggered refreshes and background pulls on one parser and storage
path.

## Navigation, history and restore

Every explicit open or navigation intent must sanitize to an official origin
and belong to its requested `SchoolSite`. Native page navigation accepts the
global official-origin allowlist because login flows cross service and identity
provider origins. External popup URLs must be plain HTTP(S) without embedded
credentials. Same-service popups load in place; Core records their opener and
overrides `window.close` so returning does not leave a stale popup entry in Back
history.

Back and Forward load a sanitized history URL as a fresh GET. They do not call
native browser history, because replaying a 教務 WebForms POST can repeat an
action or trigger its invalid-transition guard.

Restore records contain one sanitized site and URL and are written atomically.
An epoch and serialized I/O boundary prevent a late page callback from
recreating a record after restore was disabled. A record for another site or an
invalid URL falls back to that site's start page.

Reopening an existing view without an explicit target preserves its native
page, including transient SSO query parameters. Sanitized history and restore
records do not trigger a replacement navigation when the view is revealed.

## Errors and shutdown

The public boundary returns `SchoolBrowserError`: `InvalidInput`, `Platform`,
`Storage`, `Auth`, `SiteChanged`, `Timeout`, `Cancelled`, `Preempted` or
`Stopped`. Native `BrowserError` values map to the platform or stopped cases;
shell commands may preserve their existing display strings when mapping these
typed errors to Tauri.

`SchoolBrowserEngine::shutdown` closes admission, wakes owned waits, drains its
runtime tasks, shuts down the native browser port and drops the listener.
Native adapters must stop accepting callbacks and operations as part of the
same application shutdown sequence.

This document describes the code boundary and invariants. It is not a record of
desktop or physical-device acceptance; those results belong in the verification
documents after the relevant run completes.
