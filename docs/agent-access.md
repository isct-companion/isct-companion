# Agent access

IsctCompanion exposes an opt-in local MCP server over stdio. Enable it in
Settings → Developer options (switch on), then Settings → AI agent access, and register the binary with the database path shown
by the installed application:

```sh
isct-companion-mcp --db /absolute/path/to/school.db
```

The server refuses to start while Agent access is disabled.

## Read tools

- `today()`: today in Japan time: the school-calendar state (holiday,
  no-class, replacement day), classes actually held, personal plans, open
  assignments not yet due, and open assignments overdue in the last 14 days
- `schedule(from, to)`: registered or planned courses of the quarters in the
  range, intensive dates, and personal plans with their occurrences
- `courses(year, quarter)`: registered or planned courses of one quarter
- `course(id)`
- `search_catalog(query)`: each row carries `plannable`; only plannable rows
  have the schedule `plan_add` needs. Open the others once in the app first,
  since this server never contacts the syllabus site.
- `registration(year?, term?)`
- `grades()`
- `gpa()`
- `requirements()`
- `recommended(departmentId?)`: the person's admission year; the department
  defaults to the one chosen in Settings

Results leave out sync bookkeeping (`rev`, `updatedAt`, `sourceKey`, …).

School records are read from the local database. The server never contacts a
school host and never exposes credentials, cookies, sessions, raw HTML or
account/cloud settings.

## Write tools

- `plan_add(offeringId, term)` / `plan_remove(offeringId, term)`: `term` is
  `YYYY-nQ` and must be one of the offering's quarters. A course already on
  the timetable is reused, never replaced; one that sits in another quarter is
  refused, because the timetable lists a course only under its own quarter.
- `plan_event_create(title, start, duration, rrule?, room?)`
- `plan_event_update(id, …)` / `plan_event_delete(id)`: deleted plans can
  be neither changed nor deleted again
- `hide_assignment(id)` / `restore_assignment(id)`

Writes are limited to personal tables and pass through the same validation and
versioned store as the application. They return the ids they touched, are
marked `source.provider: "mcp"` where the record has a source, and appear under
Settings → AI agent access as recent writes.
