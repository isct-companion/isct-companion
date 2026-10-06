# Task 5 — Subscriptions: academic calendar and departments

## Goal

Reference data comes from a public GitHub repository (not created yet), but its two version axes are independent. Academic calendars are selected by `academicYear`; departments and recommended courses are selected by the student's `admissionYear` because the official 学部学修案内 is cohort-specific. A student admitted in 2025 continues to use `guide_2025` recommendations in academic year 2026. The app validates and applies both through the versioned-data update (task 1).

## File format

Repository layout (reference for the future data repository):

```
schema/academic-calendar.schema.json
schema/departments.schema.json
data/2026/academic-calendar.json
data/2026/departments.json
index.json          # separate academicYears and admissionYears maps
```

`academic-calendar.json`:

```json
{
  "schemaVersion": 1,
  "academicYear": 2026,
  "source": [{ "title": "…", "url": "https://…" }],
  "quarters": { "1Q": { "start": "2026-04-03", "end": "2026-06-09" }, "2Q": {…}, "3Q": {…}, "4Q": {…} },
  "days": [
    { "date": "2026-07-23", "type": "replacement", "replacementDay": 1, "label": { "ja": "月曜日の授業", "en": "Monday classes" } },
    { "date": "2026-10-09", "type": "no-class", "label": { "ja": "工大祭のため授業休み", "en": "No classes due to Campus Festival" } }
  ]
}
```

`departments.json`:

```json
{
  "schemaVersion": 1,
  "admissionYear": 2026,
  "source": [{ "title": "2026年度入学者用 学部学修案内", "url": "https://www.titech.ac.jp/guide/guide_2026/gakubu1/" }],
  "departments": [
    {
      "id": "cs",
      "name": { "ja": "情報工学系", "en": "Computer Science" },
      "syllabusScope": "4/0-904-342300-0-0",
      "unitCode": "342300",
      "guidePdf": "https://www.titech.ac.jp/guide/guide_2026/gakubu1/pdf/27-13.pdf",
      "recommended": [
        { "category": { "ja": "専門科目", "en": "Major courses" }, "level": 200, "code": "CSC.T241", "group": "A",
          "mark": "recommended", "title": { "ja": "計算基礎論" }, "credits": { "lecture": 2, "exercise": 0, "lab": 0 },
          "competencies": [1], "content": ["a"], "note": "A群" }
      ]
    }
  ]
}
```

- `mark`: `required` (◎), `recommended` (○), or omitted. `group` is the suffix letter of the guide's code (`CSC.T241.A` → `A`).
- `admissionYear` is the guide/cohort version, not the current academic year. Do not fall back from a missing admission-year guide to the current year's guide.
- The 17 departments and scopes are in the model doc ("Departments"). Bundle `data/2026/departments.json` with all 17 departments (name, scope, unit code, PDF URL) and `recommended` for 情報工学系 taken from `docs/tasks/data/cs-2026-recommended.json` (hand-transcribed 付表 pp.170–171 of 27-13; the 100-level part is missing — check the PDF); other departments start with `"recommended": []`.
- Bundle `data/2026/academic-calendar.json` with quarter dates and holidays only if taken from an official source cited in `source`; otherwise ship quarters only and leave `days` empty. Do not invent dates.

## App side

- Settings → Subscriptions: the public repository default index URL (bundled files remain a legacy/offline reference), custom URL, enable/disable, last update, errors.
- Fetch only `index.json`, hash its raw bytes locally, select the calendar from `academicYears[academicYear]`, and independently select the guide from `admissionYears[academicProfile.admissionYear]`; each selected entry declares `source`, canonical JSON `hash`, and RFC 3339 `ts`. If the local index hash is unchanged, skip dataset requests. `GET` over HTTPS only, size-bounded, no cookies. Validate against the schemas (embed them); reject the whole selected file on any error and keep its current data.
- Apply through `update()` from task 1; same hash → no change. Write `academic_calendar`, `departments`, `recommended_courses`.
- Department and recommendation row ids include the admission cohort (`cohort:{admissionYear}:…`) so applying one cohort never overwrites or tombstones another cohort.
- Check on launch at most once a day.
- `calendar_exceptions` (the person's own) override subscription days on the same date.

## For the future data repository (document only)

Separate yearly GitHub jobs process the academic calendar for the academic year and the official `guide_{admissionYear}/gakubu1/` pages/PDFs for the admission cohort. Each opens a pull request for human review.

## Acceptance

- Schema tests: valid bundled files pass; missing field, wrong date, unknown `type` fail.
- Update test: unchanged file → no write; changed holiday → one row changed.
- Recommended course `CSC.T241` group `A` is found for the 情報工学系 department.
- A 2025-admission profile selects `guide_2025` even while the calendar selection is academic year 2026; applying 2026 cohort data does not overwrite 2025 cohort rows.


## Public repository integration contract (2026-10-03)

### Calendar v1/v2: accepted by the native loader

Use `schema/academic-calendar.schema.json` (v1 compatibility, v2 teaching intervals). The public
repository entry point is an HTTPS URL to `index.json`. Its calendar paths are
relative to that index, selected by `academicYears[academicYear].academicCalendar.source`.
Keep admission-cohort guides in the independent `admissionYears` map. The calendar
and guide refresh commands select and validate their own category independently;
a calendar-only index is sufficient for calendar refresh.

Calendar rules:

- `academicYear` means the school year (April through the following March), not
  the calendar year of each row. Dates are `YYYY-MM-DD` in `Asia/Tokyo`.
- `quarters` contains exactly `1Q`–`4Q`, each with inclusive `start` and `end`.
  Ranges must be valid, ordered, and non-overlapping.
- `days` contains at most one teaching override per date. Types are exactly
  `holiday`, `no-class`, and `replacement`; `break` is a local override type,
  not a public calendar type.
- The Settings calendar JSON import/export is a separate local-override format
  (`schema`, `version`, `exceptions`), not this public subscription file. Do not
  publish that export as `academic-calendar.json`.
- `replacementDay` is required only for `replacement`, with Monday = 1 through
  Sunday = 7. It replaces that date's ordinary weekly timetable; it does not
  add another timetable on top. Local overrides have priority over public rows.
- `holiday` and `no-class` suppress weekly courses. Personal appointments stay
  on their actual civil dates; replacement weekdays must not move them.
  Explicit intensive dates remain user-owned and are not rewritten by the feed.
- Labels are `{ "ja": "…", "en": "…" }`, with `ja` required and `en` optional.
  `source` must contain official HTTPS citations. Unknown dates must be omitted;
  AI generation is not evidence for an academic date.
- Publish complete reviewed files, not patches. Invalid downloads retain the
  prior valid file. Public files contain no app-local IDs, revisions, credentials,
  student names, grades, submissions, or downloaded teaching materials.
- Do not put `exam` rows or additional root fields into calendar v1: the native
  decoder rejects unknown types and fields.

### Exam feed v1: separate contract, app adapter not implemented

The exam format is defined by
`backend/subscriptions/schema/exams.schema.json`. A future public repository
copies it to `schema/exams.schema.json` and publishes `data/<academicYear>/exams.json`.
It contains exact per-course exams. Broad exam periods do not generate individual
exams and are outside this feed's v1 scope.

- Root: `schemaVersion: 1`, `academicYear`, `timezone: "Asia/Tokyo"`, `source`,
  and `entries`. `source` records have stable `id`, `title`, and official HTTPS
  `url`; each entry's `sourceIds` references those IDs.
- `kind: "exam"`: stable `id`, `quarter`, `courseCode`, `date`, localized `label`,
  and `sourceIds`, plus explicit `room` and `status` fields; exact clock fields are optional.
  Optional `syllabusCourseId` identifies the public syllabus
  offering, not a local workspace UUID. Optional `classLabel` distinguishes
  sections. A code that matches multiple local offerings is not auto-attached.
- Unknown times omit both `startTime` and `endTime`;
  unknown room uses `room: null`. When times are known, use `HH:mm` with end after
  start. `room` is localized. Do not infer exam clock times from teaching periods.
  When the source gives teaching periods rather than clock times, retain the
  official `[first, last]` in `periods` and omit clock fields. The allocation
  is not proof of exact exam duration. A date-only exam remains date-only.
- IDs survive rescheduling; they must not be generated from mutable dates/times.
  Entries must have unique IDs, valid dates within the academic year, resolvable
  `sourceIds`, ordered times, and nonempty labels/codes. These relational checks
  supplement JSON Schema validation.
- Exam dates may fall after a quarter's teaching end; `quarter` associates the
  exam with its course quarter rather than restricting it to teaching dates.
- Do not add an `exams` key to the current `index.json` yet: its native decoder is
  strict. Adding discovery, storage, matching, and display is a separate adapter
  change; placing the file in the repository alone does not enable app exams.

Example only (synthetic course/date; not school facts):

```json
{
  "schemaVersion": 1,
  "academicYear": 2026,
  "timezone": "Asia/Tokyo",
  "source": [
    { "id": "official-exams", "title": "Exam timetable", "url": "https://example.test/exams" }
  ],
  "entries": [
    {
      "id": "2026-3Q-example-course-final",
      "kind": "exam",
      "status": "scheduled",
      "quarter": "3Q",
      "courseCode": "EXAMPLE.T001",
      "date": "2026-12-01",
      "startTime": "10:45",
      "endTime": "12:25",
      "room": { "ja": "例示教室", "en": "Example room" },
      "label": { "ja": "例示科目 期末試験", "en": "Example course final exam" },
      "sourceIds": ["official-exams"]
    }
  ]
}
```

### Calendar consistency audit

Before connecting the public feed to scheduling, reconcile these consumers:

1. Today reads local overrides plus subscription days; Schedule currently reads
   local overrides only; Next reads neither for future dates.
2. Weekly courses and recurring personal events currently share the replacement
   weekday, so holidays can hide appointments and replacement days can move them.
3. Intensive `days` is an explicit selected-date list in the editor, Schedule,
   and ICS. Today treats missing dates as default range days and can resurrect a
   removed day. Preserve explicit-list semantics across all consumers.
4. Actual date views need quarter bounds. ICS uses them, but Today and Schedule
   only respect optional meeting `validFrom`/`validTo`.
5. ICS ignores local overrides and meeting validity, and adds replacement events
   without removing the ordinary weekday's course. Boundary selection must also
   filter the academic year and ignore deleted rows.
6. Schedule conflict flags are based on weekly templates, so they miss intensive
   courses/plans and can mark collisions between meetings with disjoint dates.
   Courses' planner template conflicts should remain distinct from dated conflicts.
7. Today and frontend plan recurrence use host-local weekday/date arithmetic in
   places; all actual-date calculations must use the civil date in Japan.
8. Frontend plan recurrence supports fewer RRULE fields than native preview and
   ICS (for example BYMONTHDAY, ordinal BYDAY, and BYSETPOS). Avoid silently
   interpreting unsupported rules as a different schedule.


## Official-source verification (2026-10-03, Japan time)

The prior contract was defined before inspecting current official source layouts.
This check refines it; no official school import or live exam display was enabled.

### Verified entry points and layouts

| Feed | Official landing page | Actual format |
| --- | --- | --- |
| Academic year | https://www.titech.ac.jp/student/students/life/schedules | HTML tables grouped by Quarter; dates, fragmented teaching intervals, examination/makeup intervals, breaks and replacement weekdays |
| Monthly calendar | https://www.titech.ac.jp/student/pdf/life-schedules-month-ay2026.pdf | Four-page visual calendar; color/layout and labels distinguish dates; carries a printed update date |
| Weekday schedule | https://www.titech.ac.jp/student/pdf/life-schedules-week-ay2026.pdf | Two-page PDF with separate once-weekly and twice-weekly meeting sequences; ditto marks and heavy borders encode continuation and exam/makeup periods |
| Undergraduate exams | https://www.titech.ac.jp/student/students/life/undergraduate-exam | HTML discovery/update page; on this check its timetable link was labelled updated 2026-08-03 |
| Undergraduate exam file | https://www.titech.ac.jp/student/pdf/life-undergraduate-exam-timetable.pdf | Sixteen-page 2026 2Q PDF; date, classification, periods, full course code, bilingual title, instructors, room(s), marginal change notes and strikethrough |
| Graduate exams | https://www.titech.ac.jp/student/students/life/graduate-exam | Separate HTML discovery page and PDF; current page labels 2026 2Q, updated 2026-07-21 |
| Graduate exam file | https://www.titech.ac.jp/student/pdf/life-graduate-exam-timetable.pdf | Six-page 2026 2Q PDF with the same broad column structure |
| Teaching clock table | https://www.titech.ac.jp/student/students/life/undergraduate-timetables | HTML mapping teaching-period blocks to allocated clock intervals |

The legacy `titech.ac.jp` pages explicitly state that their published information
remains valid Science Tokyo information. Discover files through the official
landing page: an old search snippet described the undergraduate PDF as 2025,
while the fetched document's own title is 2026 2Q. Search snippets, PDF metadata,
and static filenames are not authoritative for the contained year/quarter.

### Calendar implications

The source's 1Q teaching intervals are 2026-04-07–05-27, 05-30–06-01, and 06-03.
The overall 1Q teaching/exam/makeup range is 04-07–06-09. Treating the whole range
as ordinary weekly classes therefore creates phantom classes in exam periods.
The bundled JSON currently begins 1Q on 04-03 and 3Q on 09-28: these are preparation
periods, while the monthly calendar begins the teaching/exam ranges on 04-07 and
10-01. Define that distinction before changing stored boundary semantics.

Calendar v1 remains a supported compatibility format, but is not a sufficient
complete real-date model. A next version should represent regular teaching ranges,
exam/makeup ranges, and breaks separately within each Quarter. Ranges are arrays,
not one start/end pair. The application must not infer all teaching dates from
Quarter boundaries. Do not add these fields to v1 files; the existing loader is
strict. The weekday PDF includes final sessions inside the exam/makeup period,
so it is a cross-check rather than proof that every listed date is an ordinary
class. Detailed course-level exam/makeup rows take precedence for those sessions.

The HTML also says unlisted public holidays are non-teaching days, and intensive
courses may meet on non-teaching days. Ordinary-course suppression must therefore
preserve manually selected intensive dates and personal plans. National holiday
coverage needs a separate official holiday source, not guesses by the generator.

### Exam implications and checked examples

The PDF's classification column distinguishes final exams from classes (including
makeup classes). Only final-exam rows belong in the exam feed. Preserve full codes
such as `LAL.C202-07`; a stripped base code cannot identify the class correctly.
Keep source `periods`: ranges can span several teaching slots. Allocated period
blocks may be mapped for layout, but must not be presented as verified exact exam
duration. Exact clock fields are omitted unless an explicit source gives them.

On PDF page 6, the old `MTH.B302` entry is visibly struck out: 07-31, periods 3–6,
room M-107, with a note moving it to 08-05. Page 14 has the new entry: 08-05,
periods 3–4, room W9-326. Carry the stable exam ID to the new complete record; do
not retain the old active date, periods or room. A deletion note also appears for
`LAL.S202-07` on page 12. Plain PDF text extraction retains these crossed-out
rows and loses the strike state; visual or equivalent layout-aware review is
required before publishing normalized JSON.

A normal checked record is `CSC.T343` (Databases): 2026-08-06, periods 5–6,
room S2-203, final exam (PDF page 15). This is a source observation, not a claim
that the public schedule is complete or that every exam is publicly listed.
Some rows list multiple rooms; the official exam instructions say individual
room grouping may instead be posted or given by the instructor:
https://www.titech.ac.jp/student/pdf/life-undergraduate-exam-tuigaki-j.pdf
Do not choose one room for a student without published group evidence.

Exam schema refinements based on these sources:

- Required `status`: `scheduled` or `cancelled`. Cancelled records never create
  active exam blocks. A struck-out moved record is not a second scheduled exam.
- Optional `periods`, `notes`, one-based `sourcePages`, and `rooms` preserve official
  allocation, changes, evidence location, and multiple-room cases. Keep the full
  published room string in `room` even when `rooms` is available.
- Source records may retain `landingPageUrl`, its labelled `publishedAt`, actual
  `retrievedAt`, and downloaded-byte `sha256`. These static PDF URLs are updated
  in place; archive the reviewed source snapshot in the data-generation workflow.
- Stable IDs, full codes, chronology, cancellation handling, referenced sources,
  and period order require semantic checks in addition to JSON Schema.
- Rescheduling changes date/time/room together after review. A newer source's
  removed exam must not survive reconciliation just because it is absent from
  the latest normal rows. Publication should be a reviewed complete snapshot.

Example of the checked normal record (clock times intentionally unknown):

```json
{
  "id": "2026-2Q-CSC.T343-final-1",
  "kind": "exam",
  "status": "scheduled",
  "quarter": "2Q",
  "courseCode": "CSC.T343",
  "date": "2026-08-06",
  "periods": [5, 6],
  "room": { "ja": "S2-203" },
  "label": { "ja": "データベース", "en": "Databases" },
  "sourceIds": ["undergraduate-2026-2Q"],
  "sourcePages": [15]
}
```

### Implemented public source integration (2026-10-03)

The default index is https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json. Settings → Subscriptions independently configures and refreshes the calendar by academic year and recommendations by admission year. The index URL, category/year repository path, resolved data URL, enabled refresh state, fetched time and errors are visible. A failed category fetch does not alter other categories. v1 bundled calendars remain compatible; v2 requires four quarter teaching-range arrays and may carry examRanges. Important dates live independently in `important-dates/{year}.json`; a campus event does not by itself suppress teaching. Exams use `exam/{level}/{year}-{quarter-lowercase}.json`. Important-date and exam dataset imports remain pending in the app. GitHub agent automation is deferred.

### Study-guide extraction scope

`recommended` includes source-grounded curriculum maps, standard study plans and course tables; membership alone does not imply an explicit recommendation mark. `mark` is populated only for printed required/recommended markers. `sourceKind` and PDF `sourcePages` preserve where the row came from. Missing groups and credits may be omitted; learning-content A/B letters belong in `content`, not automatically in `group`. In-app catalogue badges distinguish unmarked guide courses from explicit ◎/○ marks.

### Current public layout and missing-data policy

Use flat year filenames (`academic-calendar/2025.json`, `important-dates/2025.json`, `departments/2025.json`) and level-specific exam paths (`exam/undergraduate/2026-2q.json`, `exam/graduate/2026-2q.json`). Index entries point only to verified published files. Unavailable years/quarters do not get empty placeholder files; omission means unavailable, not "there were no exams". A guide that contains courses but has not yet been extracted is unfinished work, not an unavailable dataset.
