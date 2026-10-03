# Task 7 — Courses: list, detail, intensive dates, ICS export

Product spec: `docs/product-definition.md` → Courses. Depends on tasks 1–4.

## Build

1. **Quarter list:** current Quarter by default; switch to prior Quarters and add future ones. Each row: title (`pickLocalized`), code, schedule summary, credits `2-1-0 (3)`, state chip: `Planned` / `Declared` / `Registered` / `Completed` (from `memberships`, task 4). Footer: total credits of registered + declared, and of planned, for the Quarter.
2. **Course detail** tabs:
   - Overview: metadata fields from the offering (format, language, credits, schedule and rooms, class, academic unit, updated) — structured fields only.
   - Materials: `lms_items` grouped by LMS section; opening one goes to Web at its URL. Folders list files on demand (task 3 HTML).
   - Assignments: this course's assignments (same chips and Hide as Today).
   - Syllabus: every section of `offering_content` in the app language (fallback to the other), rendered as stored — paragraphs, lists, competencies, lecture plan table. A link opens the official page.
3. **Intensive dates** for offerings whose schedule has `intensive` (or any course, manually): an editor with
   - quick set: date range + periods (e.g. 7/1–7/8, 1–6) in one step;
   - per-day list below: change periods or remove a day;
   - saved to `intensive_schedules`; shown in Schedule and Today.
4. **ICS export** (`Export ICS` per Quarter): one `.ics` with
   - weekly courses: `RRULE:FREQ=WEEKLY;BYDAY=…;UNTIL=<quarter end>` from quarter dates (task 5), `EXDATE` for holidays/no-class days, and one-off events for replacement days;
   - intensive courses: one event per set day;
   - plans (task 9) with their RRULE.
   Times in `Asia/Tokyo` with a `VTIMEZONE`; stable `UID`s (course id + date/rule) so re-import updates instead of duplicating. Save through the native save dialog.

## Acceptance

- ICS validates (round-trip parse test) and reproduces a holiday as an EXDATE.
- Intensive quick set creates the expected days; removing one day removes it from Schedule.
- Detail tabs work with Japanese-only and English-only offerings (fallback).
- Credits footer sums halves correctly (`0.5`).
