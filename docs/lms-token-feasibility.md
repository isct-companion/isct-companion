# LMS Web Service token feasibility

Verified against Science Tokyo LMS on 2026-10-20. This is an investigation
record; the app still uses its existing session/AJAX transport. No credentials
or raw account responses are stored here.

## Result

The official mobile service returns a Web Service token to the signed-in
student. Its REST service exposes useful read APIs that are unavailable through
the page's AJAX allowlist. Native reqwest POST and resource byte downloads now
pass without browser cookies, making a token transport viable for much of the
current HTML extraction. See the [OpenAPI reference](lms-api/README.md) and
[sanitized native results](lms-api/verification.json).

The public `tool_mobile_get_public_config` response reports
`enablewebservices=1`, `enablemobilewebservice=1`, and `typeoflogin=2` (browser
login), with `/2025/admin/tool/mobile/launch.php` as its launch URL. The security
keys page was empty; that does not mean mobile Web Services are disabled.

With the user's explicit approval, the official launch flow for
`service=moodle_mobile_app` returned an app callback containing a Web Service
token. No private token was included in this run. Production must use a fresh,
single-use passport and validate the callback's site/passport correlation;
the investigation's fixed passport is not an implementation template.

## Live read checks

REST endpoint: `https://lms.s.isct.ac.jp/2025/webservice/rest/server.php`.
The initial browser checks used GET after explicit approval of credential-in-URL
exposure. All eight documented read functions subsequently passed native POST
with reqwest 0.12.28/rustls, no cookie jar, TLS verification enabled and redirects
disabled. Production should use POST bodies and redact credentials from logs.

| Function | Observed result |
|---|---|
| `core_webservice_get_site_info` | Authenticated successfully; Moodle 4.5.10+; 470 advertised service functions; file download and upload enabled. |
| `core_enrol_get_users_courses` | Returned 62 enrolled courses. |
| `core_course_get_contents` | Sample course returned 18 sections, 22 activities, 15 nonempty section summaries and 13 file entries. |
| `mod_assign_get_assignments` | Sample course returned 4 assignments, all with due dates, without warnings. Intro fields existed but were empty in this sample. |
| `mod_assign_get_submission_status` | One sample returned `new`; an earlier assignment returned `submitted`, graded status and feedback. |
| `mod_forum_get_forum_discussions` | Sample forum returned one discussion with message text. |
| `core_calendar_get_action_events_by_timesort` | Returned 10 action events. |
| `tool_mobile_get_config` | Native POST returned mobile-app settings: force logout and automatic logout disabled. The inactive auto-logout time setting was 86,400 seconds, not a token TTL. |
| Invalid-token control | Both browser and cookie-free native POST returned HTTP 200 with `invalidtoken`; HTTP status alone does not indicate API success. |

The advertised functions also include resource, folder, page, URL, quiz and
discussion-post readers. Advertisement alone is not a successful live check.
No timetable-specific function was advertised.

## Native transport and file bytes

- Default curl identification received an AWS load-balancer `403`. Using the
  existing app download User-Agent (`Mozilla/5.0 (compatible; IsctCompanion/0.1)`)
  made native curl POST succeed. The same identity in reqwest/rustls succeeded
  without cookies for all eight documented read functions. This is an observed
  client-identification dependency, not a claim about the school's full gateway
  rules. Python's local CA setup was a separate client issue; TLS verification
  was never disabled.
- A cookie-free native GET downloaded a PDF: HTTP 200, `application/pdf`, 45,268
  bytes matching the API metadata, `%PDF-` header and `%%EOF` marker. A Range GET
  returned HTTP 206, `bytes 0-1023/45268`, and 1,024 bytes matching the full prefix.
  An invalid token returned HTTP 200 with JSON `invalidtoken`, not PDF bytes.

## Lifetime and remaining checks

- Current token creation and expiry times remain unknown. Upstream mobile token
  creation uses a permanent-type token with no browser session binding and an
  expiry derived from site `tokenduration`; its default is 12 weeks. The school
  may configure another duration, and the launch flow may reuse an existing
  token. API access updates lastaccess without extending validuntil. See
  [upstream token generation](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/lib/external/classes/util.php).
- Repeated cookie-free native calls succeeded, but actual browser-session
  expiry, token expiry and revocation were not induced. There is no demonstrated
  refresh-token flow. The app must handle authentication rejection and obtain
  a token again through browser authorization while preserving cached data.
- School timetable blocks, embedded tools and other school-specific HTML may
  still need the authenticated web session. No full content-parity audit or
  performance benchmark was performed.

## Integration direction

Own the transport and mapping in `isct-core`, reuse the existing ingestion and
stored-data contracts, and keep native callback handling in platform ports.
Read assignment state and deadlines directly instead of inferring submission
from disappearing action events or scraping status tables when the API has
the authoritative value. Keep the existing path until each replacement is
verified against real data.

The mobile token's service includes write capabilities, including uploads.
The client must enforce a closed read-function allowlist; this is not a
server-enforced read-only credential. Keep any retained token in the OS
credential vault, outside workspace exports, sync, UI state and diagnostics.
The investigation's temporary credential and raw-response files are removed
after validation; the token has not been installed into the app or revoked.

Official implementation references:

- [Mobile launch flow](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/admin/tool/mobile/launch.php)
- [REST token authentication](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/webservice/rest/server.php)
- [Site info and available service functions](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/webservice/externallib.php)

## Dashboard timetable token probe — 2026-10-22

`core_block_get_dashboard_blocks` with `returncontents=1` returned the school's
`tttimetable` block using an existing mobile token, native curl POST, TLS
verification and no Cookie header or cookie jar. The token stayed in the POST
body. The result was HTTP 200 with no Moodle error or warnings.

The block's 24,835-character content contained both timetable tables and the
year/quarter filters. Wrapped in `data-block="tttimetable"`, it passed the
app's existing Rust `read_timetable` and `timetable_filters` parsers. The selected
term was 2026-3Q: five scheduled courses, eighteen day/period slots, and two
unscheduled courses. The exact slot and unscheduled-course sets matched the
iPhone's successful 13:22 JST enrolment sync; no course names or identifiers
were retained in the evidence.

This does not establish arbitrary-quarter access. Adding the block form's
`year`, `quota` and `search` fields to the REST request for quarters 1, 2 and 4
returned HTTP 200 with `errorcode=invalidparameter` each time. A final request
without those fields still returned 2026-3Q. The generic API exposes `userid`,
`returncontents` and `mypage`, rather than academic-year or quarter parameters
([Moodle 4.5 implementation](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/blocks/classes/external.php)).

The default dashboard quarter can therefore be read with token authentication.
Replacing the app's all-quarter cookie reader requires a separate decision
about earlier/later quarters; this probe did not change the app transport or
write browser preferences. Sanitized results, including the comparison and
rejected quarter probes, are under `dashboardTimetableTokenProbe` in
[verification.json](lms-api/verification.json).

## Core integration — 2026-10-22

Routine enrolment reads now use the token course list and default dashboard
block. The existing cookie reader remains the fallback for an unavailable or
unreadable block API; its all-quarter grids remain supported. No stored-data
schema changed, and a default-grid read keeps all cached quarters it did not
return. Missing or malformed blocks are errors, not empty timetables.

A live probe ran the actual Core facade, sync planner, HTTP transport, reader,
validator and ingestion against an isolated local workspace, with cookie and
browser ports that reject all requests. Background enrolment committed 62
courses, logged the token dashboard path, and made zero cookie-port calls and
zero browser navigations. The result is under `coreTokenIntegration` in the
sanitized verification file. The existing
`lms_quarter_grids_keep_clocks_overlays_and_removal_tombstones_across_official_refreshes`
test already protects preservation of quarters absent from a later read.

Further cross-quarter discovery found neither a timetable-specific function
among the token's advertised services nor `block_tttimetable` among the mobile
plugins (`mod_ltifmane` and `mod_subsection` were returned). The block's form
still targets the cookie-authenticated `POST /2025/my/index.php`. Moodle's
alternative auto-login exchange requires a matching private token, which the
app does not retain; no auto-login flow was added or claimed verified.
