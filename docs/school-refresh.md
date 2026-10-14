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
  the visible school view closes. Successful fills are remembered for the
  session; a failed fill can be requested again on reentry.
- Manual pulls enter ahead of queued background pulls. A running full pull
  coalesces another full refresh; course-detail requests stay queued.
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
reference task immediately. Calendar hashes count as usable cache only when
all eight valid quarter boundaries exist for the requested year. Automatic
reference tasks use the current academic year; the Sources action uses the
selected year, and department recommendations use the admission year.
Other reference caches use local materialization fingerprints that are excluded
from backups; this distinguishes a valid empty feed from a missing cache after
restore. An error in one source does not prevent checking the remaining sources.

Automatic school admission preserves the prior foreground/network/cellular/
visible-view conditions. Reference scheduled tasks retain their existing
HTTP admission behavior; the Sources action retains its offline/cellular
checks. Cloud recovery, syllabus lookup and individual file downloads keep
their specialized services and are outside this business pull coordinator.

## Runtime lifetime

`CoreRuntime` owns the coordinator, refresh intent, scheduler and admitted
operations. Desktop supplies a one-second wakeup; Core checks the scheduler at
most every 30 seconds and keeps its RRULE, launch and revision-claim policies.
`SchoolService` chooses the account and loads credentials before calling the
temporary `SchoolExecutor` port. The existing native Controller still owns
WebView navigation/session preemption until its separate migration.

- Losing main-window focus updates the foreground fact. Actual OS suspension
  stops new execution in both coordinator lanes; resume reopens admission and
  wakes due-work checking. Wake/suspend/resume transitions are serialized.
- Shutdown permanently closes operation admission, cancels queued/delayed and
  pre-commit pulls, and waits for their executors. It never aborts Applying.
  Delayed retry timers wake on shutdown instead of retaining the runtime.
- Owned operation tracking also waits for final subscription/task metadata and
  events after a pull settles. A dropped UI request does not detach that work.
  Completed job history retains snapshots, not executor closures or native
  handles. Repeated shutdown is safe; resume cannot reopen a closed runtime.
- Desktop owns and closes its wake/event loops, then exits only after Core
  has drained. School/native transport waits retain their existing bounded
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
