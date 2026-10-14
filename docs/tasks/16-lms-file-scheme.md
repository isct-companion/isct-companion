# Task 16 — LMS files through an app URI scheme

## Goal

Every LMS resource the UI shows (attachments, PDFs, images inside labels and announcements, `mod/resource` links) is requested through one app-owned URI scheme. Rust answers each request from the local course-file cache, or resolves and downloads it through the signed-in LMS session, or fails with a typed status that the UI turns into a fallback. The frontend no longer distinguishes local from remote files, and no failed crawl can make a material unopenable.

Crawl-time detail reads (task 3) become a prefetch optimisation. They are no longer the only way a `view.php` link becomes a file.

## Today

- `open_course_material` (`backend/src/lms_files.rs`) takes a `pluginfile.php` URL, maps it to `<term>/<course>/materials/<name>-<url-digest>.<ext>`, returns the path when the file exists, and otherwise downloads it through `lms_downloader.rs`. The frontend turns the path into an asset URL with `convertFileSrc` (`src/features/lms/files.ts`).
- `material_url` accepts only `pluginfile.php`. A `mod/resource/view.php?id=N` link can be opened only when the LMS crawl already stored its `resourceUrl`. Without it the viewer shows "File details have not been synced" (`LmsWorkspace.tsx`, material viewer branch).
- `lms_downloader.rs` drives one hidden same-session webview with a single job slot (`turn` mutex). Downloads are strictly serial.
- Images inside rich LMS HTML (`RichContent`) and thumbnails (`thumbnails.ts`) go through their own paths.
- A crawl bug (fixed on 2026-10-08) wrote module details onto copies, so no `resourceUrl` or `bodyHtml` ever reached the cache. The crawl now gives each course its newest 40 modules and caps an import at 320. Anything beyond that relies on cached details, which is why lazy resolution is needed.

Read `docs/course-file-storage.md` (storage contract, TTL), `docs/lms-performance.md` and `docs/data-ownership.md` first. This task changes the first and must update it.

## Scheme

Register an asynchronous URI scheme protocol on the Tauri builder (`register_asynchronous_uri_scheme_protocol`), named `lmsfile`.

**Address.** The frontend never hand-builds the origin. Add one helper, `lmsFileUrl(source, context)` in `files.ts`, which produces:

```text
macOS / iOS / Linux:  lmsfile://localhost/get?u=<encoded LMS URL>&term=<t>&course=<c>
Windows / Android:    http://lmsfile.localhost/get?u=<encoded LMS URL>&term=<t>&course=<c>
```

- `u` is the original LMS URL as stored: a `pluginfile.php` URL or a `/mod/{resource,url,folder}/view.php?id=N` URL.
- `term` and `course` are `CourseFolder` components, validated exactly as `open_course_material` validates them today.
- Optional `download=1` marks a user-initiated download. It bypasses the auto-download size limit, as an explicit download does today.

**Allowlist.** It is checked before any work and again on every redirect hop:

- exact origin `https://lms.s.isct.ac.jp`, with no custom port or credentials;
- any academic-year path containing `pluginfile.php`, or ending in
  `/mod/(resource|url|folder)/view.php` with a numeric `id`.

Anything else answers `400`. `backend/tauri.conf.json` sets no CSP today. If one is added later, it must allow this scheme in `img-src`, `media-src`, `frame-src` and `connect-src` only. The asset protocol scope stays as it is: the scheme replaces the frontend's `convertFileSrc` use for LMS files, not the asset protocol for notes.

## Resolution order

For each request:

1. **Resolved URL known.** Look up the material's stored `resourceUrl` by source URL, or use `u` itself when it is already `pluginfile.php`. If the cached file exists, serve it and update its modification time (the TTL access marker in `course-file-storage.md`).
2. **Known but not on disk.** Download it through the LMS session (see Concurrency), then serve it. A file removed by TTL cleanup or a changed storage root is an ordinary cache miss; there is no separate "local link invalid" state.
3. **`view.php` with no resolved URL.** Resolve it with Rust HTTP using cookies from the hidden signed-in LMS webview: a same-origin `GET`, then
   - a redirect to `pluginfile.php` gives the file URL;
   - otherwise the page is parsed with the selectors task 3 already uses (`.resourcecontent img|object|iframe|video source|audio source`, `.resourceworkaround a`, `.resourcecontent a`), after checking `body.cmid-{id}`;
   - `mod/url` reads `#region-main a[href^="https://"]`;
   - `mod/folder` reads `.foldertree a[href*="/pluginfile.php/"]` and answers with a JSON file list (below), not bytes.

   **Write the resolved URL back** to the school cache row for that material (`lms:{scope}:material:{cmid}`) through the existing versioned write. An unchanged hash is not rewritten. Then continue at step 2.
4. **Failure.** Answer with a typed status (next section). Never redirect the main window to the LMS: the main window must not load school pages.

**One native parser.** Lazy resolution passes the bounded response body to
`school_parser::lms::parse_resource_page`. The downloader owns the request,
cookie and file lifecycle but contains no HTML selectors. Crawl-time extraction
may still populate the same `resourceUrl` as a prefetch optimisation; the Rust
parser is authoritative when a resource is opened.

**Never store local paths.** The database stores only remote URLs (`url`, `resourceUrl`). Whether a local copy exists is decided per request from the URL digest.

## Responses

| Status | When | Body / headers |
|---|---|---|
| `200` / `206` | File served | Bytes. `Content-Type` from the file. `Content-Length`. `Accept-Ranges: bytes`. Honour a single `Range` request with `206` and `Content-Range` (PDF.js and media use it). `Cache-Control: private, max-age=0, must-revalidate`. `ETag` = digest + size + mtime. |
| `200` JSON | `mod/folder` | `{ "kind": "folder", "files": [{ "name", "url" }] }`, where each `url` is already an `lmsfile` address |
| `200` JSON | `mod/url` to an external site | `{ "kind": "external", "url": "https://…" }`. The UI opens it through the existing external-link allowlist, never inside the app. |
| `401` | LMS session missing or expired (login page seen) | JSON `{ "reason": "login" }` |
| `404` | Resolution found no file, or `cmid` mismatch | JSON `{ "reason": "unresolved" }` |
| `413` | Over the auto-download limit and `download=1` absent | JSON `{ "reason": "tooLarge", "bytes": n }` |
| `502` / `504` | Network failure / timeout | JSON `{ "reason": "network" }` |
| `400` | Not on the allowlist or invalid folder components | JSON `{ "reason": "invalid" }` |

Every JSON error also carries `X-Isct-Reason`, so `<img>` and `<iframe>` error paths can be diagnosed without reading the body.

## Concurrency

The current downloader serialises everything. Replace the single job slot:

- **Single-flight per target file.** Concurrent requests for the same URL wait on one download.
- **Small files in parallel.** A file whose size is known to be ≤ 2 MB, either from the probe's `Content-Length` or from a previous attempt, runs in a pool of up to 4 parallel downloads. Typical cases: images in announcements and labels, thumbnails.
- **Large or unknown-size files** run in one lane, one at a time, as today.
- **Foreground first.** A request from the active reader (the file the person opened) goes ahead of queued prefetches and inline images. Queued work is not cancelled when the reader changes; it is only reordered.
- Keep the `.part` then rename behaviour, so a half-written file is never served. Downloads stay `GET`-only through Rust's HTTP client. The hidden WebView supplies the signed-in cookie jar and owns login, but does not transfer file bytes.
- Keep the 180 s per-download timeout. Resolution (step 3) has its own 20 s timeout.

The native HTTP lanes are independent of WebView download callbacks, avoiding platform-specific `on_download` behaviour.

## Frontend integration

This is small, but it must land with the backend so the old path is removed rather than kept beside the new one.

- `files.ts`: add `lmsFileUrl`. `openMaterial` / `open_course_material` stay for the "reveal in Finder / open with" actions, which need a real path.
- `FilePreview`, `PdfDocument`, `ImageThumbnail`, `thumbnails.ts` and `RichContent` (`safeContent` link and image rewriting) request LMS resources only through `lmsFileUrl`.
- Viewer fallbacks, keyed by `reason`:
  - `login`: "Sign in to LMS" (opens the school login, as `showLoginPage` does);
  - `tooLarge`: the size plus a "Download" button that requests the same address with `download=1`;
  - `unresolved` / `network`: "Open in LMS", which opens the official in-app view (`kind: "official"`) at the original URL.

  This replaces the current "File details have not been synced" text.
- The browser preview build has no Rust. `lmsFileUrl` returns the original URL there, and the existing read-only behaviour stays.

## Out of scope

- The forum and page bodies (`bodyHtml`). They keep coming from the crawl; a lazy body fetch is a later task.
- Any change to login automation (see Rules in the task README).
- Cloud sync of downloaded bytes. Files stay device-local, per `data-ownership.md`.

## Acceptance

- Opening a `mod/resource/view.php` material whose `resourceUrl` was never crawled downloads and shows the file. Its `resourceUrl` is now stored, and a second open is served from the cache without touching the LMS.
- Deleting the cached file on disk, then reopening, downloads it again; no UI state is stale.
- An announcement with ten inline images loads them in parallel (at most 4 at once). A large PDF opened at the same time is not queued behind them.
- Two simultaneous requests for one URL produce one download.
- A PDF over the auto-download limit answers `413`. The viewer shows its size, and Download fetches it.
- With the LMS session signed out, a request answers `401`, and the viewer offers sign-in.
- A URL outside the allowlist, and a redirect leaving `lms.s.isct.ac.jp`, both answer `400` without any network request from the app window.
- `Range` requests return correct `206` slices (checked with a synthetic file).
- `docs/course-file-storage.md` describes the scheme, the resolution order and the response table; `docs/lms-performance.md` describes the download pools.

## Tests

- Rust unit tests: the allowlist (including the redirect hops), the `Range` parser, the resolution selectors against synthetic `view.php` fixtures (redirect, embedded image, embedded object, `resourceworkaround`, folder tree, `mod/url`, `cmid` mismatch, login page), single-flight, and pool routing by size.
- Frontend tests: the `lmsFileUrl` platform forms, and each viewer fallback by `reason`.
- Fixtures are synthetic HTML; no real course pages, names or files.

## Follow-up (2026-10-08): persist folder listings — done

A resolved folder is stored on its material as `folderFiles` (`FolderFile[]` in the `Material` contract) through `update_lms_folder_files`, the same versioned write `update_lms_resource_url` now shares. `open_course_material` and the `lmsfile` protocol answer from a stored listing without resolving, and the crawl keeps `folderFiles` when it rewrites the row. The frontend reads `folderFiles` first, then folder links in the crawled body, and resolves only when neither exists.
