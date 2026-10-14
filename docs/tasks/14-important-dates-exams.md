# Task 14 — Subscriptions: important dates and examinations

Depends on task 5. Follow the rules in [README](README.md). Dataset layout and semantics: [public-data-agent-prompt.md](public-data-agent-prompt.md), [subscription-repository.md](../subscription-repository.md).

`isct-companion/Isct-Data` publishes `important-dates/{year}.json` and `exam/{undergraduate|graduate}/{year}-{quarter}.json`.

## Build

- Two new subscription kinds, `important-dates` and `exams`, refreshed through Core's `ReferenceService` like `calendar` and `departments` (index lookup, schema validation, versioned writes, failure keeps data). Desktop calls `refresh_reference_subscriptions`.
- Poll the GitHub repository through the built-in `Reference subscriptions` Scheduled Task. Its hourly wake-up applies a 24-hour gate before requesting only `index.json`, which also lets an offline or metered pause retry after conditions improve. Hash the index bytes locally; if unchanged, do not download datasets. Otherwise use each selected source's declared canonical JSON `hash` and RFC 3339 `ts` to avoid downloading unchanged datasets. There is no webhook or reverse push from GitHub. Automatic polling prefers Wi-Fi, pauses on cellular/metered links, and can be explicitly enabled for cellular data in Settings → Data updates.
- `important-dates`: store events (title ja/en, date or range, category). Show them in the month/week calendar views and on Today as all-day items. An important date never cancels classes; only calendar exceptions do.
- `exams`: fetch every exam file listed for the selected academic year. Do not add a profile-level field: cross-level enrollment is valid, and the repository's exact course codes are sufficient. Match rows to non-manual registered courses by course code. Store date, periods or exact start/end (only when published), room. Show on the course detail page and on Today / the week view on the exam day. Unmatched rows are not shown.
- Use one user-owned `ScheduleOverride` model for both exam edits and future one-off course edits. A matching-course exam replaces that dated course occurrence; an overlap with another course or event stays visible with a warning.
- Course, event and exam cards occupy the full timetable column. Exams inherit the course color and add a compact exam state band.
- Missing files for a year or quarter are a normal state ("Not published"), not an error.

Automatic checks run while the app is open; every subscription kind is also available for an explicit forced refresh from Settings.

## Acceptance

- Refreshing exams with a registered course present shows its exam on that course and on the exam day.
- An important date on a teaching day leaves the timetable unchanged.
- Rust tests with synthetic fixtures for both schemas, including a missing quarter file.
