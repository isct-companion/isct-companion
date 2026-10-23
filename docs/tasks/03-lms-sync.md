# Task 3 — LMS sync through Moodle AJAX

## Goal

Import the person's LMS courses, course structure, assignments and deadlines from Science Tokyo LMS (Moodle) into `lms_courses`, `lms_items`, `assignments` (task 1), using Moodle's own read-only AJAX functions instead of scraping. HTML only for the few things the API does not expose, on demand.

Course identity is provider-independent. `course_identity_map` records `(provider, external_id, academic_year) -> course_id`; an official 9-digit offering ID produces the canonical internal ID `<academicYear>-<offeringId>` (for example `2026-202602427`). LMS, syllabus and 教務 imports must resolve this mapping before writing any `courseId`. Display course codes are never treated as unique, and term/quarter belongs to membership or planning records rather than the course identity.

## Today

`backend/src/school_extract.rs` holds `T2SCHOLA_EXTRACT_SCRIPT`, evaluated by `backend/src/school_web.rs` on every LMS page load; it scrapes the DOM and relays JSON to Rust through a `isct-companion-extract://{nonce}/…` navigation, which `stage_school_payload` / `publish_school_cache` validate into `school-cache.json`. Keep the relay and staging idea; replace the scraping and the JSON file.

## Transport (verified 2026-09-28)

Inside a signed-in LMS page (`https://lms.s.isct.ac.jp/2025/…`; `2025` is the Moodle instance path and also serves 2026 courses):

```js
const sesskey = M.cfg.sesskey;
fetch(`/2025/lib/ajax/service.php?sesskey=${sesskey}&info=${methodname}`, {
  method: "POST", credentials: "same-origin",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify([{ index: 0, methodname, args }]),
}).then(r => r.json()); // → [{ error: false, data }] or [{ error: true, exception: { errorcode } }]
```

The extractor hard-codes this allowlist and refuses any other method name:

| Function | Args | Use |
|---|---|---|
| `core_course_get_enrolled_courses_by_timeline_classification` | `{classification: "all", limit: 0, offset, sort: "fullname"}` | Courses. Returns 50 per call; repeat with `offset = nextoffset` until it stops advancing. Fields: `id`, `fullname` (`日本語 / English`), `shortname` (`LAH.T209[2026]`), **`idnumber` (= syllabus id)**, `coursecategory`, `startdate`, `enddate`, `viewurl`, `hidden`. |
| `core_courseformat_get_state` | `{courseid}` | Structure. `data` is a JSON **string**: `{course, section[], cm[]}`. `section`: `id`, `number`, `title`, `cmlist`, `visible`. `cm`: `id`, `name`, `module` (`assign`, `resource`, `url`, `folder`, `forum`, `quiz`, …), `url`, `sectionid`, `visible`, `uservisible`. |
| `core_calendar_get_action_events_by_timesort` | `{timesortfrom, limitnum: 50, aftereventid?}` | Open work across courses. `timesort` (Unix), `overdue`, `course.id`, `instance` (= cmid), `modulename`, `activityname`, `action.actionable`. Only work still to do is listed. |
| `core_calendar_get_action_events_by_course` | `{courseid}` | Same, per course. |
| `core_calendar_get_calendar_upcoming_view` | `{courseid: 1, categoryid: 0}` | All upcoming dated events, including submitted work. |
| `core_calendar_get_calendar_monthly_view` | `{year, month, courseid: 1, categoryid: 0, includenavigation: false, mini: true, day: 1}` | Past deadlines when needed. |
| `core_course_get_recent_courses` | `{limit}` | Optional ordering. |

Not available through this AJAX surface (`servicenotavailable`): `core_course_get_contents`, `mod_assign_*`, `mod_resource_*`, `mod_url_*`, `gradereport_*`, `core_enrol_get_users_courses`, `core_webservice_get_site_info`. The separate mobile Web Service token exposes broader REST access: course contents, enrolled courses, assignment deadlines/status/feedback and forum discussions were read successfully on 2026-10-20; see [token feasibility and remaining checks](../lms-token-feasibility.md). The user grade-report page returned 無効なユーザです in the earlier AJAX investigation; individual assignment feedback is available through the token API. Official grades come from task 4.

Category ids seen: 2025 → 2/3/4/5 (1Q–4Q), 6 (他); 2026 → 8/9/10/11, 12 (他). Derive year/quarter from `shortname` (`[2026]`) and the linked offering first; the category map is a fallback.

## Mapping

- `lms_courses`: `lms_id = id`, `offering_id = idnumber` (when it is 9 digits), `shortname`, `title = split(fullname, " / ")` → `{ja, en}` (if there is no ` / `, store the whole string under the LMS page language `M.cfg.language`), `year`, `quarters`, `url = viewurl`.
- `lms_items`: one row per `cm` with `cmid`, `lms_course_id`, `section_number`, `section_title`, `module`, `name`, `url`, `visible && uservisible`. Teacher-written names are single-language strings.
- `assignments`: one per `cm` with `module = "assign"` (and `quiz` if present). `due_at` from the matching calendar event's `timesort` (upcoming/monthly view, `eventtype = "due"`); `submission = "open"` while it appears in action events, `"submitted"` once a previously open item disappears before its deadline, otherwise `"unknown"` until task-3 HTML detail says more. Stable identity: `lms_cmid`.
- `memberships`: `lms-member` for each linked course; link to the timetable course via `offering_id`.

## HTML, on demand only

Fetched when the person opens an item, not during sync.

| Page | Selector | Value |
|---|---|---|
| `mod/assign/view.php?id={cmid}` | `.submissionstatustable table tr` → `th`, `td` | Status is in the **`td` class**, not the text: `submissionstatussubmitted` / `submissionstatusnew` / `submissionstatusdraft`; `submissiongraded` / `submissionnotgraded`; `earlysubmission` / `latesubmission` / `overdue`. |
| same | `.feedback .feedbacktable table tr:first-child td` | Grade as shown (`100.00 / 100.00`, or a scale item), when it is plain text. Rows have no classes; the grade row comes first, followed by its grading date. Without a grade the first row is the grader or a feedback comment, both containing markup. Verified on `id=131787`, 2026-10-15. |
| `mod/quiz/view.php?id={cmid}` | `#feedback h3` | Final grade as a sentence (`あなたの小テスト最終評点は 100 / 100 です。`); only the `grade / maximum` pair is read, which makes the quiz `graded`. Absent while hidden, without numbers while answers await grading. Verified on `id=138532`, 2026-10-15. |
| `mod/assign/view.php?id={cmid}` | `[data-region="activity-dates"] div` | Open/due dates as localized text; use only if the calendar has none (format `期限: 2026年 06月 23日(火曜日) 00:00` / English equivalent). |
| same | `#intro` | Assignment description (HTML → plain text, links kept). |
| same | `body` classes `cmid-{n}`, `course-{n}` | Identity check before storing. |
| `mod/folder/view.php?id={cmid}` | `.foldertree a[href*="/pluginfile.php/"]` | File list: name (link text), URL. Files are not downloaded. |

Daily enrolment now reads the default `tttimetable` through the token-backed
`core_block_get_dashboard_blocks(returncontents=1)` API, reusing Core's HTML
parser. Absent quarters retain their cached grids. The legacy cookie fallback
still reads other quarters through `my/index.php` (`year`, `quota`, `search`);
REST rejects those filter fields, and the school advertises no timetable
function or mobile handler. Do not change browser preferences to make a token
request select another quarter. See the 2026-10-22 verification record.

## Build

1. Replace `T2SCHOLA_EXTRACT_SCRIPT` with an extractor that calls only the allowlisted functions, paginates courses to the end, loads structure for courses of the current and next quarter, and returns one bounded payload through the existing relay. Run it once after sign-in completes and when the person opens LMS, not on every page load.
2. Rust validates (ids numeric, URLs on `lms.s.isct.ac.jp`, sizes bounded), then writes the tables in one transaction and records an `import_runs` row. Tombstone courses only after a complete paginated list; structure only per course that loaded completely.
3. Remove `school-cache.json` handling.

## Fixtures

Synthetic JSON responses for each function (shapes above; ids such as course `11663`, `idnumber` `202600907`, `shortname` `LAH.T209[2026]`, `fullname` `社会モデリングＢ / Social Modeling B`) and small synthetic HTML for the two on-demand pages with the classes listed.

## Acceptance

- Pagination stops at the end and a partial list never tombstones.
- `fullname` splits into `{ja, en}`; a name without ` / ` is stored under the page language.
- `offering_id` links to a task-2 offering when present.
- An assignment moves `open → submitted` when it leaves the action-event list before `due_at`.
- The extractor rejects a non-allowlisted method name; a login page instead of JSON produces `reconnect-required` and keeps existing rows.
