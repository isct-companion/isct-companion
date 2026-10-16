# Official course sources — 2026-10-16

## Source ownership audit

| Boundary | Owner and behaviour |
| --- | --- |
| Current enrollment, quarters, times, rooms | 教務 registration snapshots, scoped by academic year and displayed quarters. Empty current results expire only that scope. Earlier terms and user plan rows survive. |
| Current academic identity | 教務 student profile: department, admission date, programme and entry category only. Saved nonempty user choices win. No name, student number or contact fields are stored by this reader. |
| Course discovery | Authenticated 教務 search returns complete schedule, credits and requirement metadata. Quarter, day, possible start periods, level, language and requirement groups reach the official query. Teacher is remote; room and exact time overlap are checked locally. |
| Course detail | Syllabus is fetched on opening a result. Shared storage merge preserves 教務 schedule, credits and Japanese title in either arrival order. |
| Learning content | LMS remains responsible for materials and assignments. Match by offering ID; otherwise require a unique year/code/quarter match, preserving class-section suffixes. A shared projected LMS ID serves desktop content links. |
| View, agenda and ICS | Shared scheduled-course projection filters meetings by quarter before calendar generation. Official multi-quarter rooms and clocks remain separate. |
| Shared native API | Core catalog service and FFI return course IDs, next cursor and truncation state. Desktop renders this; mobile can use the same service without a second pagination policy. |

## Pagination and keyboard flow

Each query page emits at most 100 courses. The hidden reader requests the grid's 100-row page size when available, follows only approved grid paging postbacks, and excludes submit buttons from FormData. Opaque form state stays in the school origin's session storage, bound to the query URL and a five-minute expiry, with at most 16 cursors. Native/FFI receives no form state.

A university session may already have a larger page size saved and omit its page-size control. In that case, the initial server response can contain more than 100 rows; the reader caches only the public result grid and emits 100 per cursor. The client never intentionally requests 1000 rows. An apparent official result cap is surfaced as a narrowing warning rather than an exact total.

Desktop appends pages with ID deduplication and a scroll sentinel. A Load more action provides the same operation for keyboard users. Query changes invalidate pending page responses; failures retain existing rows and allow retry. Enter/Tab accept suggestions, Escape closes them, arrows reopen them, and IME confirmation is ignored.

## Removed paths and retained responsibilities

Removed automatic LMS timetable initialization, the desktop LMS comparison/import dialog and toolbar, the automatic department/study-guide subscription, and the admission-year-triggered reference refresh. Existing explicit backend LMS import remains a user-requested planning operation; it cannot override covered official enrollment. Public guide parsers and stored collections still serve existing data/import boundaries; no live guide subscription remains.

The two old LMS-initialization tests were superseded by `full_lms_pull_updates_content_without_owning_the_timetable` and `official_enrollment_owns_the_calendar_while_lms_and_personal_plans_survive`. The latter also covers quarter rooms, half credits, previous-semester retention, personal plans, content linking, personal title/colour overlays on freshly projected courses, rejection of local official-enrollment removal, and withdrawal without LMS resurrection. No active source gets a parallel shell-owned state machine.

## Verification

- Real authenticated 教務 UI: profile field structure and admission date, per-weekday rooms, current registration scope, course-grid columns, quarter combination values, day/start-period/level/language/requirement URL parameters, English course-name search, and teacher query parameter were inspected. Teacher `中田` narrowed the Programming search from 23 rows to 1.
- Public course-grid fixture has its URL, capture date, byte count and SHA-256 in `crates/isct-core/tests/fixtures/kyomu/content-provenance.json`. Surrounding profile, account header and hidden form fields are excluded.
- Automated regression covers bounded cursor batches, cross-query cursor rejection, ASP.NET page-size and next-page form events, no registration submit payload, desktop row appending, IME/keyboard selection, official projections, source field ownership and subscription removal.
- Validation passed: 515 frontend tests (4 live tests skipped), TypeScript test checking, production desktop web build, Rust workspace tests (369 Core, 22 desktop unit, 9 desktop integration, 16 FFI, 19 OCR; 4 OCR checks ignored). The final cursor retirement adjustment also passed all 28 extractor tests.
- Native authenticated end-to-end acceptance remains pending: `SCHOOL-OFFICIAL-COURSE-PAGES` in the manual table. Browser DOM evidence does not claim a pass for native hidden-browser execution. No real registration, user database restore/reset, mobile build or release was performed.

## Change size

Production and tests/generated output are counted separately, relative to the previous checkpoint. This source change is a net production addition; no net code-reduction claim is made.

- production: +2040 / −991 lines.
- tests/fixtures: +1073 / −126 lines.
- generated: +9 / −1 lines.

## Calendar, refresh and credit follow-up

Today owns a dated weekly calendar; Timetable owns quarter selection and editing.
Both consume Core read models. A bounded shared transport cache retains the last
complete result until a replacement arrives and ignores superseded responses.
Lecture schedule/task queries are one batch; orphaned assignments never assume a
course exists. LMS full pulls now cover all courses, with a separate current-quarter
intent. The first automatic pull uses that full scope; subsequent wakeups refresh
cached content in the background. Changes appear through a short in-app toast.

Historical earned credits come from the formal grade ledger, including fractional
lecture/exercise/lab splits and its official total. Pending grades do not count as
earned; failed grades contribute zero. Credits spanning quarters are shared across
those quarters. Current-semester declarations take precedence over partial grade
publication. Competency points are not a credit source: live inspection found a
course with 1.5 credits but 1 competency point and different quarter attribution.
No personal grade HTML is committed.

Desktop updates use Tauri's signature verifier and the existing owned shutdown.
CI accepts `ISCT_UPDATER_PUBLIC_KEY` as a repository variable and
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as secrets;
only a complete signed set for both Mac architectures and Windows produces an
update manifest. No signing key was generated or uploaded in this change.
An unsigned-config build can check releases and open the download page, but
cannot enable automatic installation. Native installation remains a manual check.

Follow-up code accounting (documentation excluded): production +1771 / −537 lines; tests +325 / −84 lines; generated +259 / −11 lines. No test was removed; existing suites were updated for the separate timetable route and extended for fractional/historical credits, all-quarter localization, retained query results and signed artifact collection.
