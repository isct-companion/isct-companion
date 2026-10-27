# School browser host contract

How a shell hosts school web views for Core. Core
(`crates/app-core/src/school_browser`) owns every decision: which navigation
and window may happen, which view a command goes to, what is shown about a
page, when hidden work may use a view. A host is a browser engine adapter. It
reports what the engine knows, carries out what Core decides, and keeps no
rules of its own: no host list, no scheme exception list, no injected page
shim, no history of its own.

The same contract is exposed through `app-ffi` (`NativeBrowser`,
`BrowserEvents`) for iOS and Android, and implemented directly in Rust by the
desktop (`apps/desktop/src-tauri/src/school_browser_native.rs` with
`school_webkit.rs` on macOS and the interim `school_windows.rs` on Windows).

## Views, popups and pages

- A **view** is one resident web view per `SchoolSite`, created by
  `ensure(browser, initialUrl)`. The person's browsing and Core's hidden work
  (session checks, sign-ins, pulls) share it, so the school session is one.
- A **popup** is a window a page opened. Core gives it a `BrowserId` of its
  own (the opener's site, a new generation) and registers it as a child of its
  opener, which may be the view or another popup. A popup lives and closes
  with its opener.
- `BrowserId { site, generation }` names a view or popup generation.
  `PageId { browser, sequence }` names a page in it: the host increments
  `sequence` for every page it reports loaded or failed, and passes the last
  one with requests. Core drops reports of an older page or generation.
- Every report goes to `BrowserEvents`, and callbacks return promptly. A host
  never waits for its main thread from a Core worker, and Core never waits for
  a host callback.

## What a host reports

`BrowserEvents.navigation(NavigationRequest) -> NavigationDecision`, for every
frame navigation before it starts, main frame and subframes:

| Field         | Meaning                                                                                           |
| ------------- | ------------------------------------------------------------------------------------------------- |
| `page`        | The view or popup asking, with the last page it reported                                          |
| `url`         | The request URL, as the engine has it (`about:blank`, `blob:` and `data:` included)               |
| `method`      | `GET`, `POST`, …; `null` when the engine does not say                                             |
| `mainFrame`   | Whether the main frame navigates                                                                  |
| `kind`        | `Link`, `FormSubmit`, `BackForward`, `Reload`, `FormResubmit { fromHistory }`, `Other`, `Unknown` |
| `userGesture` | Whether the person caused it, when the engine says                                                |

`FormResubmit` means the engine is about to send an earlier form again:
WebKit's `.formResubmitted`, Android's `onFormResubmission`. `fromHistory` is
true when it travels back or forward to a POST result, false when it reloads
one. WebKit uses the same type for both, so a WebKit host compares the back/
forward list's current item at decision time (WebKit has already moved it to
the target) with the item of the last committed page.

`BrowserEvents.newWindow(NewWindowRequest) -> NewWindowDecision`, for
`window.open`, `target=_blank` and forms posted to a named window: the opener
page, the first URL (`about:blank` for `window.open('')`), method, kind,
gesture, the window name when the engine exposes it, and the requested
`width`/`height`.

`loadFailed(page, cause, url, reason, detail)`, for a main-frame load that did
not finish, with a coarse `LoadFailure`: `Cancelled` (a newer load replaced
it, a policy decision cancelled it, it became a download), `Offline`,
`HostNotFound`, `Unreachable` (timeout, refused, connection lost), `Insecure`
(certificate or TLS), `Cleartext` (the engine refused plain HTTP), `Other`.
Report every failure, cancellations included; Core decides what is shown.
`detail` is the engine's text, for logs.

`pageLoaded`, `titleChanged`, `stopped` as before. `closed(browser)` when a
view or popup is gone: its page called `window.close()` (WebKit
`webViewDidClose`, WebView2 `WindowCloseRequested`, Android `onCloseWindow`),
the person closed it, `dispose` was carried out, or it could not be created.

`capabilities() -> HostCapabilities`, read once when the engine starts:

- `formResubmission`: the host reports `FormResubmit`. Without it Core sends a
  GET of the current URL for the app's Reload instead of the engine's reload.
- `inlinePopups`: popups appear inside the web pane over their opener, so the
  app's toolbar (address, Back, Forward, Reload, Close window) acts on the
  frontmost popup. Without it popups are separate windows with their own
  controls and the toolbar stays on the view.

## What Core decides

`NavigationDecision`:

- `Allow`.
- `External { url }`: cancel, and open `url` in the system browser.
- `Deny { reason }`: cancel. Every denial is logged with scheme and host only.
- `SkipEntry`: cancel this back/forward resubmission, then, once the engine
  has put its list back, step one entry past the target in the same direction
  (WebKit: `go(to: list.item(at: ±2))` relative to the origin), so the old
  form is never sent and history stays whole.

`NewWindowDecision`:

- `Popup { browser }`: build the window from the engine's own request and
  configuration (WebKit: return a web view made with the configuration
  `createWebViewWith` passed; Android: hand a real WebView to the
  `WebViewTransport`; WebView2: `NewWindow` in the opener's environment), so a
  posted form and `window.opener` survive. Report everything in it under
  `browser`.
- `External { url }`, `Deny { reason }` as above. WebKit's denial gives the
  page `null`; WebView2 gives it a dummy window.

### The rule table

Keyed by the view's role: **User** while the person is looking at the site
(`schoolBrowsing`), **Automation** for a hidden view. A popup has its
opener's role.

| Request                                                              | User                                                                | Automation                                                            |
| -------------------------------------------------------------------- | ------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `about:blank`, `about:srcdoc`, `blob:`; `data:` below the main frame | Allow                                                               | Allow                                                                 |
| `data:` in the main frame, `file:`, `javascript:`, other schemes     | Deny `scheme`                                                       | Deny `scheme`                                                         |
| User name or password in the URL                                     | Deny `credentials`                                                  | Deny `credentials`                                                    |
| HTTPS                                                                | Allow                                                               | Allow on adapter sites and identity providers, else Deny `automation` |
| Plain HTTP, main frame, by a link, form, gesture or `Unknown`        | External                                                            | Deny `automation`                                                     |
| Plain HTTP otherwise                                                 | Deny `cleartext`, shown as a notice                                 | Deny `automation`                                                     |
| `FormResubmit`, reload, main frame                                   | Deny `resubmission`; Core loads the URL with GET and shows a notice | the same, no notice                                                   |
| `FormResubmit`, back/forward, main frame                             | SkipEntry                                                           | SkipEntry                                                             |
| `FormResubmit`, subframe                                             | Deny `resubmission`                                                 | Deny `resubmission`                                                   |
| New window, HTTPS or local document                                  | Popup (at most 8 per view)                                          | Deny `hidden`                                                         |
| New window, plain HTTP                                               | as navigation                                                       | Deny `hidden`                                                         |

A person's new POST (a button, a form, `__doPostBack`) is always allowed:
教務Web pages, paging included, are postbacks. Only the engine resending an
old form is stopped. Nothing relaxes App Transport Security or Android's
cleartext rule; plain HTTP files open in the system browser.

The adapter sites are `SchoolSite::for_url`; the identity providers are
`SchoolAuthProfile::for_auth_host` (`automation_target`). There is no other
host list. What may be written to disk as a restore record, or compared as a
page identity, is `restore_record`: an automation target without fragment and
with only bounded identifying query parameters.

Files: a view the person is looking at may save any HTTP(S), `blob:` or
`data:` file; a hidden view only files from adapter sites
(`download_permitted`).

### Commands

`dispatch(BrowserCommand)` with `Navigate { url }`, `Reload`, `GoBack`,
`GoForward`, `Evaluate { script }` and `Stop { script }`, for a view or a
popup. Back and Forward are the engine's own history; Core keeps no history.
`Reload` is sent only to a host with `formResubmission`. `Stop` goes only to
views Core's hidden work used. `dispose(browser)` closes a view or popup; Core
disposes a view's popups before the view.

## What the app shows

Core emits, for the shell's UI:

- `school-web-url` `{ site, url }`: the address of the frontmost page of the
  site the person is looking at (a popup's when one is in front).
- `school-web-popups` `{ site, open }`: how many popups are open over the
  view (hosts with `inlinePopups`). The app shows "Close window"
  (`closePopup` on desktop) while one is open.
- `school-web-notice` `{ site, kind: "load-failed", url, reason }`: the page
  the person is looking at did not load (not for cancellations or hidden
  views). The app shows it next to the address with Retry and Open in browser.
- `school-web-notice` `{ site, kind: "reopened", url }`: a reload would have
  resent a form; the page was opened again with GET.

Nothing is injected into a school page to show these.

## Page dialogs and printing

`alert`, `confirm` and `prompt` are shown natively for the view or popup the
person is looking at, titled with the initiating frame's origin, and answered
Cancel for a hidden view without any UI. `window.print()` opens the system
print panel for the foreground view only (WebKit: the private
`_webView:printFrame:pdfFirstPageSize:completionHandler:`; Android:
`PrintManager`). School pages get no app script: Tauri injects every plugin's
page script into every webview, so a desktop plugin must not have one.

## Platform status

| Host                       | Facts                                                             | Popups                                             | Print, dialogs                       |
| -------------------------- | ----------------------------------------------------------------- | -------------------------------------------------- | ------------------------------------ |
| macOS (`school_webkit.rs`) | Full, `formResubmission`                                          | Inline over the opener                             | Done                                 |
| Windows (interim)          | URL only (`Unknown`, no method or gesture); no resubmission facts | Separate windows (`NewWindowResponse::Create`)     | WebView2's own                       |
| iOS (`SchoolBrowserHost`)  | Full but gesture, `formResubmission`                              | Inline over the opener                             | Done                                 |
| Android (interim)          | URL, frame and gesture (`Unknown` kind)                           | Load in the opener's view; reported closed at once | WebView's own dialogs; print pending |

The Android host implements this contract in full in its own change: real
popups (`WebViewTransport`), the full `NavigationRequest` (Android
`onFormResubmission` and an app notice for `ERR_CACHE_MISS`), `SkipEntry`,
`closed` on `window.close()`, dialogs, printing, `LoadFailure` mapping, and
`capabilities` set to true. The Windows WebView2 host (`NavigationStarting`,
`NewWindowRequested` with a deferral and the same environment, `ScriptDialogOpening`,
its own record of POST entries) replaces the interim later.
