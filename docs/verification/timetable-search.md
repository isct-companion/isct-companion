# Timetable search and progress regression, 16 October 2026

The reported CSC.T244 record contained Monday 5–8 and 7–8, and Thursday
5–8 and 5–6. Those contained slots were rendered twice and counted as
self-conflicts. The shared scheduled-course read model now removes contained
meetings with the same days, room and term scope before producing templates,
dated calendars or exports. Other courses overlapping that time still conflict.
Stored records are not rewritten by this rendering correction.

The finder previously attributed a retained asynchronous plan to the newly
selected quarter while its actual plan was loading. Only a settled successful
plan can now acquire the new request identity. Superseded catalogue jobs are
cancelled through the existing pull service, and a bounded five-minute result
cache serves repeat queries. Consumed native paging cursors are not reused.

Hidden acquisition and visible browsing share the school browser. Hidden loads
now retain a separate foreground destination, do not emit foreground URL changes
or overwrite its restore file, and opening Web restores that destination. An
interactive search preempted by visible browsing returns immediately rather
than holding the school queue while waiting for the person to leave Web.

The footer now projects the active school source, phase and progress together.
A queued 教務 pull shows its waiting state rather than an idle LMS Ready label.

The course list uses TanStack Virtual with measured row heights and five rows
of overscan. Native transport still fetches at most 100 results per page; loaded
results remain data, while only the visible range is mounted. The existing paging
case now loads 1,000 results in ten requests, scrolls to the final course and
asserts fewer than 25 mounted course rows throughout its scripted viewport.

Validation:

- Frontend: 519 passed, four skipped, including the expanded paging case and
  the source/progress regression.
- Type checks and the production frontend build passed.
- Rust workspace tests passed, including real-conflict preservation and the
  scripted browser's foreground URL restoration. The loopback-server tests
  require execution outside the default socket-binding sandbox.
- No tests were removed. The paging case was extended rather than duplicated;
  jsdom supplies geometry while running the production virtualizer.

Authenticated native postback/navigation timing remains a separate manual
check in `SCHOOL-OFFICIAL-COURSE-PAGES`; this regression run does not mark that
live-account check as passed. No remote build, release or app installation was
performed for these changes.
