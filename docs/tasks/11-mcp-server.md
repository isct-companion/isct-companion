# Task 11 — Agent access (local MCP server)

Product spec: `docs/product-definition.md` → Agent access. Depends on tasks 1–5, 9.

## Goal

Let AI agents on the same machine use the person's app data through the Model Context Protocol, safely.

## Build

- A separate binary in the Rust workspace (`backend/src/bin/isct-companion-mcp.rs`), stdio transport, sharing the data module from task 1. It opens `school.db` directly (WAL allows the app to keep running).
- Off by default; enabled in Settings → Agent access (task 10), which shows the command to register it with an agent. When disabled, the binary refuses to start.
- **Read tools** (return JSON with both languages where stored):
  - `today()` — teaching state, classes, assignments (hidden excluded), academic summary
  - `schedule(from, to)` — classes, intensive days, plans
  - `courses(year, quarter)` — timetable courses with membership state and credits
  - `course(id)` — offering metadata, syllabus sections, LMS items, assignments
  - `search_catalog(query | slot filters, department?)`
  - `registration(year, term)`, `grades()`, `gpa()`, `requirements()`
  - `recommended(departmentId?)`
- **Write tools** (person's own data only; go through the same validation and versioned update as the UI; the app picks up changes on next focus):
  - `plan_add(offeringId, term)`, `plan_remove(...)`
  - `plan_event_create/update/delete` (task 9 plans, RRULE rules enforced)
  - `hide_assignment(id)`, `restore_assignment(id)`
- **Never exposed:** anything that contacts a school system, credentials, cookies, session data, raw HTML, settings for accounts or cloud.
- Every write tool records `updated_by = "mcp"`; Settings shows the last agent writes.

## Acceptance

- Protocol tests over stdio for each tool; schema of each tool documented in `docs/agent-access.md`.
- A write tool cannot touch school tables (test attempts fail).
- Disabled in Settings → the server exits with a clear message.
