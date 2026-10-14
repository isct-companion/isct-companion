# Core school browser

The school browser is one logical service in `isct-core`. Core owns login,
session, extraction, history, popup, restore and scheduling decisions. A native
shell owns the actual WebViews, their presentation and platform callbacks.

This split is used by desktop now and is the boundary intended for later iOS and
Android shells. It does not make WebKit or Android WebView failures recoverable
inside the Rust process.

## Ownership

`Core::attach_school_browser(events, browser)` installs one
`SchoolBrowserEngine`. `Core::school_browser()` returns that same engine; shells
must not create a second login queue or session state machine.

Core owns:

- the resident logical view for each `SchoolSite`;
- the school URL allowlist and query sanitization;
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
the page hides. Credential save and deletion call `credentials_changed`, rather
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
that commit finishes. Outcomes remain `ok`,
`reconnect-required` or `error`, and Core emits progress and ingest events with
the existing wire fields.

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
