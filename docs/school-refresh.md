# School data refresh

Refreshing school data (LMS, 教務Web) feels erratic today, and it blocks the
person. This is the brief for making it predictable.

## What happens now

- `open_internal` in `backend/src/school_web_shared.rs` refuses to show any
  school page while a pull job runs: "Wait for the school pull to finish
  before opening this page." On a phone, opening a lecture starts a detail
  pull for that course; tapping "Open in LMS" right after hits the refusal.
- Refreshes start from several places that do not know about each other:
  - the frontend (`src/app/App.tsx`): when an account becomes active, when
    the window regains focus after 10 minutes (`refreshStale`), and once per
    session when a lecture opens (LMS detail fill);
  - the Rust scheduler (`backend/src/scheduler.rs`), polling every 30 s.

## What it should do

1. **The person always wins.** Opening a school page never fails because of
   a background pull. Either the pull runs in its own hidden view so the
   visible one is free, or the visible request preempts it: pause or cancel
   the job, show the page, and resume or retry the job afterwards. No error
   reaches the UI for this.
2. **One scheduler.** Rust owns when refreshes run. The frontend only reports
   events (account active, app foregrounded, lecture opened) and requests a
   manual refresh; it no longer decides TTLs itself. Remove `refreshStale`
   and the per-lecture detail trigger from `App.tsx` once Rust handles them.
3. **Predictable rhythm.** Document and test it, roughly:
   - on launch and on foreground: refresh only sources older than their TTL;
   - opening a lecture: fill that course's details at low priority, after any
     visible navigation, at most once per session;
   - manual refresh: runs now, ahead of scheduled work;
   - respect the existing cellular setting on mobile.
4. **Quiet.** Background refreshes report through the existing progress
   events only. The mobile status dot shows progress; no toasts or dialogs
   for routine refreshes.

## Acceptance

- Rust test: a visible open during an active job succeeds, and the job
  completes or is retried afterwards.
- Opening a lecture and immediately tapping "Open in LMS" on a phone shows
  the LMS page with no error.
- The frontend contains no refresh timing logic; scheduler tests cover the
  rhythm above.

## Implemented rhythm

`backend/src/school_refresh.rs` is the refresh policy; the existing scheduled
jobs feed the same queue. `App.tsx` reports foreground/network and lecture
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
