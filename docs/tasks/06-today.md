# Task 6 — Today (with tasks)

2026-10-05: Today now opens the inline LMS workspace with Classes/Assignments navigation instead of a course drawer or app-wide Web navigation. See `../lms-workspace-inspection.md` for current interaction, retention limits and remaining file/offline work. The older Build list below is the original task specification.

Product spec: `docs/product-definition.md` → Navigation, Today. Depends on tasks 1, 3, 4, 5.

## Build

1. **Navigation:** remove Tasks from the rail and from `⌘`-shortcuts (Today ⌘1, Schedule ⌘2, Courses ⌘3, Web ⌘4). `src/pages/tasks/TasksPage.tsx` content moves into Today; delete the page and its route.
2. **Teaching state** at the top from `academic_calendar` + `calendar_exceptions`: ordinary day, holiday, no-class, or replacement day ("Monday classes").
3. **Today's classes:** from `timetable_courses` whose `schedule` has a weekly entry for today's (replacement) weekday within the quarter dates, plus `intensive_schedules` days that fall today, plus today's `plans`. Order now / next / later; time and room dominant; room via `pickLocalized`.
4. **Assignments:** from `assignments` (task 3) — overdue, due today, due soon (7 days). State chip shows LMS state only. `Hide` writes `assignment_marks.hidden = true` (no school action); hidden items disappear from Today. Opening an assignment switches to Web at its URL and, on return, runs a bounded refresh (`Checking…` → state + `Last checked`).
5. **Academic summary:** one compact row: GPA, GPT, earned credits (from `gpa_summary`, task 4), with the fetched date. Tapping it opens 教務Web's GPA page in Web. Hidden when no data.
6. Empty states: no account → short `Connect account` action; no classes → nothing but the date line.

## Acceptance

- Replacement day shows the replaced weekday's classes; holiday shows none.
- An intensive course appears only on its set dates.
- Hide removes the assignment from Today and it appears in Settings → Hidden assignments (task 10).
- No `Mark complete` anywhere.
- Page fits the window without scrolling at 1280×820 with 6 classes and 8 assignments (lists truncate with "+N more").
