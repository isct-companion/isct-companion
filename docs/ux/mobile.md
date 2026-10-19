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
  course, mesh-gradient course tints on the hero card, Liquid Glass on our own
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

One page set, two containers. The layout follows the desktop rule in
`apps/desktop/src/app/form-factor.ts`: stacked (single column) up to 600 pt
wide or in portrait up to 1100 pt, otherwise split. Telling iPhone from iPad is
the one allowed runtime platform check (AGENTS.md rule 3).

| | iPhone | iPad, stacked | iPad, split |
|---|---|---|---|
| Container | Tab bar + stack | Tab bar + stack | Sidebar + content (`NavigationSplitView`) |
| Pages | 今日 · 時間割 · 課題 · ウェブ | 今日 · 時間割 · **授業** · 課題 · ウェブ | Sidebar: the same five, then 設定 |
| Courses | Pushed from any block, card or row | 授業 tab: course list → course page | 授業: course list column + course page |
| Settings | Pushed pages from the gear | Pushed pages from the gear | Sidebar item |
| Workstation | — | — | File tray bottom right (phase 3) |

The same page views fill every container; only the container changes.

## App structure

Everything not in the table above is pushed or presented from a page.

| Page | Content |
|---|---|
| **今日 Today** | One hero card chosen by the time of day, then at most two short sections (see below). |
| **時間割 Timetable** | A dated week grid with a quarter picker (see below). Tap a block → course page. |
| **授業 Lectures** (iPad) | Courses of the shown quarter; the course page on the right in split. |
| **課題 Assignments** | Open assignments grouped by deadline: overdue, today, this week, later. Submitted and hidden ones sit behind one row at the bottom. |
| **ウェブ Web** | School sites in one resident web view, a site switcher on top (see [School web](#school-web)). |

| Pushed / presented | From | Content |
|---|---|---|
| Course page (push, zoom) | Any class or assignment row, timetable block, 授業 | 概要 (room, links), 資料 (LMS sections and files, opened in Quick Look), 課題 — the desktop course drawer's tabs |
| School web (push, full screen) | Course page, assignment, notification | The page fills the screen; back returns to where it was opened from |
| Settings (pushed pages) | Gear on the Today title; sidebar in split | The desktop settings sections that apply on a phone: school accounts, notifications, study profile, matrix card import, iCloud sync |

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
| No classes | — | Deadlines · New (grades, materials since last open) · Next class card |

The title area shows the date and the day status (normal teaching day,
holiday, make-up day). The connection state is silent when healthy (see
[Sync feedback](#sync-feedback)).

First launch has no onboarding screen: Today opens empty with one sign-in card.

### Timetable

- A dated week, not a quarter: the title is the date range with
  "2026 · 3Q 第 2 週" above it; horizontal swipe between weeks. Core's
  `ScheduleView` covers any range and every term in it.
- Tapping the title opens the time picker: 1Q–4Q chips for the academic year
  (a chip jumps to that quarter's first teaching week, which is how a student
  shows someone their courses), and a month calendar whose weeks are labelled
  with their quarter and week number, breaks and exam periods marked
  (`QuarterRange`, `AcademicTermForDate`). "今日に戻る" returns.
- Monday to Friday × periods 1–10. A block shows course name and room on the
  course tint. Today's column is outlined, the current block has a 2 pt border
  in the course colour, a holiday column is hatched and its blocks fade.
- Registration states match desktop (`product-definition.md`): registered and
  declared are filled, planned courses are an outline only.
- Intensive courses sit in one row under the grid.
- Read-only. Adding, removing and planning courses stays on desktop.

### Assignments

- Rows: course colour square, title, course and deadline, one countdown chip.
- Swipe right: **Pin**; swipe left: **Hide** (the same pair as the desktop
  touch rows). Tap opens the submission page. Long press offers pin, hide and
  open course. Hidden assignments come back from the bottom row.
- Only open assignments by default; submitted and graded ones are one tap away.

### School web

- Two entries, one web view: the **ウェブ** tab (site switcher for Portal,
  T2SCHOLA, Kyomu, Mail, Library; each site's last page restored), and pushes
  from context (course → its T2SCHOLA page, assignment → its submission page),
  which return to the opener.
- The page fills the whole screen, under the status bar and toolbars. Controls
  float over it in glass: back to the opener, page title, text size; back,
  forward, share, open in browser. WKWebView back/forward swipe gestures on.
- Sign-in is driven by core through `SchoolBrowser`; the web view and
  background pulls share one session. Each site's page stays resident;
  returning restores it without reloading. Authentication callbacks are never
  restored.
- Downloads open in Quick Look or go to Files through the share sheet.
- Default text zoom above 100% (start at 115%), adjustable per site with the
  existing school web zoom setting.
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
| Long press | Quick actions (at most 4): open course, pin, hide |
| Horizontal swipe on content | Change week (timetable) |
| Row swipe | Row actions (assignment: right pin, left hide) |
| Pull down | Manual pull |
| Swipe down on a sheet | Dismiss |

Not used: dragging timetable blocks, double tap, multi-finger gestures.
Android row swipes do not start within 20 pt of the left edge, to keep the
system back gesture; verify in the Compose shell.

A gesture is only an entry point: long-press "Hide" and swipe "Hide" send the
same intent to core, and undo comes from the mutation result core returns.

## Sync feedback

The pull state machine (`PullState`, 11 states) is mapped to three levels by
Core's school status (`crates/isct-core/src/school_status.rs`), the read model
the desktop footer also renders; the UI keeps no busy flags of its own.

| Level | States | Display |
|---|---|---|
| Quiet | Succeeded · Queued · Checking | Nothing. Settings shows "updated 12:40". |
| In progress | Running · Applying · Yielding | Sync ring beside the settings button, from `SchoolPullProgress` completed/total. Cancel disappears after Applying. |
| Needs you | Failed · Blocked · RetryWaiting | One banner at the top of Today with one action. |

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
Dictionary generates the desktop CSS variables and Mantine theme values,
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
