# Today LMS workspace — inspection, 2026-10-05

Status: first workspace implementation is available for UX review. Native authenticated file preview/download and offline file reopening remain incomplete and have not been verified end to end.

Development review: `http://127.0.0.1:5173/?preview=lms` uses isolated sample data and never imports it into the user's workspace. Production Today uses imported school records. A new native LMS refresh is required to populate section bodies and module details.

Implemented: inline Today course workspace, Classes/Assignments mini navigation, source-section expansion, type indexes, course search, document tabs, read-only assignment entry followed by an explicit official submission tab, conservative optional bilingual filtering, and section-relative scroll restoration around the viewport center. Wheel scrolling tracks the current section without forced snapping.

Retention limits: Today remains mounted; at most two course workspaces remain mounted; each course has at most five document tabs, with only the current document mounted. Hidden PDF/image frames are released. The resident native school view is hidden when inactive and reused through the existing login session. Reading metadata is bounded to eight courses and contains no imported HTML or file bytes. Parsed search/preview text is memoized so scrolling does not repeatedly parse all teacher HTML. These bounds are tested; a native process-memory profile has not been performed.

## Requested interaction

- Open a course from Today in an inline workspace on the same layer, replacing the course drawer for this entry point.
- Keep the parsed course content on the left. Keep opened files/pages in a tabbed viewer on the right.
- Render course descriptions, announcements, lesson sections and assignment requirements locally.
- Clicking the submission action opens the official LMS page in a right-side tab, retaining the official form and the app's existing school login flow. Do not navigate the entire app to Web or build a replacement submission form.
- Downloaded files must reopen offline. A saved resource link is not an offline file.
- Parse the common LMS structure, not course-specific features. Sections, rich content, activities, attachments and links are the shared contract; do not infer behavior from a course name or teacher-written heading.
- Present sections as expansion panels: collapsed headers show the original title and activity count; expand to read the body and activities. Retain expansion state per course, allow multiple sections open, and avoid adding another accordion level for every activity.

## Optional bilingual presentation

Automata (`course/view.php?id=8432`) was also inspected. Its lesson summary contains Japanese and English in one HTML body separated by `<br>`. There are no paragraph-level `lang` or Moodle `multilang` markers; the document's `lang="ja"` describes the interface, not every content paragraph.

Do not classify by the largest character count: Japanese paragraphs contain Latin abbreviations, formulas and source identifiers. Literal newline splitting also fails because HTML uses `<br>` and block elements for visible line breaks.

If implemented, Settings → Advanced may offer optional LMS bilingual filtering, disabled by default. Preserve the original rich content and use a presentation-only filter. Prefer explicit content-language markers when present (not the page's language). A conservative fallback may recognize paired bilingual labels such as `授業計画 / Course schedule` and `課題 / Objectives`, with their continuation blocks, only when boundaries are unambiguous. This is shared formatting recognition, not a course-specific parser, and does not prove the two blocks contain identical information. Keep ambiguous blocks, tables, code, formulas, attachments and links visible. Provide a local “Show original” action; never delete source content or alter task/date extraction based on the filter. Broad language guessing is outside the initial implementation.

## Real examples

All URLs below are within `https://lms.s.isct.ac.jp/2025/`. The instance path is not the academic year: the inspected course identities are `[2026]`.

| Course | Observed content | Consequence |
| --- | --- | --- |
| Sports, `course/view.php?id=12793` | Bilingual lesson descriptions and contact section; most lessons have no material modules | Persist sections independently from resource rows, including text-only sections |
| OOP, `course/view.php?id=8442` | Inline schedule image, PDF resources, Java source, repeated lesson titles and a section order that differs from title numbers | Preserve section identity and LMS order; support inline assets and more than PDF |
| Assembly, `course/view.php?id=8438` | Folder `mod/folder/view.php?id=135411` contains a ZIP; assignment `mod/assign/view.php?id=135414` has description, dates and submission status | Folder contents and assignment details need separate read-only extraction |
| Switching Circuit Theory, `course/view.php?id=8433` | A schedule table contains cancellations, room changes, report deadlines and an exam entry | Preserve tables; these teacher entries are not automatically global calendar exceptions |
| Archaeology, `course/view.php?id=11677` | Announcements `mod/forum/view.php?id=68508` contain a classroom change absent from the course outline | Announcement list and body are part of the useful course view |
| Group Work, `course/view.php?id=9315` | One section describes multiple sessions; contact text contains an external form | Section titles cannot be mapped one-to-one to lessons or dates; keep external links explicit |

Archaeology announcement `mod/forum/discuss.php?d=33638` states that all classes from October 1 move from M-156 to W3-707. The later reminder `d=33724` mentions that day's class. Reading only the reminder would lose the continuing scope. Show the source and full announcement; do not silently infer calendar edits from a title.

OOP resource `mod/resource/view.php?id=136736` resolves to `pluginfile.php/223866/mod_resource/content/3/OOP-0.pdf`. The file/version path matters. The inspected browser did not display PDF pages, so this does not verify an integrated PDF preview.

Assembly's visible folder file is `pluginfile.php/222370/mod_folder/content/0/20260928.zip?forcedownload=1`. No download or submission was performed.

Group Work's syllabus wrapper `mod/url/view.php?id=84250` exposes `https://syllabus.s.isct.ac.jp/courses?jwc=202637648`. A Moodle URL module can be a wrapper around another site rather than the final page.

## Current implementation gaps

- `crates/isct-core/src/lms_ingest.rs` whitelists section identity/title/visibility and module identity/link fields. It drops section bodies and inline assets, then persists module rows without independent sections.
- `apps/desktop/src/pages/courses/CourseDetails.tsx` builds sections from material rows; text-only and empty sections therefore disappear. Resource links currently use the app-wide Web route.
- `apps/desktop/src/pages/today/TodayPage.tsx` opens the shared course drawer through `App.tsx`.
- `apps/desktop/src/cache/index.ts` stores bounded JSON metadata; it is not a binary file cache.
- `apps/desktop/src-tauri/src/school_web.rs` already has persistent school WebViews and measured bounds. The right-side official tab must reuse this infrastructure and hide the native view when another tab is active. Native WebView placement cannot be handled by CSS z-index alone.

## Implementation boundary

1. Preserve course sections, original order, safe rich content, module types, visibility and source identity. Extend available read-only APIs only where verified; use bounded on-demand HTML extraction for missing bodies and detail pages.
2. Render the same section/content/activity model for every course. Use Moodle module types for shared readers (resource, folder, forum, URL and assignment), not title matching or course-specific layouts. Keep contact text, syllabus links and schedule tables in their original content positions. The word `課題` in section text can mean learning objectives; only actual activity modules identify submissions. Do not interpret teacher prose into calendar overrides in this workflow.
3. Build the inline course workspace and tab ownership before wiring the official submission view. Mark that tab clearly as official LMS, with its host visible.
4. Resolve authenticated file resources using the native app's school session. The signed-in inspection browser is not proof that the app WebView shares that session.
5. Store downloaded bytes in an app-managed file cache, with account/course/resource identity, source version, file name/type, integrity and completion status. Never show a failed or partial download as available offline. Cache read-only content snapshots separately, with a last-synced timestamp.
6. Verify PDF/image/source previews, ZIP download, offline reopening, session expiration and switching away from the official native tab. Preserve the last valid local snapshot when refreshing fails.

This note records the current user direction. Older task documents describing assignment clicks as app-wide Web navigation need reconciliation when implementation starts.

## UX decisions after research

Visual thesis: a calm, dense reading workspace with two useful columns, plain dividers and one accent for the selected course or item.

Content plan: Today is the contextual entrance; the course outline remains on the left; the opened document or official action page occupies the right. Avoid a permanently expanded third Today column that makes both reading columns too narrow. Keep a compact course-switch/return control available within Today.

Interaction thesis: expand and reveal the clicked target on entry; use a brief selection highlight to explain the landing position; preserve each course's reading position and document tabs. Respect reduced motion and never repeat auto-scrolling after a user has started reading or a background refresh finishes.

### Contextual entry and positioning

Today currently calls `open(courseId)` and loses the clicked occurrence's date/time. Carry a structured entry context: course identity, occurrence key, local date, start/end time and optional activity/section identity. The unified schedule resolver determines actual occurrences, including replacements; it must not independently invent an LMS lesson-to-date mapping.

Positioning priority:

1. An assignment/activity click supplies its stable identity: reveal that exact item and its parent, without a second selection step. Reading the assignment must not automatically initiate the official submission flow.
2. A course occurrence may reveal a corresponding section only when an explicit LMS date/identity association or a saved user association exists. Display the original title; do not label an inferred section as today's lesson.
3. Without a trustworthy association, resume that course's last reading position. On a first visit, show the outline and a relevant dated activity when its date is explicit; describe it as a dated activity, not as a lesson match. Otherwise remain at the outline rather than guessing from `第N回` or week count.
4. Preserve both the association evidence and the fallback reason internally. Never derive the academic year from `/2025/`, the lesson number from section order, or the lesson date from file upload time.

Automatic landing happens once for an explicit entry context. A user-requested jump to the entry target may repeat it. Refresh must not move focus, reset expanded panels or steal the current document tab.

### Grouping and expansion

Use common module types to provide scan-friendly areas: course content/resources, assignments, forum announcements/discussions, links and other activities. These are presentation indexes over stable source records, not changes to the imported LMS structure. Keep unknown module types visible with their original names.

Keep only one expansion level in the primary reading path. Course content can use source sections as expansion panels; type labels are plain headings, not a second enclosing accordion. Short assignment/forum/link rows are directly actionable. Do not require opening a type group, then a lesson, then a file just to read a document.

Headers retain original section titles, counts and a short body preview when useful. The landing panel is already open. Other panels can stay closed, with independent expansion and an unobtrusive Expand all action. When a type index lists an item outside its original section, include the original section title as secondary context and resolve to the same identity, not a duplicated record.

Course search matches titles and cached body text; results reveal their source section automatically. Search must not depend on manually opening every panel. Date/deadline and download state should be visible in relevant rows rather than hidden inside an expansion.

### Viewer and operation budget

- Click Today course → course workspace and best trustworthy landing position, one action.
- Click a visible resource → right preview, one further action; no intermediate resource detail dialog.
- Click Today assignment → its read-only requirements/status directly, one action.
- Click Submit in that assignment → official LMS tab on the right, one further action. The actual submission stays in the official UI.
- Reopening a downloaded resource offline uses the same row and tab behavior; no separate offline browser.

Reuse an already open tab for the same item. Avoid automatically opening all lesson attachments or returning to the first tab on refresh. Persist per-course selected tab, expanded sections, scroll position and document position. Mark official LMS and offline content with clear, concise status. On narrow windows, show one reading surface at a time with a persistent back control rather than squeezing three columns or introducing nested drawers.

### Accessibility and verification

Use heading buttons with `aria-expanded` and `aria-controls`; support Enter/Space and normal Tab navigation. Collapsed content must not retain invisible focus targets. Keep download/menu buttons beside an accordion heading button rather than nested inside it. Preserve logical focus when revealing a target and respect sticky header offsets.

Verify contextual landing against explicit identity/date, no matching date, duplicate section titles, replacement days and reopened courses. Measure actual action counts to reach a file, not merely the count of visible controls. Check search with collapsed panels, empty sections, unknown modules, narrow layout, refresh while reading and offline cached files. These are acceptance targets, not results of a conducted usability study.

Research informing these decisions:

- [NN/g: Accordions on Desktop](https://www.nngroup.com/articles/accordions-on-desktop/): reduced scrolling trades off against hidden content and extra interaction; avoid forcing repeated expansion or closing another needed panel.
- [NN/g: Progressive Disclosure](https://www.nngroup.com/articles/progressive-disclosure/): prioritize frequent actions and reveal secondary complexity as needed.
- [VS Code: User Interface](https://code.visualstudio.com/docs/editing/getting-started/userinterface): stable explorer/context beside a tabbed document area and preserved workspace state. This provides a documented workstation reference; the design does not copy every editor control.
- [W3C APG: Accordion](https://www.w3.org/WAI/ARIA/apg/patterns/accordion/): keyboard, heading/button and expanded-state requirements.

## Adopted reader direction — 2026-10-05

This supersedes the fixed outline/viewer composition discussed above. Today remains full width until opening a course, then its existing Classes and Assignments become navigation groups. The main workspace is a continuous section reader in original LMS order, with common text activities and read-only assignment requirements inline. Sections start expanded; collapsing remains optional. A document or local note adds a second column, and closing the last document returns the course to full width. Narrow windows switch between reader and document instead of squeezing both.

Section `+` creates a Markdown note with math support using ByteMD. Notes autosave with an 800 ms debounce and flush on unmount; writes are ordered. Failed writes retain a recovery draft and expose retry. Full TeX documents are displayed as source; no TeX compiler is included. Official submission remains in the resident LMS WebView with unchanged login handling. Unknown resources retain a source-link fallback rather than course-specific parsing.

Files use `<root>/<year-quarter>/<course-code>/section-<stable-id>/materials|notes/`. Notes have independent UUID files; cached materials use source URL hashes plus filenames, preserving version changes and file extensions. Native downloads stream authenticated LMS attachments to disk; text preview is UTF-8 and bounded to 1 MB, attachment downloads to 128 MB. The file picker changes only the configured root, leaving previous files intact. The browser demonstration uses separate preview note storage and does not impersonate native authenticated downloads. See `lms-performance.md` for measured limits and unverified native behavior.

## Native regression review — 2026-10-05

Reproduced on the packaged IsctCompanion Tauri app with the existing local data, not the browser sample. Course 8442 (Object-Oriented Programming) displayed the first lesson summary but none of its four resources. The stored course section IDs were JSON strings (`"115791"`), while material `sectionId` was numeric (`115791`); strict equality silently dropped every section activity. Normalize upstream IDs and also normalize previously cached sections at the reader boundary.

The two General images were image-only label activities. Their teacher HTML lived in a theme-specific wrapper omitted by the previous body selector. Extraction now uses Moodle API activity/section anchors and IDs, clones the whole corresponding content node, removes controls/navigation by element semantics and URLs, and preserves teacher text, tables, file links and image links. It does not interpret lesson titles or add rules for this course. A linked thumbnail preserves its full-size destination as one attachment link instead of creating nested anchors. Theme icons are excluded. DOM structure was also inspected on courses 8432 (Automata and Formal Languages) and 12793 (Advanced Sports Exercise).

A real SQLite concurrency regression reproduced `database is locked` when a workspace save upgraded a deferred read transaction while an import writer held the lock. Workspace save and LMS import now reserve the write transaction before reading; workspace persistence runs on a blocking worker instead of blocking the app UI. Permanent failures still surface the actual reason and preserve the in-memory draft.

Workstation feedback uses short entry/expansion transitions, slight attachment-row feedback, and pane motion. There are no animation loops; reduced-motion preference disables them.

### Announcement and syllabus follow-up

Forum list tables are no longer used as the reading surface. A bounded read-only GET follows at most eight discussion links per forum and sixteen per import, validated to the existing LMS origin and `/2025/mod/forum/discuss.php`. Post identity (`p<ID>` / `post-content-<ID>`) supplies title, date, body and attachments without theme wrapper classes. The real discussion 33638 has this contract. Announcements render as separate articles; submission/reply controls are excluded. This is a recent, bounded snapshot rather than an exhaustive forum archive.

The redundant シラバス / Syllabus item is hidden in the course outline. A dedicated Open in syllabus action loads the existing syllabus reader in the right tab pane. Other course links stay visible.

Native verification exposed blank WebKit PDF iframe rendering despite successful disk caching. Replaced the PDF iframe with a lazy PDF.js canvas reader, one page at a time. Viewer unmount cancels rendering and destroys the document/worker; a lifecycle regression checks both. Teacher images keep the full-size attachment target, not the thumbnail.

Actual packaged App (`tauri://localhost`, debug .app) acceptance: course 8442 now lists all four first-lesson files and both full-size image links. The schedule image downloaded into the native course folder; OOP-0.pdf reopened from local disk with Available offline and rendered its 24-page document in the right reader. A second real course (11677) displayed the classroom-change announcement bodies inline. Native refresh completed without a workspace-save error in this run; the intermittent write-lock path is independently reproduced by the SQLite concurrency regression. Final frontend suite: 179 tests across 32 files passed; debug .app build passed.

### Follow-up acceptance changes

Announcements now use compact cards on the left and a full-text right tab; attachments and file/link rows have separate card treatments. Moodle's empty-fragment collapse/expand link is filtered. PDF reading is continuous: placeholder page geometry preserves scrolling while canvases mount only near the viewport and cancel/free on exit. The native download action uses Save As, rather than sending the internal asset protocol to a system browser; original URLs use the native external opener.

Settings / General: Automatically save course files defaults to false. False uses a per-process temporary preview folder, removed on normal application exit; true uses the configured course folder. Explicit Download always presents Save As on desktop. Existing course folders are not deleted. Session-restored events retry previews that were opened before the resident LMS view was ready.

Account status now persists only username and saved/OTP/matrix-present flags in application metadata. Startup no longer unlocks the vault to draw badges; the existing persistent WebKit session is checked first. An actual renewed login still requires Keychain authorization when macOS requires it. A regression recreates the native service with a locked fake vault: status needs zero vault reads, contains no secrets, and cannot authorize an actual login.

The shared syllabus reader now accepts the official offering-ID redirect locator and rejects a returned different offering. An English-source regression covers locator resolution and English objectives. Cached Offering syllabusUrl is preferred when present.

### Document reader and unified credentials (2026-10-05)

The right reader now has document chrome with a separate source/status bar. PDF.js provides continuous vertical pages, page-number navigation, fit-width/zoom controls, selectable text, and an on-demand text search that locates the next matching page. Text search does not eagerly index the entire document. Markdown uses the existing readonly Milkdown renderer; TXT and C/C++/Java/Python/R attachments use a lazy, readonly, virtualized CodeMirror view with line numbers and syntax highlighting for the named programming languages. UTF-8 text previews remain bounded to 1 MB. Manual Save As defaults to the platform Downloads directory, independently of the configured notes/offline storage directory.

Actual native verification before the final polish: Save As exported `/private/tmp/isct-native-export-proof.pdf`, the UI reported Saved locally, and file inspection confirmed a valid PDF. The native continuous reader displayed adjacent pages 8 and 9 with only nearby page canvases present in the accessibility tree. Cold startup exposed a navigation/authentication race: a provisional LMS dashboard URL was mistaken for a completed session. Attachment preparation now waits for the finished page-load callback, rather than the provisional webview URL.

School credentials are stored in one versioned `school-accounts` Keychain JSON item with separate Science/Tokyo Tech entries. Read-modify-write is serialized. Existing per-profile items are loaded, validated, written into the bundle, and read back before legacy cleanup. Migration/write failure leaves original credentials intact. Forgetting a single profile preserves the other; an empty bundle prevents failed legacy cleanup from resurrecting forgotten credentials. This migration runs when credentials are actually needed, not simply when cached status badges are displayed. Credential values remain native-only and never enter the metadata status cache.

Final automated reader checks: frontend suite 186/186 across 35 files, including later-page PDF search, zoom, bounded canvas mounting and worker destruction; native school-auth suite 11/11, including migration retry and preservation of the other account on deletion. The OS vault allowlist includes the new stable `passwords.school-accounts` item; legacy portal key IDs stay available exclusively for migration.

### Final native acceptance: section notes and workstation polish

Each section has one primary note entry. Repeated creation opens the existing note; older duplicate notes remain under Previous notes. Deletion queues behind autosave, moves the note into recoverable deleted-notes storage, closes its tab, and offers Undo. Native acceptance used a newly created OOP first-section note: repeated creation, delete, restore and final soft-delete passed without touching existing notes. Markdown editing remains one inline document surface with formulas and lazy code-block highlighting.

Workstation chrome follows the supplied panel reference: rounded selected tabs, compact document/source bar and a full reading canvas. Opening and closing use short CSS transitions with reduced-motion support; the closing pane is inert until removed. Native close restored the full Today overview, and reopening restored the course reader.

Final packaged App acceptance verified the cached 24-page OOP-0 PDF reopening with automatic storage off, PDF find for Java navigating to page 6, selectable text layers, and the native Save As panel selecting Downloads. Previously saved files are reused before authenticated downloads, without silently enabling persistent storage. PDF search explicitly reads the text stream with getReader(), avoiding unsupported ReadableStream async iteration in native WebKit. English LAH.T319 syllabus loaded its English overview and 14-session outline in the right tab. The foreground native screenshot is /private/tmp/isct-workstation-final.png.

Final frontend suite: 188/188 tests across 35 files. Native school-auth tests: 11/11; unified vault ID allowlist: 2/2. Latest debug native bundle built successfully. Fresh uncached downloads had an intermittent transport failure during an earlier run; cached reopen and manual export passed, but universal fresh-download reliability is not established.

### Compact reader and direct attachments (2026-10-05)

Removed the visible document tab strip; course cards remain the navigation. Closing the active reader no longer reveals an unrelated previously opened file. PDF title/file actions and page/zoom controls share one wrapping header, with find shown only on demand. Native WebKit gesture events, ctrl-wheel trackpad pinch, and two-touch fallback temporarily scale the surface and redraw bounded canvases only at gesture end (wheel inactivity: 140 ms). The pivot stays fixed during each gesture. Physical trackpad feel remains unverified.

Unsupported inline attachments now follow the same direct Save As route as unsupported resource rows; ZIP no longer opens an empty right preview. Resource URLs stored in either resourceUrl or the pluginfile URL field are handled. Native attachment preparation waits for a completed LMS page even when a newly created webview already exists. Redirects are limited to at most three validated HTTPS LMS pluginfile targets; external/login redirects stop, and non-file errors show HTTP status.

Left image links preserve their full-size destination and lazy-load bounded thumbnails (280 px maximum width, 160 px maximum height) through the native authenticated file cache. Native images do not send cookies to frontend JavaScript, and no new persistent storage is implicitly enabled.

Acceptance: frontend 189/189, native file-store tests 2/2, debug bundle builds. A native ZIP attempt before the session-readiness follow-up returned a non-file LMS response; native LMS dashboard sign-in was subsequently confirmed. Final scrolling/download acceptance is blocked by repeated native window-control noWindowsAvailable errors, so ZIP save success, the final thumbnail appearance and physical pinch behavior are not claimed as verified.
