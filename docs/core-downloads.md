# Course materials and downloads

This is the Phase 2c service boundary. The complete Core/FFI API catalogue is
still part of the consolidation plan; these Rust services are not yet exported
through Swift, Kotlin or MCP bindings.

## Shared services

`CoreRuntime` owns one `Downloader` and one `LmsSession`. Construct
`CourseMaterials` with the shared data store, the platform's storage root, that
downloader and an optional `DownloadSession` port. Do not create a downloader
per command: its flight table, transfer limits and shutdown scope must be
shared by every caller.

| Entry point | Behaviour |
| --- | --- |
| `CourseMaterials::open(context, source, download, read_text, limit)` | Resolve an LMS resource and return a cached file, folder, external URL, missing result, size limit or typed failure. `download=false` permits resolution but does not fetch file bytes. |
| `CourseMaterials::preview(context, source, explicit, foreground)` | Apply the saved automatic size limit (default 20 MiB, bounded to 1–1024 MiB). An explicit download has no size limit and foreground priority. |
| `CourseMaterials::clean(days)` | Remove stale complete material cache files from current and legacy layouts. Retention is bounded to 1–3650 days. |
| `LmsSession::cookie_header(port, url)` | Prepare the shared native session, apply readiness/retry policy and obtain cookies for the requested LMS URL. |
| `Downloader::shutdown()` | Stop admission, signal cancellation and drain owned download/resolution workers. Called by `CoreRuntime::shutdown()`. |

The serialized `Material` results preserve desktop's `state` discriminator.
`MaterialFailure` exhaustively represents `login`, `invalid`, `network`,
`timeout`, `cancelled` and `unresolved`. Input/storage/unsupported-operation
errors remain distinct `MaterialError` variants. Adapters map these into their
own protocol or UI response without repeating material acquisition policy.

## Native boundary

Cookie storage belongs to the native browser session: WKWebView, Android
WebView and Tauri expose different APIs. `LmsSessionPort` reports school and LMS
readiness, requests the existing school session, creates/navigates the native
view, and reads cookies. The Core session owns the wait and retry decisions;
HTTP requests consume its cookie header. No second persistent cookie jar is
introduced.

The existing school queue still executes login and browser navigation. Its
full state-machine migration is Phase 3. This checkpoint moves only the
download caller's session preparation policy and removes the previous native
wait loop. Core serializes readiness to avoid racing native view creation.
It retains the 180-second school wait, two 20-second LMS readiness windows and
one five-second reload delay. Transport allows 230 seconds for cookie
preparation and 30 seconds for an HTTP request; probe/resolution have a
260-second overall bound. The file transfer retains its 180-second bound.

The desktop adapter owns custom-protocol response headers/ranges, asset-scope
grants and system file opening. Its range responder reads only the requested
bytes from the same file handle used to obtain metadata. A full response still
requires a response buffer. Assignment editor/workspace integration and note
sync have not moved in this checkpoint.

## Cache and task rules

- Preserve the existing course/term paths and readable URL-hashed filenames.
  Cached files remain usable without an authenticated download port and
  without revalidation; ETag freshness is not introduced here.
- Join callers only when target path and size limit match. Different limits
  serialize on the target and recheck the completed cache. Source locks avoid
  simultaneous transfers of the same URL into different targets.
- Known files up to 2 MiB share four transfer lanes with foreground priority.
  Unknown-size and larger files use one lane. Probe requests precede lane
  acquisition, as before.
- The worker survives the caller dropping its future. Every completed or
  panicking worker removes its flight before waking waiters, permitting a
  fresh retry. Shutdown rejects new operations and cancels network waits.
- Download to a unique `create_new` sibling, flush/sync it, then rename to
  publish a complete file. Error, cancellation and rename failure clean the
  temporary file. Cleanup excludes partial files, symlinks, notes and
  assignment workspaces, including nested user directories.
- Resource-page parsing limits accumulated input to 1,000,000 bytes before
  declining an oversized page, including responses without Content-Length.

Desktop MCP material operations will need to call the running desktop's Core
through the planned local bridge to reuse its native session and task scope.
The existing MCP authorization/audit boundary still applies. No separate MCP
download runtime or new tool registration is added by this checkpoint.
