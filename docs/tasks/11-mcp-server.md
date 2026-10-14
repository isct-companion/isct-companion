# Task 11 — Agent access (local MCP server)

Product spec: `docs/product-definition.md` → Agent access. Depends on tasks 1–5, 9.

## Goal

Let AI agents on the same machine use the person's app data through the Model Context Protocol, safely.

## Build

- A standard Streamable HTTP MCP server embedded in desktop at `http://127.0.0.1:47832/mcp`, using the official Rust SDK and the existing Core service. No CLI or headless database mode.
- Off by default; enabled in Settings → AI agent access (a Developer options page, task 10), which provides the URL and Bearer token. Permission is checked on every call.
- **Read tools** (return JSON with both languages where stored):
  - `today()` — teaching state, classes, assignments (hidden excluded), academic summary
  - `schedule(from, to)` — classes, intensive days, plans
  - `courses(year, quarter)` — timetable courses with membership state and credits
  - `course(id)` — offering metadata, syllabus sections, LMS items, assignments
  - `search_catalog(query | slot filters, department?)`
  - `registration(year, term)`, `grades()`, `gpa()`, `requirements()`
  - `recommended(departmentId?)`
- **Write tools** (person's own data only; go through the same validation and versioned update as the UI; the app receives a workspace change event):
  - `plan_add(offeringId, term)`, `plan_remove(...)`
  - `plan_event_create/update/delete` (task 9 plans, RRULE rules enforced)
  - `hide_assignment(id)`, `restore_assignment(id)`
- **Never exposed:** anything that contacts a school system, credentials, cookies, session data, raw HTML, settings for accounts or cloud.
- Every write tool records `updated_by = "mcp"`; Settings shows the last agent writes.

## Acceptance

- Real HTTP protocol tests cover the tool manifest, calls, authentication, revocation and shutdown; tool schemas are documented in `docs/agent-access.md`.
- A write tool cannot touch school tables (test attempts fail).
- Disabled in Settings → tool calls are denied, including already-connected clients.
