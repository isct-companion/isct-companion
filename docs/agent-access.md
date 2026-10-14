# Agent access

IsctCompanion includes a local Streamable HTTP MCP server. Enable Developer
options in Settings, then open AI agent access and enable Allow local agents.
Register the URL shown there as an HTTP MCP server:

```text
http://127.0.0.1:47832/mcp
```

Use **Copy token** to configure Bearer authentication in your MCP client. Clients
that accept HTTP headers should send `Authorization: Bearer <access token>`.
The token stays on this device, outside exported data, and persists across app
restarts. Keep it private. Keep IsctCompanion open while using the server.

Calls use the desktop's existing Core service, with permission checked on every
call. Turning Allow local agents off blocks subsequent tool calls. There is no
CLI command, stdio adapter, bridge file or direct database mode to configure.
The server binds only to loopback; authentication and Host/Origin checks protect
the endpoint. If the port is unavailable, Settings shows the startup error.
See [shared Core integration](core-api.md).

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
  `YYYY-nQ` and must be one of the offering's quarters. `plan_add` plans an
  offering running over several quarters (e.g. 3Q–4Q) into each of them and
  returns those `terms`; `plan_remove` removes one quarter's plan. A course
  already in the workspace is reused, never replaced, whichever quarter it was
  added in.
- `plan_event_create(title, start, duration, rrule?, room?)`
- `plan_event_update(id, …)` / `plan_event_delete(id)`: deleted plans can
  be neither changed nor deleted again
- `hide_assignment(id)` / `restore_assignment(id)`

Writes are limited to personal tables and pass through the same validation and
versioned store as the application. They return the ids they touched, are
marked `source.provider: "mcp"` where the record has a source, and appear under
Settings → AI agent access as recent writes.

Writes and their audit entry commit in one transaction, with permission checked
inside that transaction. The server returns the local result while cloud
capture remains owned background work. If the connection breaks before a reply,
read the current state before retrying a create operation.
