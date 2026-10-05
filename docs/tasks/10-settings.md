# Task 10 — Settings: department, subscriptions, hidden items, scheduled tasks, data

Product spec: `docs/product-definition.md` → Settings, Advanced. Depends on tasks 1, 5, 6. Keep the existing Mantine layout and the "simple, no page scroll" rule.

## Sections to add or change

- **General → Department:** select one of the departments from the subscription (task 5), shown in the app language. Used by the course planner filter and recommendations.
- **Subscriptions:** see task 5.
- **Hidden items:** list of hidden assignments (title, course, due date) with `Restore` per item and `Clear all`; clearing removes the `assignment_marks` rows.
- **Scheduled tasks:** generalize today's `customization.hooks` (`action`, `schedule: manual | launch | hourly`) into `scheduled_tasks`:
  - actions: built-in `school-refresh`, `export` (cloud backup), and user hooks;
  - schedule: `launch` or an RRULE (minimum one hour for tasks; presets: hourly, daily at …, weekly);
  - list with enable toggle, last run and result, `Run now`.
  - Runs only while the app (or its tray process, when enabled) is running; no background server.
- **Storage & sync:** `Export…` / `Import…` of the task-1 document (file), and the existing cloud providers pointing at the same export/import.
- **Advanced:** app CSS, per-site CSS/JS, hooks editors stay; make sure per-site CSS/JS still apply to the school webview after tasks 3/4 add extractors (extractors and user scripts are independent).
- **Agent access:** toggle for the MCP server (task 11) with the command line to add it to an agent.

## Acceptance

- Choosing a department changes the planner's filter immediately.
- Restoring a hidden assignment shows it again on Today.
- A daily scheduled export runs at the set time while the app is open and records its result.
