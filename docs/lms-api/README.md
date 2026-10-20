# Science Tokyo LMS API

[OpenAPI 3.0.3](openapi.yaml) · [Swagger UI](index.html)

How Isct Companion reads the Science Tokyo LMS (Moodle 4.5.10 at
`https://lms.s.isct.ac.jp/2025`; the `2025` path also serves 2026 courses).
The client is `crates/isct-core/src/net/moodle/`: one `MoodleClient` with two
transports behind it.

| Transport | Used when | Authenticates with | Reads |
|---|---|---|---|
| Token (primary) | always tried first | mobile web-service token in the form body | `webservice/rest/server.php` |
| Cookie (fallback) | the token path is unavailable | WebView session cookie + `sesskey` | `lib/ajax/service.php` and course/activity HTML |

The custom timetable block (`data-block="tttimetable"`) has no web-service
function, so it is read from `/2025/my/` (and the `/2025/my/index.php`
year/quarter POST) with the session cookie on both paths.

To browse the OpenAPI file, from the repository root:

```sh
python3 -m http.server 8766 --bind 127.0.0.1 --directory docs/lms-api
```

then open `http://127.0.0.1:8766/`. The viewer loads Swagger UI 5.34.0 from
jsDelivr with subresource integrity, and has "try it out" disabled.

Sources: the Astra probe of 2026-10-20 (native reqwest/rustls requests from a
real account, no token or course content recorded) and our own probes of the
same day from the desktop dev app. Where they disagreed, our probe wins; the
differences are listed at the end.

## Getting a token (verified 2026-10-20)

No external browser and no custom URL-scheme handler are involved.

1. Generate a fresh, unpredictable `passport` for this launch.
2. With the LMS session cookie from the school WebView store, and redirects
   **disabled**, send

   ```http
   GET /2025/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport=<passport>&urlscheme=moodlemobile
   ```

3. The server answers **302** with
   `Location: moodlemobile://token=<base64>` directly; no intermediate page.
   A 302 to `/login/` or to the SSO host means the web session is gone: sign
   in in the WebView and launch once more.
4. Decode the base64 into `signature:::token[:::privatetoken]` (our run
   returned all three parts) and accept it only if
   `signature == md5(wwwroot + passport)` with `wwwroot =
   https://lms.s.isct.ac.jp/2025`. This proves the answer belongs to the
   launch this client started.
5. Keep only `token`. `privatetoken` is dropped at once; it is not a refresh
   token and no feature uses it.

Parse the `Location` with a dedicated parser (`net/moodle/token.rs`). A
generic URL parser treats the token as the URL's host; one probe wrote a
token into a result file that way.

### Storage and lifetime

- The token is stored only in the OS vault, item `lms-token-science`
  (`SchoolAuthService::{load,store,delete}_moodle_token`). It is
  device-only (not synchronized), never written to SQLite, logs, backups or
  cloud sync, and its `Debug` output is redacted.
- The vault record names the account it was issued to. A token found for
  another account is deleted, not used. Forgetting the school account (sign
  out) deletes it. "Clear school cache" keeps it.
- Its lifetime is unknown to the client. Upstream Moodle issues permanent
  tokens valid for the site's `tokenduration` (12 weeks by default), and the
  launch may return an existing token, so neither its creation time nor its
  expiry can be inferred. `tool_mobile_forcelogout=0`,
  `tool_mobile_autologout=0`; `tool_mobile_autologouttime=86400` is the value
  of a disabled setting, not a TTL. There is no refresh token.
- Students cannot view or revoke tokens: `/2025/user/managetoken.php` is an
  empty page for students and none of the 470 service functions revokes one.
  A leaked token stays valid until it expires or an administrator revokes it.
- On `invalidtoken` or `webservice_access_exception` (both arrive as HTTP
  200, see below) the client drops the token, confirms the WebView sign-in
  through `SignInGate`, launches once more and repeats the call. A token
  refused again ends the token path for that pull, which then runs on the
  cookie transport.

## REST calls

One RPC path: `POST /2025/webservice/rest/server.php`, form-encoded, with
`wstoken`, `wsfunction`, `moodlewsrestformat=json`, and
`moodlewssettingfilter=true` (text filtered as the web pages show it, which is
what the AJAX path returns). Arrays are sent as `courseids[0]=…`,
`courseids[1]=…`; objects as `options[0][name]=…`; booleans as `1`/`0`. Never
a JSON body or a comma list.

```http
POST /2025/webservice/rest/server.php HTTP/1.1
Host: lms.s.isct.ac.jp
User-Agent: Mozilla/5.0 (compatible; IsctCompanion/0.1)
Content-Type: application/x-www-form-urlencoded

wstoken=REPLACE_WITH_TOKEN&wsfunction=mod_assign_get_assignments&moodlewsrestformat=json&moodlewssettingfilter=true&courseids%5B0%5D=1001
```

- **Errors are HTTP 200** with `{exception, errorcode, message}`. Check this
  envelope before reading any success shape. `message` and `debuginfo` are
  never logged; they can echo request values.
- `warnings` in a success body means the result may be partial: nothing
  absent from it may be treated as deleted.
- A non-200 answer (a gateway page) is HTML, not a Moodle error.
- `Module.id` is the **cmid**; `Module.instance` is the assign/forum/quiz id.
  `Assignment.id` is the assignid and `Assignment.cmid` the cmid. In a
  discussion list `discussion` is the discussion id and `id` the first post.
  Timestamps are seconds; `0` usually means unset.
- The service also grants **write** functions (submission, `mod_ttws_*`
  evaluations, …). The app keeps a closed read-only list
  (`MoodleFunction`); a write cannot be expressed in the client.

### Functions the app calls

| Function | Purpose | Status |
|---|---|---|
| `core_webservice_get_site_info` | prove the token, user language | verified |
| `core_course_get_enrolled_courses_by_timeline_classification` | course list (category name = quarter) | AJAX verified; REST expected (mobile app service) |
| `core_course_get_contents` | course outline, section/module HTML, file URLs | verified |
| `core_calendar_get_action_events_by_timesort` | open work, paged by `aftereventid` | verified |
| `core_calendar_get_action_events_by_course` | open work of one course | listed |
| `core_calendar_get_calendar_upcoming_view` | upcoming non-action entries | listed |
| `core_calendar_get_calendar_monthly_view` | past due/close dates (full pull) | listed |
| `mod_assign_get_assignments` | assignment intro and attachments, assignid | verified |
| `mod_assign_get_submission_status` | own submission, grading, feedback | verified |
| `mod_quiz_get_quizzes_by_courses` | quiz id, maximum grade, decimals | listed |
| `mod_quiz_get_user_attempts` | finished attempts | listed |
| `mod_quiz_get_user_best_grade` | shown grade | listed |
| `mod_forum_get_forums_by_courses` | forum intro, forum id | listed |
| `mod_forum_get_forum_discussions` | announcements (first post) | verified |
| `mod_page_get_pages_by_courses` | page intro and content | listed |
| `mod_resource_get_resources_by_courses` | resource intro | listed |
| `mod_folder_get_folders_by_courses` | folder intro | listed |
| `mod_url_get_urls_by_courses` | URL intro and external URL | listed |

"Verified" means a native REST call returned the documented shape on
2026-10-20. "Listed" means the function is among the 470 the site's
`core_webservice_get_site_info` reports, but its answer has not been probed
yet; `SCHOOL-LMS-TOKEN-PULL` in `docs/verification/manual-checks.yaml`
covers that. `core_courseformat_get_state` is **not** in the mobile service;
`core_course_get_contents` replaces it. `core_course_check_updates` only
checks the modules passed in and cannot see new ones, so a course's outline
is re-read in full on every course read.

## Files

Take `contents[].fileurl` as returned. The app stores it without credentials
and without the web-service path (`…/webservice/pluginfile.php/…?token=…` is
stored as `…/pluginfile.php/…`), so a stored link survives a change of
transport or token. A download adds the credential of the transport in use:
`token=` (not `wstoken`) on the `webservice/pluginfile.php` form, or the
session cookie on the plain form; same origin only.

An invalid token on a file request returns **HTTP 200 with JSON**
(`invalidtoken`), not the file. Check content type, magic bytes and size;
never trust the status alone. Range requests work (`bytes=0-1023` → 206 with
`Content-Range: bytes 0-1023/45268`, matching the full file's prefix).

## Cookie fallback

- `GET /2025/my/` with the session cookie; `M.cfg.sesskey` and
  `M.cfg.language` are leaf values of a JSON literal in the page. No sesskey
  means a sign-in page.
- `POST /2025/lib/ajax/service.php?sesskey=…&info=<function>` with the JSON
  batch `[{"index":0,"methodname":…,"args":{…}}]`. Only functions marked
  `ajax` work there; `core_course_get_contents` and the module getters answer
  `servicenotavailable`. A non-JSON answer is a sign-in page; `invalidsesskey`
  fetches a new sesskey once.
- Structure comes from `core_courseformat_get_state` (a JSON string), and
  teacher text, activity details, forum posts and submission states from
  `course/view.php` and `mod/*/view.php` HTML (`school_parser::lms`).

What the fallback cannot provide compared with the token path: file URLs of
resources whose page was not read in this pull (the token path gets every
`fileurl` from the outline; the cookie path reads at most 40 activity pages
per course and 320 per pull), folder file lists in the body, and section or
module text when the course page cannot be read. Session-dependent: after an
app restart the cookie path needs a WebView sign-in first; the token path
does not (except for the timetable block).

## Timetable block

`/2025/my/` shows the selected year and quarter; the block's form posts
`year`, `quota` and the hidden `search` value to `/2025/my/index.php` (keep
`search`: without it Moodle ignores year and quota). Two tables: the period
grid (first row header, first column period 1–12, columns Monday…Sunday) and
the unscheduled courses. This POST is a read-only filter query and is the only
POST the cookie path sends besides AJAX reads.

## Corrections to the Astra notes

1. Getting a token needs no external browser or URL-scheme handler: one Rust
   GET of `launch.php` with the LMS cookie returns the 302 directly.
2. `privatetoken` was returned (three parts). Treat it as optional and never
   store or log it.
3. The gateway 403 for a default curl User-Agent did not reproduce from our
   network: curl's UA, no UA and the app UA all reached Moodle. The app UA
   is still always sent (reqwest sends none by default).
4. Public config works as `POST service-nologin.php` with a JSON body and as
   `GET` with `args=`.
5. The probed account had 46 courses (Astra's had 62); a count is not a
   property of the API.
6. `core_courseformat_get_state` is not in the mobile service.
7. `core_course_check_updates` cannot detect new modules; structure is re-read
   with `core_course_get_contents` on every course refresh, and only
   per-module details may be fetched incrementally.

Read-only and account-scoped: confirming any of this needs a real account,
which is why the checks live in `docs/verification/manual-checks.yaml`.
