# Quarter switching and stalled school refresh, 17 October 2026

The installed `1.0.1-dev.2` was reproduced with native accessibility and an
8-second macOS thread sample. The footer remained at 5% and course search
remained pending. System WebKit logs recorded JavaScript evaluation failures
approximately 60 seconds apart, matching the hidden page capture timeout.

Both official extractors end in a semicolon. The generated wrapper embedded
that statement inside `Promise.resolve(<script>)`; parsing the actual assets in
Node reproduced `SyntaxError: missing ) after argument list`. The shared relay
now runs the extractor as an initializer in a promise continuation, accepting
leading comments and final semicolons. Runtime exceptions return a correlated,
redacted error instead of silently waiting for the capture timeout. The JS
suite executes this production relay template with both real official scripts,
asynchronous success, Unicode output and asynchronous rejection. Existing Rust
browser cases retain run-correlation coverage and now read the relay constants.

The native sample also showed repeated workspace reads and cloud work. A
profiling example, run on a disposable consistent copy of the user's database,
measured the quarter template itself at 0–3 ms, while full debug workspace reads
cost 975–2976 ms and quarter mutations 1406–3875 ms. Background app and compiler
load varied during the run, so these are observed timings, not a controlled
release benchmark. The previous installed local package used a debug build.

Rust now retains the validated workspace, school records and content fingerprint
behind a persistent SQLite reader. `PRAGMA data_version` invalidates the cache
on external commits; the Japan calendar date invalidates time-dependent fields.
Workspace writes use that same connection under an IMMEDIATE transaction,
validate the intent revision and publish their committed snapshot into the
cache. Quarter selection changes only its field in the already validated
projection. Failed or stale writes cannot publish a speculative snapshot.
The existing settings/validation case now checks invalidation through a separate
DataStore handle without changing the user-intent revision.

With the cache and retained fingerprint, subsequent debug snapshots were 25–45 ms;
mutations were still 828–1126 ms under concurrent compiler load. Release/native
end-to-end timing is recorded separately below rather than inferred from this.

The finder no longer starts a remote request simply because a quarter is opened.
It displays stored results until an explicit official search, query edit or
refresh. The existing paging regression checks that initial mounting issues no
catalogue request before its 1,000-row virtual-list flow. The footer's Refresh
now bypasses remote TTL via the existing manual school pulls. It refreshes
教務Web and LMS and coalesces through the shared coordinator. Automatic 教務
captures now advance their own progress when a page arrives; they previously
left the manual-only progress value unset and displayed 5% throughout.

Quarter selection uses the existing Mantine debounced callback with a 100 ms
trailing delay in both layouts. Rapid clicks submit only the final quarter; the
existing picker case now verifies this through the real shell.

Fixed source TTLs remain in Core, with the existing bounded five-minute finder
transport cache. Adaptive hit-based TTL shortening has not been enabled. Local
SQLite cache invalidation is commit-based, not TTL-based.

Verification before native packaging: 520 frontend cases passed, four skipped;
Rust workspace tests passed (372 Core cases, 22 native, nine native integration,
16 FFI, 19 OCR with four ignored). The final automatic-progress extension passed
its focused shell run. No test was removed; existing paging, settings and shell
cases were extended. The live native postback check is not marked passed by
these headless results.

`ISCT_TRACE_READS=1` enables metadata-only cache and native command timings on
stderr: cache hit/miss, queue/work time and mutation/cloud-capture time. It does
not print query strings, course contents or credentials. The profiling example
requires `--disposable-db` because it changes quarter selection in that copy.

Native release verification (`1.0.1-dev.3`): quarter mutations measured 93–97 ms
including persistence (87–89 ms) and cloud capture (6–8 ms); subsequent schedule
queries took 13–15 ms, with native queue waits of 0–26 ms. These are command
timings, not end-to-end rendering latency, and the new debounce adds 100 ms
before submission. The former 3–4 second stall did not recur in those switches.
The official declared/profile/grades/GPA imports completed on the real account,
and an explicit 4Q catalogue search returned seven courses. Both signed-in
connections and the existing timetable remained visible.

The final local `1.0.1-dev.4` release package includes the 100 ms debounce. Its
Developer ID signature passed strict/deep verification before installation.
Native reopening retained both signed-in connections; 2Q selection settled
correctly, and opening Edit showed stored results and the explicit Search
教務Web button without starting a catalogue search. Every original ID in manual
courses (63), LMS courses (62), grades (49), plans (2) and course overrides (1)
was retained. The final frontend run passed 520 cases with four skipped; type
checking passed. The previous application and a consistent pre-install database
backup remain local. No remote build, release or push was performed.
