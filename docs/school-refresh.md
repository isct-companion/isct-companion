# Pull coordination

Reference subscriptions, LMS and 教務Web share `crates/isct-core/src/pull_coordinator.rs`.
Manual commands, launch/scheduled refreshes, account activation, foreground
events and lecture detail requests all enter its native admission path.
The frontend reports facts and renders snapshots; its busy flags and workspace
Promise queue no longer decide which requests execute.

## Refresh policy

`crates/isct-core/src/school_refresh.rs` owns freshness and lecture debounce
intents; Core's `scheduler.rs` feeds the same policy. Neither owns an execution
queue. `App.tsx` reports foreground/network and lecture
navigation events without a clock or a session-local detail cache. Native
session completion also reports directly, including before the UI subscribes.

- LMS source TTL: 12 hours. 教務Web TTL: 7 days, with every required page
  included in the freshness check (`contracts.rs` is the shared policy).
- Lecture details wait 2 seconds before becoming eligible, and wait until
  the visible school view closes. Repeated opens within 60 seconds coalesce;
  a failed fill can be requested again on reentry.
- Full pulls have an automatic retry cooldown, separate from data TTL. It
  starts at five minutes and triples after each pull that leaves the profile
  due (5 min, 15 min, 45 min, ~2 h, then at most 6 h); observing the profile
  fresh resets it. A missing or failed page stays stale; repeated login/focus
  notifications retain one pending intent until the cooldown expires. Each actual full-pull
  executor owns a lease, shared across manual and automatic entry points; exit,
  errors and cancellation all release it. Manual refresh bypasses automatic
  admission. The cooldown is session-local and does not mark failed data fresh.
- Sequential batches recheck full-pull activity before dispatching each profile:
  a manual refresh may have completed while an earlier profile was running.
- Manual pulls enter ahead of queued background pulls. A running full pull
  coalesces another full refresh; course-detail requests stay queued.
- Automatic work is admitted as one batch at a time and submitted in order:
  the LMS full pull, then 教務Web, then ready lecture details. Each pull is
  submitted after the previous one settles, so the coordinator's FIFO order
  is the policy order. Failures of automatic pulls are logged.
- Visible navigation wakes an interrupted extractor by dropping its waiters.
  The worker requeues its original completion channel, then resumes after
  `hide_all`; preemption is never returned as an error to the caller.
- Automatic work waits while offline or, on cellular, when
  `subscriptionRefresh.allowCellular` is disabled. Mobile starts with the
  conservative cellular state until the shell reports its connection.

The timetable comparison/review remains an explicit action. Automatic source
refreshes do not open its review dialog. The 30-day timetable review policy is
not a separate automatic import timer.


## Execution and outcomes

- Exact target keys coalesce and every caller receives the actual result.
  Reference keys include kind, normalized HTTPS index URL and year; school
  keys include site and optional positive course ID.
- Full LMS and course detail are distinct keys. Full pulls only read assignment
  details; lecture pulls also read files, pages, folders and announcements.
  A full pull must therefore never consume a queued detail request.
- One school lane protects the shared session and WebView executor. The HTTP
  lane permits two reference jobs. Manual admission has priority; equal
  priorities are FIFO. Authentication/navigation retain their own executor
  lifecycle because visible browsing shares those resources.
- Native state snapshots arrive through `pull-state-changed`; `pull_state`
  provides a current read after the listener attaches. Each run has a unique
  identity, so callbacks from earlier attempts cannot change a later run.
- The reducer explicitly handles every pair of 11 finite states and 14
  events (154 combinations). Unsupported events preserve the state and return
  a rejection diagnostic. Queue entries, waiters, counters and run IDs are
  runtime metadata outside the finite control state.
- Cancellation drops queued work immediately. Running work owns its lane until
  the underlying executor finishes; a same-key successor waits for that exit.
  Entering Applying is atomic with cancellation and disables cancellation once
  SQL writes begin. 教務Web may commit pages incrementally, so this gate begins
  with its first successful page.
- Partial school outcomes preserve their original `Ok` payload, including
  successful pages and reconnect-required. The job snapshot ends Failed with
  the reported problem. Retries default to zero, matching the prior adapters;
  an explicit retry policy uses a fresh executor future for each attempt.

Reference refreshes share Core's `ReferenceService` for probing, cache validation,
fetching, applying and metadata writes. Startup always checks the built-in
reference task immediately. Calendar hashes count as usable cache when all
eight valid quarter boundaries exist for the requested year, or when the local
materialization marker still matches the stored rows, so a partially
published calendar is not refetched every hour. Automatic
reference tasks, the Sources action and the Settings view all use the
selected term's year; department recommendations use the admission year.
All reference caches use local materialization fingerprints that are excluded
from backups; this distinguishes a valid empty feed from a missing cache after
restore. An error in one source does not prevent checking the remaining sources.

Automatic school admission preserves the prior foreground/network/cellular/
visible-view conditions. Reference scheduled tasks retain their existing
HTTP admission behavior; the Sources action retains its offline/cellular
checks. Cloud recovery, syllabus lookup and individual file downloads keep
their specialized services and are outside this business pull coordinator.

## State-machine audit — 2026-10-17

The control states remain separate by ownership:

| Owner | State it decides | Inputs / outcomes |
| --- | --- | --- |
| `SchoolRefresh` | Intent, freshness and retry admission | Account/focus facts, successful import timestamps, full-pull lifetime |
| `PullCoordinator` | Queue, lanes, joins, cancellation and terminal result | Admitted targets and correlated executor callbacks |
| `SchoolBrowserEngine` | Authentication, navigation, extraction and visible-view preemption | Browser ports and page relays |
| `DataStore` | Committed page data and successful import timestamps | Per-page apply gate |
| Shell | Presentation | Coordinator snapshots plus advisory progress events |

Observed defect: read-only import metadata showed repeated successful GPA,
declared-course, profile and grade imports seconds apart, with no successful
registration or requirements import. Because the policy correctly required all
six pages, it stayed due. Account/focus events could requeue it immediately.
There was no attempt-level cooldown shared with manual pulls. The lease and
cooldown above close that feedback path without advancing failed-page TTLs.
The exact live failure of the two missing pages still requires native execution;
absence of successful imports alone does not identify a parser error.
The store narrows it: 799 教務Web runs since 2026-10-16 contain no
`registrations` or `requirements` run in any state, while the pages after them
keep importing. So the extractor ran and rejected both pages; navigation did
not time out. Core used to discard the extractor's reason and report the page
as `unknown`. It now keeps both: the kind from the approved source URL, and
the reason as `kyomu extraction was incomplete: <check>`.

A live run with that reporting (2026-10-17, after registration closed) showed
two extractor defects, not unavailable pages. On the registration page, an
empty unit GridView rendered only its empty-data row with no header, and the
header check rejected the whole page. The requirement table has no <thead>;
its blank top-left <td> spans four header rows, so header inference took one
row and treated three header rows as unlabelled requirements. The extractor
now skips a header-less empty GridView. It takes the requirement header depth
from the corner cell's span and rejects any unlabelled row itself. Requirement
ids include the column index, because combined headers need not be unique.
After the fix, all six pages imported (registrations 14 rows, requirements 93),
so the profile becomes fresh. Manual check
`SCHOOL-KYOMU-REGISTRATION-REQUIREMENTS` covers the open 申告 window, which
was not observed.

A separate presentation defect discarded all terminal jobs, including partial
failures, allowing the footer to read Ready. The footer now renders the latest
coordinator failure for each school target, with no running percentage; a newer
successful pull for that target replaces it. Maintenance retains its independent
service status. Partial page failures are logged with site and page kind as well
as preserved in the coordinator outcome.

Existing coordinator protections were retained: exact-target joining, manual
promotion, run identity checks, cancellation holding the lane until executor
exit, per-page apply gates, bounded terminal history and owned shutdown.
The Rust suites exercise late callbacks, cancelling successors, partial-result
payloads and shutdown during commits. Refresh tests additionally exercise the
cooldown boundary, manual bypass and manual work interleaved with an automatic
batch. The real-shell test covers active progress, partial failure, recovery and
maintenance.

Remaining presentation limitation: percentages are estimates from separate
progress/ingest events, rather than a correlated progress field in the
coordinator snapshot. They are not used for admission or terminal outcomes.
A future exact progress contract should carry the run identity and completed /
total units from Core; adding another frontend state machine would compound the
problem. The cooldown is retry throttling, not proof that the missing pages have been
repaired; manual refresh remains available during that interval.

## Footer presentation

The workstation footer shows a progress bar only for pulls the user started
(the view progress or a `manual` coordinator snapshot). Automatic pulls show a
one-line note with a small spinner, including why Core is holding them. When
nothing runs, the footer shows how long ago LMS and 教務Web last imported
successfully (`schoolRecords.lastRuns`), not a generic "Ready".

## Runtime lifetime

`CoreRuntime` owns the coordinator, refresh intent, scheduler and admitted
operations. Desktop supplies a one-second wakeup; Core checks the scheduler at
most every 30 seconds and keeps its RRULE, launch and revision-claim policies.
`SchoolService` chooses the account and loads credentials before calling the
`SchoolExecutor` port, which Core's `SchoolBrowserEngine` implements. The engine
owns WebView navigation, sign-in and visible-view preemption; native shells only
implement the `SchoolBrowser` port (`apps/desktop/src-tauri/src/school_browser_native.rs`).

- Losing main-window focus updates the foreground fact. Actual OS suspension
  stops new execution in both coordinator lanes; resume reopens admission and
  wakes due-work checking. Wake/suspend/resume transitions are serialized.
  Wake reads its stored facts on a blocking worker before taking that lock,
  and focus/visibility facts republish conditions from a spawned task, so the
  UI thread never waits on a workspace read.
- Shutdown permanently closes operation admission, cancels queued/delayed and
  pre-commit pulls, and waits for their executors. It never aborts Applying.
  Delayed retry timers wake on shutdown instead of retaining the runtime.
- Owned operation tracking also waits for final subscription/task metadata and
  events after a pull settles. A dropped UI request does not detach that work.
  Completed job history retains snapshots, not executor closures or native
  handles. Repeated shutdown is safe; resume cannot reopen a closed runtime.
- Desktop owns and closes its wake/event loops, then exits after Core has
  drained, or after 15 seconds with a logged warning if a port never settles. School/native transport waits retain their existing bounded
  timeouts; shutdown uses cooperative cancellation, not forced task abortion.
- Scheduler claims precede side effects. Automatic tasks recheck whether they
  are still enabled/due, and final writes cannot overwrite a concurrent user
  edit or deletion. Event delivery failure does not prevent final persistence.

## Verification

The coordinator tests exercise real fake executors, covering same-key joining,
manual priority/FIFO, full/detail separation, HTTP bounds, cancellation, retry,
panic settlement and stale callbacks. The automatic-school condition matrix
checks all 32 combinations against the previous policy. Adapter tests compare
serialized school outcomes before and after coordination, including partial
failures. Frontend tests verify concurrent requests reach native admission and
that a late initial snapshot cannot overwrite a newer state event. Existing
extractor and SQLite fixture tests remain the data-contract regression suite.
Live portal and physical iOS/Android verification remain separate checks.
