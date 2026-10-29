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

`historyChanged(browser, canGoBack, canGoForward)`, whether the engine's own
history of a view or popup can step back or forward from its page. Report it
when a page commits and when it finishes (WebKit `didCommit`/`didFinish`,
Android `doUpdateVisitedHistory`), and right after a popup is created. Until a
host reports it, Core treats a view's or popup's history as unknown.

`pageLoaded`, `titleChanged`, `stopped` as before. `closed(browser)` when a
view or popup is gone: its page called `window.close()` (WebKit
`webViewDidClose`, WebView2 `WindowCloseRequested`, Android `onCloseWindow`),
the person closed it, `dispose` was carried out, or it could not be created.

`becameDownload(page, hadPage, disposition)`, when a navigation of a view or
popup becomes a download (WebKit `navigationResponse:didBecomeDownload:` and
`navigationAction:didBecomeDownload:`; Android `DownloadListener`; WebView2
`DownloadStarting`). `hadPage` says whether that view had committed a page
before; a popup opened for the file has not. `disposition` is `Shown` for a
file the host fetches and then displays (a PDF sent as an attachment) and
`Saved` for one written to the device. A shown file is displayed in the view
whose navigation it was, never in the engine's "originating" view: WebKit's
`WKDownload.webView` is the opener for a popup that has not committed a page.
A host keeps a closed popup's download running until the file is written.

`capabilities() -> HostCapabilities`, read once when the engine starts:

- `formResubmission`: the host reports `FormResubmit`. Without it Core sends a
  GET of the current URL for the app's Reload instead of the engine's reload.
- `inlinePopups`: popups appear inside the web pane over their opener, so the
  app's toolbar (address, Back, Forward, Reload, Close window) acts on the
  frontmost popup, on every surface that shows the view. Without it popups are
  separate windows with their own controls and the toolbar stays on the view.

An inline popup is a sibling of its site's view, never a subview of a web
view: the host adds it to the view's superview (container, layout) above the
view and the site's earlier popups, in opening order, with the view's frame,
and moves, resizes, shows and hides it with the view. WebKit answers a drag
at the outermost web view under the pointer: on macOS WKWebView overrides
AppKit's drag hit test (`_hitTest:dragTypes:`) to return itself for any
point in its frame without descending into subviews, so a popup inside it
would never receive a file dragged onto it, and the page underneath would.
Clicks would still work, since the ordinary `hitTest:` descends, which is why
this is easy to miss. macOS keeps popups in the main view's superview
(`school_webkit.rs` `arrange_popups`), iOS in the container holding the view
(`arrangePopups`), Android in the host's `FrameLayout`.

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
(`download_permitted`). A popup with no committed page whose load became a
`Saved` download has nothing to show, so Core closes it (`dispose`), as a
browser closes an empty download tab; a `Shown` file, or a popup that already
had a page, stays.

### Commands

`dispatch(BrowserCommand)` with `Navigate { url }`, `Reload`, `GoBack`,
`GoForward`, `Evaluate { script }` and `Stop { script }`, for a view or a
popup. Back and Forward are the engine's own history; Core keeps only what the
host reported about it (`historyChanged`), never entries of its own. `Reload`
is sent only to a host with `formResubmission`. `Stop` goes only to views
Core's hidden work used. `dispose(browser)` closes a view or popup; Core
disposes a view's popups before the view.

The app's commands act on the page in front: the last popup open over the
view (hosts with `inlinePopups`), the view itself otherwise.

| App command        | What Core does                                                                                             |
| ------------------ | ---------------------------------------------------------------------------------------------------------- |
| Address bar, Retry | `Navigate` the page in front, as a browser loads a typed address in the window it was typed in             |
| Back               | A popup at the start of a known history closes; history known to have no step: nothing; otherwise `GoBack` |
| Forward            | History known to have no step: nothing; otherwise `GoForward`                                              |
| Reload             | The engine's reload (with `formResubmission`), or a GET of the current URL                                 |
| Close window       | Closes the popup in front and every popup it opened                                                        |
| `open(site, …)`    | See below                                                                                                  |

Closing a popup at the start of its history is what Safari and Chrome do for
a tab a page opened. "Unknown" is never taken as "no": the command goes to the
engine, which does nothing if it cannot step.

## Surfaces and entries

A surface is an app page that shows a site's view: on the desktop the Web page
(`web`) and every mounted LMS reader in Lectures (`reader-1`, `reader-2`, …);
on iOS and Android their one web screen (`web`, fixed by `app-ffi`). A surface
brings the view on screen with `open(site, surface, entry)`, and Core records
it as the site's **presenter**:

- `Resume`: the view as it is, popups and a form in progress included; the
  restored or start page when the view has none.
- `Go(url)`: an app link. Core closes every popup over the view, then
  navigates the view itself only if it is not at `url` already.
- `Return(url)`: a surface showing the view again (a confirmation covered the
  LMS reader; the person looked at Today and came back). If it is still the
  presenter, this is `Resume`, even if the person has since moved the view
  elsewhere; if another surface showed the view in between, it is `Go(url)`.
  Core does not compare URLs.

The desktop Web page resumes when switched to and goes for links from Today,
a course panel or Settings, and for choosing the site already shown (back to
its start page). A reader goes to its activity when it mounts and returns to
it when shown again. On iOS and Android `openSchool(site, target)` is `Go`
with a target and `Resume` without. A closed view forgets its presenter.

Popups stay when a surface leaves: they are part of the view, so a 教務Web
dialog filled halfway is still there when the person comes back.

## What the app shows

Every surface renders one read model, the page the person sees of a site:

```text
SchoolWebPage {
  site, revision,          // revision grows with every change, across sites
  url: string | null,      // the page in front; null until the view has one
  popups: number,          // popups open over the view (inlinePopups hosts)
  back:    "yes" | "no" | "unknown",
  forward: "yes" | "no" | "unknown",
  notice:  { url: string | null, reason: LoadFailure } | null,
}
```

`back` is `yes` when the host reported a step back, or when the page in front
is a popup with known history (Back closes it); `no` when the host reported
none (the app disables the button and Core sends nothing); `unknown` when the
host never reported history. `forward` is the same without the popup rule.

`notice` is a failed load of the page in front, bound to that view or popup
and to the page sequence of the failure. It is set for a main-frame load the
person sees fail (not cancellations, not hidden views) and for plain HTTP the
rule table refused, and replaced by a later failure. It is cleared when that
view or popup reports a later page, when Core sends it a navigation
(`Navigate`, `Reload`, `GoBack`, `GoForward`: Retry, the address bar, an app
link), and when it closes. A popup's notice shows while the popup is in front;
the view's own shows again once the popup closes. The app shows it next to the
address with Retry (not for plain HTTP) and Open in browser. Because it lives
in the page, a failure is still shown after the person switches surfaces.

Core publishes the page as `school-web-page` (the page itself as payload)
whenever it changes, for the site the person is looking at; when a site comes
on screen, its page is published if it changed while off screen. The snapshot
is `page(site)` (desktop command `school_web_page`, FFI `schoolWebPage`). A
surface subscribes first, waits until the subscription is in place, then reads
the snapshot, and keeps whichever of the snapshot and the events has the
highest revision. A revision tells newer from older but cannot recover an
event nobody heard, so the order matters: on the desktop
`onSchoolWebPage` resolves only once Tauri's `listen` is registered
(`features/school-web`'s `useSchoolWebPage`), and the mobile shells read the
snapshot after their event stream is running.

`school-web-notice` `{ site, kind: "reopened", url }` stays a one-time event:
a reload would have resent a form, so the page was opened again with GET. The
app shows it for a few seconds; a surface that misses it loses nothing.

Nothing is injected into a school page to show these.

## Page dialogs and printing

`alert`, `confirm` and `prompt` are shown natively for the view or popup the
person is looking at, titled with the initiating frame's origin, and answered
Cancel for a hidden view without any UI. `window.print()` opens the system
print panel for the foreground view only (WebKit: the private
`_webView:printFrame:pdfFirstPageSize:completionHandler:`). Android WebView
has no hook for a page's `window.print()`, so the page's button does
nothing there and the app's page menu prints the page on screen through
`PrintManager`. A `beforeunload` question is shown the same way, and a
hidden view always leaves.

A file input (`<input type=file>`) chooses files only for the page in front of
the view on screen, in a main view and a popup alike; anything else chooses
nothing:

- macOS: without the UI delegate's
  `webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:`,
  WebKit treats the request as Cancel, and a popup has no wry delegate under
  it. So the host's `SchoolUiDelegate` implements it itself, for main views
  and popups in one code path (wry's version is no longer reached). The
  completion handler is held by an answer-once guard that answers Cancel when
  dropped, because WebKit requires it to be called exactly once. It answers
  Cancel at once unless the view is the foreground site's page in front and
  its window is visible. Otherwise `NSOpenPanel` opens as a sheet on that
  window, choosing files, with multiple selection and directories as
  `WKOpenPanelParameters` asks. When the view is hidden, its popup closes,
  the view is disposed or the app quits, the host ends the sheet and answers
  Cancel. The sheet's handler holds the web view weakly and answers Cancel if
  it is gone or detached. A page that closes its own popup while the panel is
  open is covered by the same path.
- iOS: WebKit shows its own document picker; no delegate is involved.
- Android: `onShowFileChooser` launches `ACTION_GET_CONTENT` and answers the
  callback once, with nothing when the activity is cancelled.
  Android WebView adds its own windows for a `<select>` list or a date picker
  with the context the view was built on, so the host builds views over the
  activity in front (through a `MutableContextWrapper`, since views outlive it):
  over the application context those windows have no token and never open.
  School pages get no app script: Tauri injects
  every plugin's page script into every webview, so a desktop plugin must not
  have one.

## Platform status

| Host                             | Facts                                                                                           | History  | Popups                                                    | Drag and drop              | Print, dialogs                                                                    |
| -------------------------------- | ----------------------------------------------------------------------------------------------- | -------- | --------------------------------------------------------- | -------------------------- | --------------------------------------------------------------------------------- |
| macOS (`school_webkit.rs`)       | Full, `formResubmission`                                                                        | Reported | Inline, siblings of the view                              | Views and popups           | Done; file panel as above                                                         |
| Windows (interim)                | URL only (`Unknown`, no method or gesture); no resubmission facts                               | Unknown  | Separate windows (`NewWindowResponse::Create`)            | WebView2's own, per window | WebView2's own                                                                    |
| iOS (`SchoolBrowserHost.swift`)  | Full but gesture, `formResubmission`                                                            | Reported | Inline, siblings of the view                              | Views and popups (iPad)    | Done                                                                              |
| Android (`SchoolBrowserHost.kt`) | URL, method, frame, gesture, redirect; `onFormResubmission`; `ERR_CACHE_MISS` as a load failure | Reported | `WebViewTransport`, inline, siblings in the `FrameLayout` | WebView's own              | Dialogs and file inputs done; print from the page menu (no `window.print()` hook) |

macOS popups are views of their own (`WKWebView` subclass in
`school_webkit.rs`) with the capabilities of a main view: the file panel
above, file drops (they are siblings of the view), the opener's page zoom
when created and the app's zoom afterwards, and no keyboard focus while
hidden. The host logs when a popup opens, when a navigation becomes a
download, when a drag enters a popup and when a popup closes, with the site
and host only, never a path or query.

The Windows interim is a temporary exception to the history every other host
reports: it reports none, so `back` and `forward` stay `unknown` and Back and
Forward always go to the engine; its popups are separate windows with their
own controls, so the popup rule never applies. It ends when
`school_webview2.rs` reports history (`approach/school-web-windows-host-later`).

Android never resends a form from history: back or forward to an uncached
POST result fails with `ERR_CACHE_MISS`, reported as `Other`, and the
notice's reopen reloads that entry, which asks `onFormResubmission` and so
reaches Core's GET. `onCreateWindow` names no URL, so a popup is requested as
`about:blank` and its first navigation is decided from inside it; a popup
whose first page went to the system browser or was refused is closed.

Android WebView displays no PDF and cannot finish a download itself, so every
file a school page hands over reaches its `DownloadListener`, reported as
`becameDownload` with `Saved`, and goes to Core (`save_school_file`). School
hosts are reached only through a WebView or Core
(`constraint/school-network-two-stacks`), never the system `DownloadManager`.
Core fetches the file with the WebView's cookies and platform TLS verification,
from school-domain HTTPS hosts
only (`school_file_url`, checked on every redirect hop; a redirect to an
identity provider is a lost session), streams it to a `.part` file in the
shell's staging folder and publishes it under the name Core reads from
`Content-Disposition` (`inline` included, `filename*` first) or the URL and
type. The shell moves it into Downloads through MediaStore and shows a
toast. A file on another host opens in the system browser. The transfer
runs in the app's process, so it outlives a popup Core closes but not the
app being killed. Only a site on screen saves files; WebView gives no
request body, so a file that answered a POST is fetched again with GET, and
`blob:` or `data:` files are not saved.

The iOS host reports `becameDownload` as `Saved`: it writes every file and
previews it outside the web view. The Windows WebView2 host
(`NavigationStarting`, `NewWindowRequested` with a deferral and the same
environment, `ScriptDialogOpening`, `DownloadStarting`, its own record of
POST entries) replaces the interim later.
