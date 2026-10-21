# Task 7 — Courses: list, detail, intensive dates, ICS export

Product spec: `docs/product-definition.md` → Courses. Depends on tasks 1–4.

## Build

1. **Quarter list:** current Quarter by default; switch to prior Quarters and add future ones. Each row: title (`pickLocalized`), code, schedule summary, credits `2-1-0 (3)`, state chip: `Planned` / `Declared` / `Registered` / `Completed` (from `memberships`, task 4). Footer: total credits of registered + declared, and of planned, for the Quarter.
2. **Course detail** tabs:
   - Overview: metadata fields from the offering (format, language, credits, schedule and rooms, class, academic unit, updated) — structured fields only.
   - Materials: `lms_items` grouped by LMS section; opening one goes to Web at its URL. Folders list files on demand (task 3 HTML).
   - Assignments: this course's assignments (same chips and Hide as Today).
   - Syllabus: the school's シラバス page for the offering, inside the app (Core builds the URL from the offering id and content language), with a link that opens it in the browser.
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


## Intensive editor review (2026-10-03)

Keep the existing batch range → explicit selected dates flow. The current editor
needs these targeted UX/validation follow-ups:

- Show the course title, actual clock range, and selected-day count.
- Allow editing each selected day's periods, as originally specified; the current
  list only offers Remove. Keep explicit removals removed in Today as well.
- Add room input and preserve the existing room when saving; the current editor
  does not send its optional room field back.
- Reject empty, fractional, reversed, or out-of-range period values. Explain that
  clock conversion uses the existing two-period teaching slots, or select those
  slots directly so `[2, 3]` cannot silently fall back to a whole teaching day.
- Reconcile date-range edits with the selected-date list before saving. Currently
  changing From/To after batch generation can save stale out-of-range dates.
- Explain that Set every day in range replaces the list (including prior removals
  and per-day changes). It includes weekends intentionally; do not infer which
  days an intensive course meets.
- Keep a bounded, scrollable day list and visible Cancel/Save actions for long
  ranges. Validate ranges and periods at Save, not only at batch generation.
- Provide a way to clear an existing intensive schedule. Currently removing all
  selected dates disables Save and cannot clear the saved schedule.

### 2026-10-03 统一实际日期计算

Today、Next、Schedule 与 ICS 使用 `src/core/schedule.ts` 的 `occurrencesOnDate`。周课程同时遵守季度边界、v2 校历的分段 teachingRanges、停课/调课与单场有效日期；集中讲义仅用显式选中日期，允许校历停课日上课。个人事件和 Plan 按自然日期计算，RRULE 的预览、展示、导出共用 JS rrule 引擎。Courses 的未来季度课表保留周模板。

ICS 默认仅课程，通过下拉提供课程 + 个人 Plan。导出展开为实际日期事件，支持跨午夜、文本转义、UTF-8 行折叠，移除了旧 Rust 的重复课程展开逻辑。相关日期、冲突、递归和导出测试位于 `src/core/schedule.test.ts`。

### 2026-10-05 Unified Schedule workspace

Schedule and Courses now share one navigation entry, Schedule. With Edit off, the selected quarter shows an actual dated week, including calendar exceptions, intensive occurrences and personal plans. With Edit on, it shows the quarter's weekly planning pattern and syllabus finder; local mutation controls require this mode. Switching modes preserves the selected quarter and viewed week. Course details remain available while viewing; personal items open a preview before enabling editing.

ICS uses the same joined split-button treatment as Pull from LMS, with courses-only as the primary action and courses plus personal plans in the menu. Academic summary sits beside Export ICS in the top toolbar and opens an independent panel; the toolbar displays `This term {credits} credits | Details`. Grade values are rendered only while the person has opened Details; there is no persisted grade visibility switch or GPA in the toolbar.
