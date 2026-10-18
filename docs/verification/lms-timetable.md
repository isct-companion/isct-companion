# LMS timetable native acceptance — 2026-10-18

Verified on the locally installed, Developer ID signed macOS 1.1.0 app built
from `e44b218`, including the existing uncommitted refresh diagnostics and worker
changes. The installed executable SHA-256 is
`3c0347113012ae853b3d4b8ef8e2ce9e1966dc18732b9e334b593208bd5df8e0`.
System `codesign --verify --deep --strict` passed.

## Fresh-data run

With the app stopped, all app-owned databases, managed course files, caches,
preferences and WebKit state were removed. Keychain credentials, the local
agent credential and school account metadata were retained. Before launch,
only `agent-token` and `school-credential-status.json` remained in the app data
folder, with their contents unchanged. Both school accounts signed in without
credential entry after launch.

The initial automatic full LMS import completed at 00:45:40 JST. The queued
manual school refresh read 教務 and completed a second full LMS import at
00:48:22 JST. Both LMS import records have scope `all` and state `complete`.
The rebuilt database contains 62 LMS courses and 110 assignments. User course,
membership, override, assignment-mark, personal-event and plan tables are empty.
The final app is left running with the newly fetched data.

## Eight-quarter comparison

A read-only SQLite backup supplied the shipped Rust Core scheduled-course read
model. For each term, its complete LMS course-ID set and every expanded
weekday/period pair were compared with the authenticated LMS grids captured on
2026-10-17. Explicit non-weekly membership was compared separately. Both full
imports produced the following result, with no missing/extra courses or slot
differences. Each quarter was also selected in the installed app, with matching
counts; historical and current grids rendered their weekly meetings.

| Term | Weekly | Non-weekly | Total | Result |
| --- | ---: | ---: | ---: | --- |
| 2025-1Q | 7 | 0 | 7 | Match |
| 2025-2Q | 6 | 1 | 7 | Match |
| 2025-3Q | 8 | 4 | 12 | Match |
| 2025-4Q | 8 | 4 | 12 | Match |
| 2026-1Q | 6 | 2 | 8 | Match |
| 2026-2Q | 5 | 3 | 8 | Match |
| 2026-3Q | 5 | 2 | 7 | Match |
| 2026-4Q | 4 | 2 | 6 | Match |

There are 62 distinct LMS courses across 67 course/quarter placements.

The live run caught the dashboard filter's required `search` field being
replaced with an empty value: all seven background requests returned the current
quarter. Preserving the original form field fixed the live requests. Quarter
identity checks continue to reject mismatched responses and retain cached data.

## Home and lecture sidebar

With Show completed selected, the current quarter's graded assignment with no
deadline appears under Earlier → Completed. Its submitted assignment with a
future deadline appears under Today. Submission states and the absent/present
deadlines agree with the rebuilt assignment records. The app was left on the
Home completed view after both refreshes finished.

The installed Lecture Desk has its title on a separate row, with the quarter
selector, mode switch and search field sharing their left edge and width. The
production component also passed browser layout inspection at 167 px and
231 px sidebar widths.

This run covers fresh import, all eight timetable grids, Home completion states
and the sidebar layout. The visible-school-page interruption steps in
`SCHOOL-LMS-PULL` were not repeated in this run. Account HTML, credentials,
assignment titles and grade values are excluded from this committed record.

## Automated validation

- Core: 388 tests passed for the timetable/Home checkpoint.
- Frontend: 537 tests passed, four live tests skipped for that checkpoint.
- Filter follow-up: 34 extractor tests and two extraction-boundary tests passed.
- Sidebar follow-up: 15 existing Today/Lectures tests and frontend build passed.
- Final signed desktop package built and installed successfully.

This acceptance record changes documentation only; no additional production,
test or generated code was added or removed.
