# Task 8 — Course planner

Product spec: `docs/product-definition.md` → Course planner. Depends on tasks 1, 2 (including department sync), 4, 5.

The main course-editing surface; the aim is the fewest meaningful steps on one page, no dialogs.

## Layout (one page, fits the window)

- **Header:** Quarter switcher, department filter checkbox (department from Settings; unchecked = all departments + liberal-arts units), keyword search field, sync status of the department catalog.
- **Left — draft timetable** of the Quarter: Mon–Fri × periods 1–10 plus an "Intensive / other" row. Cells show registered, declared (from 教務Web) and planned courses; planned are lower contrast. Conflicts: a slot with more than one course is marked.
- **Right — candidates** for the selected slot (or keyword results): offerings whose `schedule` has a weekly entry covering that day and period in that Quarter, from `catalog_search`. Each row: title, code, credits, format, language, room, and badges:
  - recommendation from `recommended_courses` of the person's department: group + mark (`A ○`, `R ◎`);
  - `Registered` / `Declared` / `Planned` if already in the timetable;
  - `Conflict` if it collides with another chosen course.
  Sort: recommended first, then level (番台), then code.
- **Below the candidates — outline** of the selected offering: structured fields only (format, language, credits split, schedule and rooms, class, updated, competencies, lecture plan) and the evaluation section text as written; `Open syllabus` for the full view. No values are parsed out of free text.

## Interaction

- Click a slot → candidates update; click a candidate → outline; `Add` / `Remove` act immediately (writes `plan_items`), with undo in a toast.
- A course with several weekly slots is added to all of them at once; the timetable highlights every slot of the hovered candidate.
- Keyboard: arrows move between slots, Enter adds the focused candidate.
- Registration stays in 教務Web: a `Open 教務Web` action goes to Web at the registration page.

## Acceptance

- Selecting Tue 3–4 in 3Q with 情報工学系 lists only offerings meeting then, recommended ones first with their group/mark.
- Unchecking the department filter adds other departments' and liberal-arts offerings.
- Adding a two-slot course fills both slots and flags conflicts.
- Works with a partially synced catalog (shows progress, lists what is stored).
- Page fits 1280×820 without page scroll (lists scroll inside their panels).
