# Subscription data repository

The future public repository supplies reviewed reference data to IsctCompanion. `index.json` has two independent maps: `academicYears` selects academic calendars, while `admissionYears` selects cohort-specific undergraduate guides. The app must never use the current academic year as a substitute for a missing admission year.

## Lightweight polling contract

The repository has one stable client entry point: `index.json`.

- The client downloads only `index.json` to discover repository metadata. It hashes the raw bytes locally and skips dataset requests when that hash is unchanged.
- Do not configure or derive a separate hash URL. Dataset locations, hashes and publication timestamps come only from the validated index.
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

## Exam acquisition boundary

The data-source agent owns exam discovery, document interpretation, evidence
review and consolidation. This includes official exam tables and course-specific
notices. Core consumes the resulting exam feed, associates records with courses,
and applies personal edits before building calendar and notification views.
Natural-language exam extraction belongs in the data repository.

The source-side processing contract is:

1. Inventory sources by academic year, quarter and coverage. Record whether each
   source is public or requires an authenticated session. An exam-week PDF covers
   its published rows; a missing course remains a coverage gap until its other
   sources have been checked.
2. Extract candidates with the exact course code and class suffix, offering ID
   where verified, assessment type, dates, times, room and source evidence. Keep
   quizzes, optional oral tests and final exams distinct. Resolve omitted years
   against the source's verified academic context. Derive an end time only when
   the source states both a start time and duration, and record that derivation.
3. Consolidate the same assessment across sources before publishing. Preserve
   multiple assessments for one course. Select corrections using their explicit
   scope and revision evidence; unresolved conflicts remain review candidates.
4. Preserve an assessment's ID when its date, time or room changes. Match updates
   to the existing source-side identity registry before assigning a new ID.
   App personal edits are keyed by that ID, so changing it creates a different
   exam and disconnects those edits.
5. Publish confirmed records to the existing level/quarter feed. Keep extraction
   evidence, unresolved candidates and coverage reports in source-side review
   artifacts. The v1 client rejects unknown fields and requires either allocated
   periods or a complete start/end time pair.
6. Validate every selected feed and its index hash, then review the resulting
   diff. Retain cancellation evidence explicitly. A failed or partial acquisition
   must not remove previously published exams as though they had disappeared.

Authenticated sources require an authorized session in the agent's execution
environment. Publish only the necessary exam facts and source references; keep
session credentials and unrelated course content out of subscription artifacts.
Record the access requirement in the source inventory so unattended runs can
report an unavailable source rather than silently claiming complete coverage.

This contract defines the processing boundary. The data repository tracks which
collectors and agent execution steps are implemented.
