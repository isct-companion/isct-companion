# Desktop Core checkpoint

Desktop Core consolidation (Phases 0–5) is complete. Implementation checkpoint:
`06e0a36`; desktop closure: `aa9a4a4`. Phase 6 application bindings and native
mobile adoption are deferred. The public contract is recorded in slotledger.
This document keeps final evidence and material observations.

## Completed scope

| Phase | Result |
|---|---|
| 0 | Root Rust workspace, platform boundaries, compact build profiles and validation baseline |
| 1 | Shared domain modules in `app-core`; independent optional `app-ocr` |
| 2 | One Core owner for persistence, references, scheduling, files, notes, backups, cloud, credentials and agent operations; local HTTP MCP |
| 3 | Core-owned school login/session/browser extraction over native ports; correlated callbacks and owned cancellation |
| 4 | Typed domain/runtime failures, no-panic lint boundaries, worker recovery and draining shutdown |
| 5 | Workspace intents and snapshots, schedule/calendar/task/finder/LMS read models, reminders and OCR input preparation; removal of superseded TS engines |
| Closure | Retired standalone school-record/recurrence Tauri commands and scheduler helper removed; README/API references updated |

Tauri supplies platform handles and transport. Frontend state is a disposable
snapshot/read-model cache, editable forms and presentation state. School session
hooks forward lifecycle facts and cache versioned native events. Civil-date UI
helpers navigate/display dates; browser preview seeds are read-only. These do
not form a second domain state machine.

Workspace snapshots include school rows in one read transaction. Mutations
recheck the revision in an IMMEDIATE transaction, validate and commit atomically,
then publish the committed snapshot. Failed writes recover authoritative state
and require explicit retry. Form-based preference intents resolve against the
preceding committed snapshot in the write queue. Async listeners are registered
before rereading state, closing the startup subscription gap.

Shared plan parsing accepts strict ISO civil dates/minute/second values as JST
for legacy compatibility and canonicalizes new writes to RFC 3339 with an
explicit offset. Complete recurrence validation precedes commit. Desktop and
MCP use the same JST occurrence and calendar-date exclusion semantics. Machine
dates do not depend on device locale/calendar; display timestamps use JST.

## Automated acceptance

| Checkpoint | Result |
|---|---|
| Phase 5 full Rust workspace | 330 passed, 0 failed; 4 private OCR fixtures ignored; Core 269 passed |
| Final frontend / type checking | 49 files, 270 tests passed with one worker; `tsc -b` passed |
| Closing native boundary tests | 20 passed, 0 failed |
| Static checks | Workspace all-target Clippy `-D warnings`, formatting and diff checks passed |
| Portable Core | Checks passed for `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android` without desktop features |
| OCR private parity | All 67 expected cells recognized; no wrong accepted cells; private image not committed |

Closing cleanup changed adapters/docs only. Core domain code and generated
contracts did not change, so Phase 5 parity and portable checks remain applicable.
Type generation has one explicit exporter; automatic binding-export test artifacts
were removed.

Coverage includes:

- Frozen TS parity: 136 finder traces plus schedule/calendar/color/ICS outputs
  and edge cases; typed queries and ordered batched results.
- SQLite conflicts/atomic rollback, catalog and LMS projections, durable task
  records, malformed plan dates/RRULE rejection without writes, legacy civil and
  non-JST offset compatibility, and desktop/MCP exclusions.
- Joining identical pulls, manual priority/FIFO, cancellation versus Applying,
  stale/duplicate relays, caller detachment, browser timeouts, worker panic,
  lane release, retry and draining shutdown; partial Kyomu imports remain durable.
- Reference bootstrap with missing quarter rows, source/lifecycle/network changes,
  failed sources, empty feeds and restore; shared freshness/cache behavior.
- Notes/backups/cloud delivery recovery, download waiter release/retry and stored
  formats; these automated checks do not imply every cloud path was exercised live.
- Reminder migration barrier, device-only mode-0600 atomic fired log, delivery
  deduplication/retry and shutdown; OCR bounds, orientation/alpha/resize and cancellation.
- Frontend snapshot revision recovery, stale response isolation, explicit retry,
  pending-submission guards and rapid queued preference edits.

Production Core and OCR deny `unwrap`, `expect`, `panic`, `todo`, `unimplemented`
and `unreachable` without production lint exemptions. The runtime task boundary
returns typed worker failures; it does not recover process/native aborts or undo
already committed work. See slotledger `approach/core-operation-ownership`.

## Live macOS acceptance

The closing signed bundle is
`1.0.0-rc.1.dev.20261014123935+g7fdf1b6.dirty` (179.62 MiB). About matches,
and `codesign --verify --deep --strict` passes. Home retains 14 October,
Teaching day, 2026 3Q, 7 timetable courses and 2 active assignments. Both saved
school accounts settle Signed in, including after normal Quit and cold relaunch
of the exact bundle. The application is left on English Home with editing off.

Across the completed desktop checkpoints:

- Home/Week/Lectures and finder load; finder shows 63 syllabus/20 results and
  preserves conditions, details, replacement/undo and rapid time-filter input.
- Subscription Check now completes with quarter data retained. Full LMS finishes
  with 7 matched courses and no differences; Kyomu refresh updates its timestamp.
- Cached 24-page PDF renders; ZIP material saves and Show in Finder selects it.
- English/Chinese settings retain valid dates; the existing English Home date
  caption is unchanged. Appearance Automatic → Dark → Light → Automatic succeeds
  and restores the original preference. Diagnostics/academic summary dates render.
- Plan save succeeds. The same plan appears in MCP `today` and `schedule` with
  matching stored `+09:00` time. Only the owned temporary plan is cleaned through
  Core/MCP using a recoverable tombstone, without changing permissions. Final
  PlanEditor time/occurrence preview succeeds and Cancel writes nothing.
- MCP initialization, 17 tool names and actual today/grades/GPA reads pass before
  and after restart; wrong token returns 401, hostile Origin 403. Token remains
  unchanged and mode 0600. Credentials/individual records are not printed.

Material observations retained from intermediate validation:

- Startup listener registration and failed-editor dismissal caused a plan conflict;
  snapshot/listener recovery and pending-submission handling fixed them.
- Civil-time plans displayed in desktop but were omitted by MCP; shared parsing,
  canonical writes and exclusion logic fixed this and passed live parity.
- One earlier full LMS run with a visible-Web interruption timed out; uninterrupted
  and repeated interrupted retries completed. That timeout's cause is unconfirmed.
- One two-worker frontend run while the app was open timed out on its first
  navigation test and cascaded overlapping React `act` failures. The independent
  single-worker rerun after Quit passed without raising timeout thresholds.
  Resource contention is suspected, not established.
- One signing attempt hit an unavailable Apple timestamp service; same-identity
  retry succeeded, and the closing normal Tauri signing run succeeded.

Live coverage excludes cloud mutations, the PDF viewer's separate save toolbar,
the graphical Plan Delete click (computer-use action-time confirmation prevented
it), and foreign/mobile bindings. Automated delete coverage and Core/MCP temporary
plan cleanup passed. The checkpoint is complete within this documented coverage.

## Code size and retained outputs

Count physical `.rs/.ts/.tsx` production separately from dedicated/inline Rust
tests and generated contracts; exclude documents and fixture JSON.

| Scope | Production | Tests | Generated |
|---|---:|---:|---:|
| Phase 5 affected files | 20,301 → 24,227 (+3,926) | 7,724 → 9,639 (+1,915) | 705 → 1,064 (+359) |
| Closing cleanup affected files | 3,820 → 3,748 (-72) | 360 → 350 (-10) | Unchanged |

Phase 5 TypeScript production falls 14,607 → 12,135 (-2,472); Rust rises
5,694 → 12,092 (+6,398). Typed read models, transactions and failure handling
increase overall production size. The total reduction target is not achieved;
redundant state owners are removed. Closure deletes an unused test stub, not assertions.

Generated target/frontend/TypeScript caches are removed after checks. Only the
signed app and provisioning profile remain in target (179.66 MiB allocated),
with retained strict signature verification. Original roadmap/mobile edits and
named OAuth/upstream stashes remain outside the checkpoint commits.
