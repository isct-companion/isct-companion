# Course file storage contract

This is the authoritative contract for downloaded LMS files and local course
notes. School record metadata remains in `school.db`; file bytes do not.

## Types at the frontend/native boundary

All command fields use camel case. TypeScript `number` values marked as
integers below must be finite whole numbers in the stated range; Rust receives
them as unsigned integers and applies the same bounds again.

```ts
type CourseFolder = {
  term: string; // validated folder component, at most 120 ASCII characters
  course: string; // validated folder component, at most 120 ASCII characters
  section: string; // notes only; same validation
};

type LocalFile = {
  id: string; // source URL for materials; UUID for notes
  name: string;
  section: string;
  path: string; // absolute native path; accepted back only after root validation
  bytes: number; // non-negative integer bytes (Rust u64)
  cached: boolean;
  text?: string | null; // UTF-8 only, returned only when requested and <= 1,000,000 bytes
};

type MaterialResult =
  | { state: "ready"; file: LocalFile }
  | { state: "missing" }
  | { state: "tooLarge"; bytes: number };
```

## LMS file transport

All native LMS transfers are owned by Rust. The reader calls
`open_course_material`; Rust resolves and downloads the file, then the frontend
uses a scoped local asset URL. The `lmsfile` protocol remains available for
inline/range-based consumers and is built only through `lmsFileUrl`:

```text
macOS / iOS / Linux: lmsfile://localhost/get?u=…&term=…&course=…
Windows / Android:   http://lmsfile.localhost/get?u=…&term=…&course=…
```

The network boundary is the exact HTTPS origin `https://lms.s.isct.ac.jp`
(no credentials or custom port), independent of Moodle's academic-year path.
Business inputs must still be a `pluginfile.php` URL or a numeric
`mod/{resource,url,folder}/view.php?id=…` address. Redirects are checked against
the origin again. Folder components use the same validation as notes.
`download=1` denotes an explicit user download and bypasses the automatic size
limit.

Resolution is cache-first. A stored `resourceUrl`, or an already resolved
`pluginfile.php` URL, maps deterministically to the course materials path. A
missing file is downloaded again by Rust's HTTP client using cookies copied
from the signed-in LMS WebView. The WebView owns login only; it does not perform
file transfers. An unresolved `view.php` is fetched with a read-only GET and
parsed in Rust; successful file resolutions are written back to the versioned
LMS item. Only remote URLs are stored in SQLite.
Folder activities return a JSON list of already wrapped `lmsfile` URLs, while
external `mod/url` activities return a typed external-link JSON response.

File responses provide `Content-Type`, `Content-Length`, `Accept-Ranges`, an
ETag based on the URL digest/size/modification time, and private revalidation.
A single byte range receives `206` and `Content-Range`. Typed JSON failures are
also exposed as `X-Isct-Reason`:

| Status | Reason | Meaning |
| ---: | --- | --- |
| 401 | `login` | LMS session is missing or expired |
| 404 | `unresolved` | The activity did not resolve to a file |
| 413 | `tooLarge` | Automatic download limit exceeded |
| 502 | `network` | LMS request failed |
| 504 | `timeout` | Resolution or download timed out |
| 400 | `invalid` | URL or course folder left the allowlist |

The persisted settings are JSON integers:

| Setting                        | Type and range        | Default | Meaning                                                                                             |
| ------------------------------ | --------------------- | ------: | --------------------------------------------------------------------------------------------------- |
| `settings.autoDownloadLimitMb` | integer, 1..1024 MB   |      20 | Maximum size downloaded automatically for a preview. A user-initiated download has no policy limit. |
| `settings.autoCleanDays`       | integer, 1..3650 days |      30 | Retention TTL for downloaded materials.                                                             |

Rust owns the defaults and bounds in `CourseFilePolicy` and generates the
TypeScript constant used by the frontend; Rust also clamps command arguments
at the trust boundary. These values are device preferences and are not part of
the user-data backup.

## TTL semantics

The TTL applies only to downloaded LMS material bytes, never to notes or LMS
metadata. It is an inactivity TTL:

- a successful open updates the file modification time;
- once per application launch, files whose modification time is older than
  `autoCleanDays * 24 hours` are removed;
- cleanup ignores symbolic links, notes and deleted notes; stale `.part` files
  in a materials directory are eligible for the same cleanup;
- changing the setting does not run cleanup immediately; it takes effect on
  the next launch.

The modification time is the access marker because reading atime is unreliable
on common desktop filesystems. Cleanup is best-effort: a file that cannot be
inspected or removed is kept.

## Storage locations

The default root is:

- desktop: OS Documents directory / `IsctCompanion`;
- desktop fallback: app data directory / `IsctCompanion`;
- mobile: app data directory / `IsctCompanion`.

A custom desktop root is stored separately as
`<app-data>/course-files-root.json`; changing it does not move or delete the old
root. Under the active root:

```text
<term>/<course>/materials/<readable-name>-<url-digest>.<extension>
<term>/<course>/<section>/notes/<uuid>.md
<term>/<course>/<section>/deleted-notes/<uuid>.md
```

Materials are shared by all sections of a course and keyed by the full,
allowlisted LMS attachment URL. Downloads first use a sibling `.part` file and
are renamed only after completion. File paths coming back from JavaScript are
canonicalized and must remain inside the active root before they can be opened
or revealed.

Markdown notes are local user data: the UI labels them “Local note · stored
only on this device”. They are included in manual local export/backup (including
soft-deleted notes), but are outside the future cloud-sync boundary. Downloaded
materials and custom-root configuration are never exported; another device can
download official materials again.

Unsaved browser recovery drafts are capped at 1 MB each, 20 entries and 5 MiB
total, and expire after 30 days. They are removed after a successful save.
