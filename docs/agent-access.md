# Agent access

IsctCompanion exposes an opt-in local MCP server over stdio. Enable it in
Settings → Agent access, then register the binary with the database path shown
by the installed application:

```sh
isct-companion-mcp --db /absolute/path/to/school.db
```

The server refuses to start while Agent access is disabled.

## Read tools

- `today()`
- `schedule(from, to)`
- `courses(year, quarter)`
- `course(id)`
- `search_catalog(query)`
- `registration(year?, term?)`
- `grades()`
- `gpa()`
- `requirements()`
- `recommended(departmentId?)`

School records are read from the local database. The server never contacts a
school host and never exposes credentials, cookies, sessions, raw HTML or
account/cloud settings.

## Write tools

- `plan_add(offeringId, term)` / `plan_remove(offeringId, term)`
- `plan_event_create(title, start, duration, rrule?, room?)`
- `plan_event_update(id, …)` / `plan_event_delete(id)`
- `hide_assignment(id)` / `restore_assignment(id)`

Writes are limited to personal tables, pass through the same validation and
versioned store as the application, and record `updatedBy: "mcp"`.
