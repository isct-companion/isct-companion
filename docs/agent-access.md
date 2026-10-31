# Agent access

IsctCompanion includes a local Streamable HTTP MCP server. Open Settings →
AI agent access and enable Allow local agents. Under Connect an agent, pick
Claude Code, Codex, Claude Desktop or Cursor and copy the ready-made setup
(the token is masked on screen and copied whole). Any other client registers
the URL shown there as an HTTP MCP server:

```text
http://127.0.0.1:47832/mcp
```

Use **Copy token** to configure Bearer authentication in your MCP client. Clients
that accept HTTP headers should send `Authorization: Bearer <access token>`.
The token stays on this device, outside exported data, and persists across app
restarts. Keep it private. Keep IsctCompanion open while using the server.

Calls use the desktop's existing Core service, with permission checked on every
call. The server listens only while Allow local agents is on: the token is
created the first time it is turned on, and turning it off stops the listener
(within a few seconds, or immediately from Settings). There is no CLI command,
stdio adapter, bridge file or direct database mode to configure. The server
binds only to loopback; authentication and Host/Origin checks protect the
endpoint. If the port is unavailable, Settings shows the error and the app
retries every few seconds, so the server starts once the port frees.
Local storage failures reach clients as a generic error; details stay in the
app's log.
The server contract is recorded in slotledger as `approach/mcp-local-server`.

## Tools

The server's `tools/list` is the reference. Every tool comes with its
description, an input schema that refuses unknown arguments, an output schema
and MCP hints, all from one registry in Core (`crates/app-core/src/agent`).
Every answer is a JSON object, returned as text and as structured content. A
failure is an error result whose text is `{"code", "message"}`; `code` is one
of Core's stable error codes, such as `agent-disabled` or `invalid-input`.

Read tools carry `readOnlyHint` and answer from the local database: today's
classes, events and open assignments (`today`), the courses and dates of a
range or quarter (`schedule`, `courses`), one course with its LMS items and
assignments (`course`), the local course catalog (`search_catalog`), and the
imported registration, grade, GPA, requirement and recommended-course records.
They name the fields they return, so sync bookkeeping never reaches a client,
and school HTML (assignment instructions, LMS pages) arrives as plain text.
The server never exposes credentials, cookies, sessions or account and cloud
settings.

Assignment tools work on the same workspace folders as the app, under the
course storage root chosen in Settings: `assignment(assignmentId)` reads one
assignment with its description, attachments and workspace;
`workspace_create(assignmentId)` has Core download the attachments (through
the app's school session), write `ASSIGNMENT.md` and an empty `submit/`, and
returns the absolute path, or the existing workspace as it is; and
`workspace_status(assignmentId)` lists what `submit/` holds with each file's
size and SHA-256. The agent writes the files to hand in into `submit/` itself.

Write tools change only the person's own data on this device: the plan
(`plan_add`, `plan_remove`), weekly personal events (`weekly_event_create`,
`weekly_event_update`, `weekly_event_delete`) and locally hidden assignments
(`hide_assignment`, `restore_assignment`), plus `workspace_create` and
`submission_prepare`. Tools that remove or overwrite carry
`destructiveHint`; those safe to repeat carry `idempotentHint`.

Writes pass through the same validation and versioned store as the
application. Each write and its audit entry commit in one transaction, with
access checked again inside it; records with a source are marked
`source.provider: "mcp"`, and Settings → AI agent access lists recent writes.
The server returns the local result while cloud capture stays owned background
work. If the connection breaks before a reply, read the current state before
retrying a create.

## Handing work in to the LMS

Writing to the LMS needs a second switch under Allow local agents: **Allow
AI agents to write to LMS**, off by default and kept on this device only.
Core reads it on every LMS write call and again before each request that
changes the LMS, so turning it off stops a write that has not sent its save.
IsctCompanion never asks for confirmation itself
(`approach/mcp-submit-confirmation`): the MCP client does. The two LMS write
tools carry `destructiveHint`, `openWorldHint` and
`_meta["anthropic/requiresUserInteraction"] = true`, which Claude Code honours
even in bypass mode; Codex, Cursor and other clients follow their own
approval settings.

At this school every assignment has drafts off, so saving is submitting.
A write takes two steps:

1. `submission_prepare(assignmentId, action)` with `save` or `remove`.
   Nothing is sent. For a save, Core copies the files now in `submit/` into
   its own data folder by name, size and SHA-256 and reads the submission on
   the LMS. The answer is a `preparationId`, an `expiresAt` and a `summary`:
   the assignment title, the files with their size and SHA-256 prefix, what
   is added, replaced or removed on the LMS, and whether the person can still
   change or withdraw it afterwards (until the cut-off, while the assignment
   stays open, or never for a removal). A save that would replace a
   submission already on the LMS is refused with what it would replace. The
   same content against the same LMS state gives the same preparation.
2. `submission_save(preparationId, summary)` or
   `submission_remove(preparationId, summary)`, with the summary passed back
   unchanged. The client shows these arguments when it asks the person, and
   Core refuses any difference from the preparation (`submission-confirmation-mismatch`).

What Core guarantees, whatever the agent does:

- Only what was prepared is sent: the snapshot, never a later `submit/`.
  Files changed since the preparation (`submission-files-changed`), another
  signed-in account (`submission-account-changed`) or a submission changed on
  the LMS in the meantime (`submission-remote-conflict`) are refused before
  anything is sent.
- One write per account and assignment at a time. While one is running or
  its outcome is unknown, other writes are refused (`submission-in-progress`,
  `submission-outcome-unknown`).
- Idempotency is Core's: it derives the write's identity from the account,
  assignment, attempt, action and snapshot, so the same call again answers
  with the same operation instead of writing twice.
- Every write is read back from the LMS. `succeeded` means the LMS holds
  exactly the snapshot's names and sizes; anything else is `failed`, with a
  code such as `submission-rejected` or `submission-mismatch`.
- A write whose answer was lost is `outcome_unknown`. Core settles it by
  reading the LMS, right away, on the next start and in its periodic wake,
  and never sends it again. It turns `failed` only after the LMS has had
  three minutes to finish; executing again is then a new write, which the
  client confirms again.
- A write that has been sent is never reported undone by a local cancel.

`submission_status(operationId)` answers with a write's state, reading the
LMS first when its outcome is unknown. Only individual file assignments are
supported; online text, drafts, submission statements, group work, timed
assignments, ZIP hand-ins and file types named by group are refused with
`submission-unsupported` and the reason. Operation records and snapshots stay
on this device, outside backups and cloud sync, and are cleaned after success
or expiry. The contract is recorded in slotledger as
`approach/lms-submission-contract`.
