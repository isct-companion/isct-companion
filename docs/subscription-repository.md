# Subscription data repository

The future public repository supplies reviewed reference data to Campus Life. `index.json` has two independent maps: `academicYears` selects academic calendars, while `admissionYears` selects cohort-specific undergraduate guides. The app must never use the current academic year as a substitute for a missing admission year.

## Lightweight polling contract

The repository root publishes both `index.sha256` and `index.json`.

- `index.sha256` is plain UTF-8 containing exactly the lowercase SHA-256 of the raw `index.json` bytes, with an optional trailing newline. The client normally downloads only this file.
- When that hash changes, the client downloads `index.json` and verifies its raw bytes against `index.sha256` before parsing it.
- Every dataset reference in `index.json` is an object with `source`, `hash`, and `ts`. `hash` is the lowercase SHA-256 of canonical JSON; `ts` is an RFC 3339 publication timestamp. The client compares the selected dataset hash before downloading its source and verifies the downloaded JSON against the declared hash.
- Exam subscriptions combine the selected exam-file `source + hash` pairs into one deterministic subscription hash. A timestamp change alone does not download unchanged data.

Example:

```json
{
  "academicYears": {
    "2026": {
      "academicCalendar": {
        "source": "academic-calendar/2026.json",
        "hash": "<64 lowercase hex characters>",
        "ts": "2026-04-01T00:00:00Z"
      },
      "importantDates": {
        "source": "important-dates/2026.json",
        "hash": "<64 lowercase hex characters>",
        "ts": "2026-04-01T00:00:00Z"
      },
      "exams": []
    }
  }
}
```

## Yearly update workflow

One GitHub Actions workflow follows the official calendar for each academic year. A separate workflow follows `https://www.titech.ac.jp/guide/guide_<admissionYear>/gakubu1/` and its 17 curriculum PDFs for each admission cohort. The agents read only cited official documents and must not infer missing dates, marks, credits, or course metadata.

Each workflow opens a pull request rather than publishing directly. A human reviewer checks every cited source, calendar exception, department scope, and recommended-course row against the official page/PDF. Merge is the publication gate. The app fetches only merged HTTPS JSON and validates each selected file again. A failure keeps the current data for that axis.

Recommended yearly checks:

1. Confirm the guide root contains the selected `admissionYear`, then confirm all 17 departments, syllabus scopes, unit codes, and guide-PDF URLs use that same guide version.
2. Compare quarter dates and every holiday, no-class day, and replacement day with the official calendar.
3. Compare each guide-table course code, group suffix, mark, credit split, competencies, content, and note.
4. Run schema validation and reject duplicate department IDs, calendar dates, and `(admissionYear, department, code, group)` rows.
5. Review the generated diff and merge only after a second person signs off.
