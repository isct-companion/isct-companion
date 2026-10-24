# Mobile app design

Design direction for the native phone shells (SwiftUI on iOS, Jetpack Compose
on Android). Logic lives in `isct-core` (see `docs/core-consolidation.md`);
this document covers what the shells show and how.

The shells follow the desktop's pages and actions (`routes.ts`: Home,
Timetable, Lectures, Web, Settings) adapted to touch, not a separate feature
set. iPhone and iPad differ slightly on purpose: the iPhone is for quick,
accurate looks; the iPad is half a workstation (see [Devices and
layouts](#devices-and-layouts)).

Design canvas (iOS, 12 screens):
<https://claude.ai/artifact/Gz5MakpmtDvnHJWZADsyEP>

## Principle: designed for moments, not modules

Desktop is the workstation: planning, files, editing, customisation. Its
structure follows the domain (timetable, lectures, catalog, web) because the
user sits down to work in one of them.

The phone is for short looks between other things. Its structure follows the
moments when someone picks it up, not the modules or read models behind them.
Most opens should answer one question within a second, and the question
changes through the day:

| Moment | Question |
|---|---|
| Before class / between classes | Where is my next class and when does it start? |
| In class | What material is this class using right now? |
| After classes | What is due tonight, and what is first tomorrow? |
| No classes (weekend, holiday, break) | What is due, and what changed? |

Priorities, in order:

1. **Answer without opening the app**: widgets, Lock Screen, notifications.
2. **Answer within a second of opening**: render the cached read model at once,
   refresh silently in the background. No spinner in front of content.
3. **Go deeper**: one push or sheet away, limited to high-frequency actions.

Read models and intents are how a screen is implemented, not how it is
designed. A screen must still be expressible as "subscribes to X, sends Y"
(see [Core vs shell](#core-vs-shell)); if it is not, it is hiding a decision in
the shell.

## Look: native frame, own content

On the phone, looking professional means behaving exactly like the platform.
Desktop earns its look through density and tool-like precision; a phone app
that repaints system chrome reads as a web page pretending to be an app.

- **System frame, unchanged.** Large titles, inset grouped lists, the system
  tab bar (Liquid Glass on iOS 26, Material 3 navigation bar on Android),
  sheets, back gestures, switches, pickers, search, dark mode, Dynamic Type.
  Never draw these ourselves. Only the neutrals behind lists come from our
  tokens (see [One token source](#one-token-source)).
- **Own identity in the content only.** The pieces that make it recognisably
  IsctCompanion are the ones the user reads:
  - course colours,
  - time and room typography (large tabular figures, room right behind time),
  - the deadline chip,
  - the hero card on Today,
  - the sync ring.
- **App tint** is the campus blue accent (`#2f5fd0`). Semantic pairs
  (ok / warn / danger with soft backgrounds) are the desktop ones.
- **No left-border cards, no count badges, no explanatory footnotes.** Urgency
  shows through ordering, grouping and one chip per row.
- **Polish the content components, with the platform's own tools.** The pieces
  above are built once under `Design/` and carry the app's finish: numeric
  text transitions on countdowns, sensory feedback on swipe actions, symbol
  effects on the sync ring, zoom transitions from a block or card into its
  course, mesh-gradient course tints on the hero card (the shared 'now'
  gradient, `design/README.md`), Liquid Glass on our own
  floating controls (iOS 26, a material below). Pow supplies the few
  celebratory effects the platform lacks. Everything respects Reduce Motion.
  iOS 18 is the minimum.

## Surfaces outside the app

### Widgets

| Size | Content |
|---|---|
| Small · next class | Time, room, course, countdown, on the course tint |
| Small · deadline | Nearest deadline as a countdown, title, course |
| Medium | Next class (tinted) + rest of today (time, room) + nearest deadline |
| Lock Screen inline | `10:45 W9-202` next to the date |
| Lock Screen rectangular | Next class, or nearest deadline countdown |

Android uses Glance with the same layouts. Tapping a widget item opens the
matching screen (class → course detail, deadline → its submission page).

### Notifications

All local; no push server.

| Kind | Body | Long-press actions |
|---|---|---|
| Class reminder (5/10/15/30 min lead) | `10:45 機械学習基礎` / `W9-202 · starts in 15 min` | Open course |
| Deadline | `演習 3 提出 · 今日 23:59` / course · status · countdown | Open submission page · Hide |
| New grade, material or announcement | What changed and where | Open course |

"Hide" is the same intent as hiding an assignment in the app. There is no
local "mark done": submission state comes from the LMS.

### Maps

- Not shipped yet. A text search for the building code (`東京科学大学 M`)
  rarely lands on the right building, so the map button is held back until
  Core has a building table (code → campus, coordinates) covering Ookayama,
  Suzukakedai, Tamachi and Yushima. Shells then open the coordinates in
  Apple Maps / Google Maps; MapKit or a navigation API for walking
  directions comes after that.

### Deferred

- Live Activities / Dynamic Island countdown: starting one while the app is in
  the background needs a push server. Revisit if one exists.

## Devices and layouts

One page set, two containers, chosen by the window's size alone: stacked
(single column) up to 600 pt wide or in portrait up to 1100 pt, otherwise the
workstation, desktop's layout. On an iPad that is upright versus landscape.
Telling iPhone from iPad is the one allowed runtime platform check (AGENTS.md
rule 3), and it only adds 授業 to the tab bar.

| | iPhone | iPad, stacked (upright) | Workstation (iPad landscape) |
|---|---|---|---|
| Container | Tab bar + stack | Tab bar + stack | Desktop's rail + page + school status bar |
| Pages | 今日 · 時間割 · ウェブ · 設定 | 今日 · 時間割 · **授業** · ウェブ · 設定 | Rail: 今日 · 時間割 · 授業 · ウェブ, 設定 at its foot (⌘1–4, ⌘,) |
| Courses | Pushed from any block, card or row | 授業 tab: course list → course page | 今日 opens courses and assignments beside itself (desktop's TodayWorkbench); 授業: course list column + course page |
| Settings | Last tab | Last tab | Rail foot |
| Workstation | — | — | File tray bottom right (phase 3) |

The same page views fill every container; only the container changes, and
今日 on the workstation. With nothing open there it is a two-column
dashboard: the header across the top, the moment on the left (the class
cards laid out rather than stacked, the first in full and the next ones side
by side, a day said only where it changes; then exams and 新着) and the
assignments in their own column on the right, each column scrolling on its
own. Upright on an iPad, 今日 stays the iPhone's single column. On
the workstation, what 今日's column points at (a class, an assignment, a
submission page, すべて) opens in a pane beside it while 今日 narrows to a
column; pages opened inside the pane push inside it, and opening the same
row again or Close returns to 今日 alone. Turning back upright pushes what was
open onto 今日's tab. The status bar names each school site with its sign-in
dot and last update, and opens the connection sheet.

## App structure

Everything not in the table above is pushed or presented from a page.

| Page | Content |
|---|---|
| **今日 Today** | One hero card chosen by the time of day, then at most two short sections (see below). |
| **時間割 Timetable** | A dated week grid with a quarter picker (see below). Tap a block → course page. |
| **授業 Lectures** (iPad) | The academic year (menu) and quarter (1Q–4Q) above a 授業 / 課題 switch, as desktop's LectureDesk: the quarter's courses, or all its assignments in the record's groups. On the workstation the chosen course or assignment opens on the right. |
| **課題 Assignments** | Open assignments grouped by deadline: overdue, today, this week, later. Submitted and hidden ones sit behind one row at the bottom. |
| **ウェブ Web** | School sites in one resident web view; the site picker sits behind a long press on the tab (see [School web](#school-web)). |

| Pushed / presented | From | Content |
|---|---|---|
| Course page (push, zoom) | Any class or assignment row, timetable block, 授業 | Header (room, LMS and syllabus links, the course's exams), then the LMS content one lesson at a time (see [Course page](#course-page)) |
| School web (push, full screen) | Course page, assignment, notification | The page fills the screen; back returns to where it was opened from |
| Settings (pushed pages) | 設定 tab; the rail foot on the workstation | The desktop settings sections that apply on a phone, under the desktop's keys so cloud sync keeps both ends equal: school accounts (with matrix card import), notifications (class, exam and assignment reminders), 学習プロフィール, データ更新 (cellular, last pull per site, pull now), ウェブ (text size, first site; device-local), バックアップ (iCloud, earlier versions, recovery files, backup file), 情報. |

Navigation follows the platform: stack pushes with edge-swipe back (Android
back), sheets for settings-like content, pull to refresh as a fallback only.
Tapping the current tab returns to its root.

### Today

Core returns the phase and the content for it; the shell renders it in fixed
order and never sorts, filters or decides what is due.

| Phase | Hero card | Then |
|---|---|---|
| Before first class / between classes | Next class: start time and room in large figures, course, countdown chip. Course tint. | Rest of today (time, course, room) · Nearest deadlines |
| In class | Current class: minutes left, progress bar, end time, room. Course tint. | This class: newest material, open assignment · Next class · Deadlines due today |
| After classes | Nearest deadline tonight: time, title, course, countdown, "open submission page". Danger tint. | Tomorrow's first class · Other deadlines |
| No classes | — | Deadlines · New (grades, materials since last open; すべて既読 on its label marks every new item seen) · Next class card |

The title area shows the date and the day status (normal teaching day,
holiday, make-up day). The connection state is silent when healthy (see
[Sync feedback](#sync-feedback)).

First launch shows one welcome sheet, as desktop does: the app language,
the course and syllabus language (people did not know they could differ), and
one button to the school account; closing it leaves Today with its sign-in
card. After that, TipKit shows a one-time tip beside each gesture nobody can
see (long press a timetable block or lesson chip, swipe an assignment, switch
web sites). Settings › About › Show welcome and tips again brings both back;
TipKit forgets shown tips at the next launch.

### Timetable

- A dated week, not a quarter: the title is the date range with
  "2026 年度 · 3Q 第 2 週" above it as an accent capsule on its own row, free
  to take the full width (it never shares a column with the range), the cue
  that the title block opens the picker. Core's `ScheduleView` covers any
  range and every term in it.
- Weeks change by horizontal swipe on the grid; the only button is 今日 beside
  the range, dimmed on the current week, which doubles as "you are here". A
  one-time tip says so.
- Long press on the 時間割 tab exports the shown quarter as an .ics (courses
  only, or with personal plans) through the share sheet, the same panel as the
  ウェブ tab's site picker; the workstation's rail row offers it as a context menu.
- Tapping the title opens the time picker: 1Q–4Q chips for the academic year
  (a chip jumps to that quarter's first teaching week, which is how a student
  shows someone their courses), and a month calendar whose weeks are labelled
  with their quarter and week number, breaks and exam periods marked
  (`QuarterRange`, `AcademicTermForDate`). "今日に戻る" returns.
- Monday to Friday × periods 1–10, on Core's `timetable_axis` (every period
  one row). A block shows course name and room on the course tint; the title
  takes as many lines as the block's height allows and the room line shows the
  first room only. Today's column is outlined and carries the now line, the
  current block has a 2 pt border in the course colour, a holiday column is
  hatched and its blocks fade.
- An exam block has a warn-coloured border and 試験 before its room; a
  cancelled block fades, strikes its title and says 休講.
- Long press on a class or exam block marks it (`MarkSession`): 試験にする,
  休講にする, 通常の授業に戻す, the current state ticked. Plan blocks and
  personal events have no marks.
- Registration states match desktop (`product-definition.md`): registered and
  declared are filled, planned courses are an outline only. Core marks a
  block `planned` against the block's own quarter, so a week in another
  quarter than today's reads correctly.
- Intensive courses sit in one row under the grid.
- Otherwise read-only. Adding, removing and planning courses stays on desktop.

### Course page

- Header: title, "code · day periods · room", the LMS and シラバス chips, and
  one warn-coloured line per exam of the course from `CourseDetail.exams`
  ("試験 · 11/27 5–6限 · W9-202"; a cancelled exam struck through).
- The LMS content one lesson at a time, as the desktop Lecture Desk lays it
  out: lesson chips (一般, 1, 2, …) and the lesson's text, files, pages,
  labels, announcements and assignments from Core's `CourseContent`.
- Lessons turn by horizontal swipe through the shared `HorizontalPager`: the
  content follows the finger for a third of the width, then rubber-bands; a
  pull past a third or a flick commits; the page moves as one piece toward the
  side the finger went. Drags that start within 24 pt of the leading edge are
  left to the system back gesture. LMS text is rendered before a lesson is
  shown, so a page arrives at its final height.
- The chip with a dot under it is Core's current lesson (`currentSection`,
  which honours the desktop's double-click mark in `lessonLog`). Long press on
  a chip offers "この回を M/D の授業にする" for the class at `classAt`; it sends
  `RecordLesson`, the same intent as the desktop.

### Assignments

- Rows: course colour square, title, course and deadline, one countdown chip;
  no disclosure chevron, the row itself is the button.
- Swipe right: **Pin** (a full swipe pins); swipe left: **Hide**, which needs
  a tap on the revealed button so a long pull cannot hide by accident. Tap
  opens the assignment page; long press offers open course, LMS で提出, pin and
  hide. Hidden assignments come back from the bottom row.
- Only open assignments by default; submitted and graded ones are one tap away.

### School web

- Two entries, one web view: the **ウェブ** tab (Portal, LMS, Kyomu, Mail,
  Library, TSUBAME; each site's last page restored), and pushes from context (course →
  its LMS page, assignment → its submission page), which return to the opener.
- The tab draws nothing over the page. Its site picker is a long press on the
  ウェブ tab item (`TabBarLongPress`): a glass panel rises above the tab bar
  with the six sites, 再読み込み and ブラウザで開く; the finger can slide to a
  row and lift to choose it, or lift first and tap. Tapping the tab again
  returns the shown site to its start page; WKWebView's own edge swipes go back
  and forward; text size lives in Settings. The workstation's rail row offers the
  same rows as a context menu.
- A pushed page keeps its glass controls: back to the opener, page title, text
  size; back, forward, share, open in browser.
- Sign-in is driven by core through `SchoolBrowser`; the web view and
  background pulls share one session. Each site's page stays resident;
  returning restores it without reloading. Authentication callbacks are never
  restored.
- Downloads open in Quick Look or go to Files through the share sheet.
- Default text zoom above 100% (start at 115%), set in Settings › ウェブ
  (100 / 115 / 130 %), kept on the device.
- Pages that need a desktop show one inline "better on a computer · open in
  browser" notice, never a dialog.

### Not on the phone

Stays on desktop: course planner and adding courses, note editor, custom
CSS/JS, advanced catalog search, academic calendar editing, developer
settings. LMS course content is readable on every device. Assignment
workspaces come to the iPad in phase 3: files shared in (for example from
GoodNotes) land in a tray, are dragged onto an assignment and uploaded one by
one or as a ZIP through Core's `workspaces` and `prepare_submission`; the
iPhone picks the assignment in a sheet instead of dragging.

## Gestures

One gesture has one meaning across the app, and only platform gestures are
used.

| Gesture | Meaning |
|---|---|
| Tap | Open |
| Long press on a row | Quick actions (at most 4): open course, pin, hide |
| Long press on a timetable block | Mark: exam, cancelled, normal class |
| Long press on a lesson chip | Mark this lesson as the class's lesson |
| Long press on the ウェブ tab | Site picker; slide to a row and lift |
| Long press on the 時間割 tab | Export the shown quarter as .ics; slide to a row and lift |
| Horizontal swipe on content | Change week (timetable), change lesson (course page); drags from the leading 24 pt are the system back gesture |
| Row swipe | Row actions (assignment: right pin, left hide; hide needs a tap) |
| Pull down | Manual pull |
| Swipe down on a sheet | Dismiss |

Not used: dragging timetable blocks, double tap, multi-finger gestures.
Android row swipes do not start within 20 pt of the left edge, to keep the
system back gesture; verify in the Compose shell.

A gesture is only an entry point: long-press "Hide" and swipe "Hide" send the
same intent to core, and undo comes from the mutation result core returns.

## Sync feedback

Core's sync snapshot (`SyncSnapshot`, see [`../school-refresh.md`](../school-refresh.md)),
the same value the desktop footer renders, is mapped to three levels; the UI
keeps no busy flags or progress estimates of its own.

| Level | Snapshot | Display |
|---|---|---|
| Quiet | No working run | Nothing. Settings shows each source's "updated 12:40". |
| In progress | A working run | A ring beside the settings button only when the run has a `total`; a small spinner otherwise and for background-only work. |
| Needs you | A run with `failure` or a foreground run `held` | One banner at the top of Today naming the dataset, when Core retries, and one action (Retry sends `Refresh` for its scope; sign-in opens the account). |

Banner actions by `CoreError`: `NeedsSignIn` → sign in; `Network` → retry or
"offline, showing 12:40 data"; `SiteChanged` → open in web, noting how many
pages were read; `Busy` (OCR) → "still reading the previous image". Shells do
not rewrite the copy.

## Text size

Users reported the old mobile text as too small (body 10.5–12.5 px in
`src/pages/mobile/mobile.css`).

- iOS: Dynamic Type text styles; body follows the system size (17 pt default).
- Android: `sp` units, honouring the system font scale.
- Times and rooms are the largest text on a row; they are what people look
  for.
- Layouts must survive the largest accessibility sizes: rows wrap, nothing is
  clipped. Timetable blocks clamp the name and keep the room.

## One product across desktop and phone

Shared content language, platform-native frame.

### Course colours are identical everywhere

The strongest recognition cue: a course is the same colour on the desktop
timetable, the phone timetable, widgets and notifications. `displayColor`
(`src/core/course-colors.ts`) is a product rule and moves into core, so every
shell asks core instead of re-implementing the hash. Phone surfaces use the
colour as a light tint with dark text, and as a small square in rows.

### One token source

`design/` at the repository root holds the shared visual language as W3C
design tokens (see `design/README.md`): neutrals and semantic colours for
light and dark, the course palette and tint strengths, radius by role. Style
Dictionary generates the desktop CSS variables,
core's course palette, and SwiftUI / Compose constants. Platforms override a
role's value (phone radii are larger), never its name.

The phone uses the token neutrals behind its native lists (warm background,
white cells), so both apps share one palette; tab bars, navigation bars,
sheets, controls and system fonts stay the platform's.

### Signature components

Specified once (size, states, gestures, read-model fields), implemented in
React, SwiftUI and Compose. The phone may make them larger.

| Component | Must match |
|---|---|
| Course block | Course colour as a light tint, dark text, name then room |
| Deadline chip | Same colour thresholds and countdown format |
| Time + room | Time in large tabular figures, room next to it, course name secondary |
| Hero card | Phone only; course or danger tint, one primary action |
| Sync ring | Same shape as desktop `PullRing` |

### Type and wording

- System fonts on every platform (desktop web font stack, SF on iOS, Roboto on
  Android; Hiragino / Noto for Japanese) to get native Japanese rendering and
  dynamic sizes. Tabular figures for times and countdowns.
- Same names for the same things. Strings are generated from the i18next
  resources into an iOS String Catalog and Android `strings.xml`; never
  translated twice. If i18n stays in the shells, core exposes stable error
  codes rather than strings.

### Continuity

Something hidden on the phone is hidden on the desktop (cloud sync). Later:
deep links from phone items to the same item on desktop.

## Core vs shell

| Screen / surface | Core read model | Intents |
|---|---|---|
| Today | `today(now)`: phase, hero, sections, day status | Manual pull, hide assignment, open course, open web target |
| Timetable | Week plane (classes, intensive courses, exams, plans) | Change week, open course |
| Assignments | Open assignments grouped by deadline; hidden and submitted lists | Hide, unhide, open submission page |
| Course detail | Course header, room, open assignments, latest materials, links | Open file, open web target |
| School web | Current site, safe restorable URL, opener | Switch site, navigate |
| Settings | Account cards, credential state, last pull, sync state | Sign in, forget credentials, import matrix card, connect cloud |
| Widgets | `glance(now)`: next class, rest of today, nearest deadlines | — |
| Notifications | `notification_plan(now, horizon)`: ordered list with time, title, body, target, actions | Same intents as the app |

| Concern | Core | Shell |
|---|---|---|
| Widget data | `glance(now)` | Write to App Group / Glance state, reload timelines, layout |
| Notifications | `notification_plan(now, horizon)` | Schedule with UNUserNotificationCenter / AlarmManager, apply the platform limit (iOS: 64 pending), permission prompts |
| Today phase | `today(now)` decides the phase and its content | Render in order; re-request on wake and at the phase boundaries core reports |
| Course colour | `display_color(course)` | Render |
| Background refresh | `Core::tick` / `refresh_once` | BGAppRefreshTask / WorkManager; report foreground, network and cellular facts |
| Campus Wi-Fi | Policy | `CampusWifi` port; the row does not exist where there is no port |
| Matrix card OCR | `OcrProgress`, raw bytes in | HEIC → JPEG before OCR |
| School web view | Drives navigation and scripts | Creates, lays out and hides the WebView; hides before switching routes |

Widget extensions run with a small memory budget; they read the snapshot the
app wrote and never load core or the OCR model.

## Open questions

1. Phase boundaries: whether `today(now)` returns the next boundary time so the
   shell can schedule a redraw, or the shell re-requests on a fixed cadence.
2. "Recent" in the school site list depends on core recording safe URLs.
3. Error copy ownership: localised in core, or error codes localised by the
   shells.
4. Row swipe vs Android system back gesture; verify in Compose.
