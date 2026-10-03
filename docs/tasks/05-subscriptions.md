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

- Settings → Subscriptions: a default subscription URL (empty until the repository exists; bundled files are used), custom URL, enable/disable, last update, errors.
- Fetch `index.json`, select the calendar from `academicYears[academicYear]`, and independently select the guide from `admissionYears[academicProfile.admissionYear]`; `GET` over HTTPS only, size-bounded, no cookies. Validate against the schemas (embed them); reject the whole selected file on any error and keep its current data.
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
