# Campus Life for Science Tokyo

Unofficial student companion for Institute of Science Tokyo.

## Product promise

Open the app and know today's classes, the next room, and what is due. Reach course materials and official school actions without repeating the portal maze.

Campus Life combines MyStudyLife's academic-day priority with TitechApp's school-system access. It organizes official information but does not submit assignments, register courses, or impersonate an official school service.

## Primary jobs

1. Check today's classes and the next classroom.
2. See assignment deadlines and LMS submission status.
3. Download or open course materials without another login.
4. Plan next Quarter's courses on one page: slot, department, recommendations, syllabus outline, conflicts.
5. See registration state, credits, GPA/GPT and requirement progress without opening 教務Web.
6. Reach registration, mail, library and other official services quickly.

## Platforms

- macOS and Windows are the primary full desktop products.
- iOS uses the same Tauri application and data model with a simpler mobile layout; current distribution may require user self-signing.
- Platform differences stay behind small adapters for credential storage, notifications, background work, files and WebView lifecycle.
- The iOS UI does not expose JavaScript or hooks; it may support site CSS.

## Navigation

Desktop primary rail:

1. Today
2. Schedule
3. Courses
4. Web

Settings is a utility action at the bottom. Tasks is part of Today, not a separate destination. Mobile uses the same destinations in compact bottom navigation; Web is full-screen.

### Today

Order:

1. Date and teaching state: ordinary day, holiday, no-class day or replacement day (from the academic-calendar subscription).
2. Today's classes: now, next, then later. Time and room are visually dominant.
3. Assignments: overdue, due today, then due soon.
4. A compact academic summary: GPA, GPT and earned credits (from 教務Web).
5. Exams enter Today seven days before the exam when an exam source is available.

Assignments:

- Official LMS state only: open, submitted, graded, late, or unknown. No local `Mark complete` that can contradict LMS.
- `Hide` removes an assignment from Today on this device and its backups only; it never changes school state. Hidden items are listed in Settings → Hidden items, where they can be restored or cleared.
- Open the parent course or the exact official assignment page. Returning triggers a bounded refresh that shows `Checking…`, then the LMS state and `Last checked`. A failed refresh keeps the last state with its timestamp.

Do not place a full weekly timetable or dashboard-card mosaic on Today.

### Schedule

- Ordinary seven-day week, Monday through Sunday by default.
- Weekday with the date below it; highlight today.
- Previous week, next week and return to this week.
- Courses (including person-set intensive dates), exams and personal plans share one time plane.
- Sunday-first is a small preference. A month calendar is not initial scope.

### Courses

- Open the current Quarter by default; switch to prior Quarters and add future Quarters.
- Show courses for that Quarter as a simple list with credits (lecture-exercise-lab and total).
- Course detail: Overview, Materials, Assignments, Syllabus.
- A planned course is lower contrast with a small `Planned` state. Registration state comes from 教務Web (declared, registered); LMS membership alone is not proof of registration. Official registration stays in 教務Web.
- **Intensive courses** (集中講義) have no weekly slot. The person sets dates: one action for a range with the same periods (e.g. 7/1–7/8, periods 1–6), then optional per-day changes or removals.
- **Export ICS** for a Quarter's courses and plans.

### Course planner

The main course-editing surface, designed to minimise steps. One page:

- Left: the draft timetable of the selected Quarter (registered, declared and planned courses).
- Click an empty or occupied slot: the right side lists offerings that meet in that slot, filtered by the department chosen in Settings (a checkbox removes the filter). Recommended courses from the department's study-guide table are marked with their group and mark (e.g. `A ○`, `R ◎`).
- Selecting an offering shows its syllabus outline in place: only fields that are extracted reliably — format, language, credits, schedule and rooms, class, updated date, competencies, lecture plan — plus the evaluation section as written. No numbers are inferred from free text.
- `Add to plan` / `Remove` act immediately; conflicts are shown on the slot, never blocking. Search by keyword stays available on the same page.
- Local guidance only; it never overrides 教務Web.

### Web

- One persistent primary Web tab with service subtabs for LMS, 教務 Web, Mail, Syllabus, both portals, Library and later services.
- App-to-Web is the initial integration direction: a class, assignment, material or credit action switches to Web, selects the service and navigates to its exact official URL. Do not place a second WebView in a modal or overlay.
- Contextual entry stores the source route and shows one compact return action such as `← Course` or `← Task`. Returning restores the source page and relevant inspector. Opening Web from primary navigation has no contextual return action.
- Browser back/forward remains inside the selected service; the contextual return action belongs to app navigation.
- Web-to-App interception and complex DOM event bridges are deferred.
- Portal, LMS and Syllabus share one resident school WebView. Leaving Web queues its native hide before committing the app route; returning shows the same session again.
- The last service, safe page URL and scroll context may be restored. Authentication callbacks, credentials and unsafe query data are never restored.
- On mobile, known desktop-oriented pages show a compact `Desktop recommended` status and keep `Open in browser` available.
- Do not interrupt navigation with repeated compatibility dialogs. Preserve pinch zoom and horizontal scrolling; use bundled site CSS only for safe readability improvements.
- Show a one-time landscape hint only when the page's primary task is otherwise unusable at the current width.

### Settings

Primary sections:

- General (language, department, admission year)
- Accounts
- Notifications
- Storage & sync (export / import, cloud)
- Subscriptions (academic calendar, recommended courses)
- Hidden items
- Scheduled tasks
- Downloads
- Data
- Advanced (per-site CSS/JS, hooks)

Diagnostics and raw provider details stay out of ordinary settings.

## Accounts and authentication

- Science Tokyo and legacy Tokyo Tech are separate account cards and authentication families.
- Users connect accounts from Settings, not through a forced full-screen startup gate.
- With explicit consent, account credentials, Matrix table and supported reusable OTP configuration are stored only in macOS Keychain, Windows Credential Manager or the corresponding iOS secure store.
- Received one-time codes exist only for the active login.
- Secrets are submitted only to exact allowlisted official authentication forms. They never enter workspace data, logs, exports, hooks, cloud storage or analytics.
- Each account provides `Sign in`, `Reauthenticate`, `Forget credentials` and `Clear session` as appropriate.

## First launch

Use a short setup with labels and actions, not explanatory pages:

1. `Campus Life` / `for Science Tokyo · Unofficial` / `Continue`
2. `Accounts` with `Science Tokyo` and `Tokyo Tech` connect actions / `Continue` / `Skip`
3. `Sync` and `Notifications` optional actions / `Open Today`

Every setup action is optional. An unconnected user enters a real empty Today with a short `Connect account` action. Do not seed fake classes or stale assignments into a real user's workspace.

## Notifications and background work

- Next class: default 15 minutes before, configurable.
- Assignments: the due day is enabled by default. Five-day and three-day reminders are available but default off; submitted work stops reminders.
- Exams: visible in Today from seven days before; exam notifications can be added with the future exam source.
- Rescheduling cancels obsolete notifications and creates current ones without duplicates.
- Background service is optional. It maintains local notification schedules while idle and performs bounded Pulls at configured intervals.
- No continuous polling. The resident school WebView stays hidden outside Web; a Pull uses the same session without creating additional views.
- Closing the main window may leave a low-resource menu-bar/system-tray process when the user enables it.

## Materials and downloads

- Sync metadata first. The first open downloads to temporary cache and previews inside the app.
- Primary action is app preview; secondary actions are `Open in Web` and `Open in browser`.
- PDF, images and common text are previewed in-app. Other files open in a system application.
- `Download` saves a user-owned copy outside disposable cache.
- Settings may enable automatic caching of new files and provides cache size, per-course clearing and clear-all.
- Cached or downloaded teaching files do not sync between devices. Another device downloads them again from the official source.

## User-created content

School data remains primary. Users may quickly add a plan, task, exam or event with a title and a time; course, room and reminder are optional.

- A plan is one-off or repeating. Repetition uses an iCalendar RRULE (so ICS export is exact) with a minimum unit of one day, entered with simple presets (daily, weekdays, weekly on …) or typed as an RRULE.
- Do not build project management, attendee systems or tag taxonomies.

## Data and sync

- All data lives in one local SQLite database owned by the native app.
- Every stored record is versioned: content hash, timestamp, source and revision. The same mechanism detects changed school data, reconciles imports and updates subscriptions, with `use latest` / `use current` (or manual resolution) when both sides changed.
- Cloud sync is export/import of one document, client-side encrypted, to Google Drive, OneDrive or later providers. The application has no account or sync server.
- The export includes the person's own data and their school records (courses, registration, grades, requirement progress, assignments); rebuildable caches (full syllabus text, search index) are excluded.
- Credentials, Matrix/OTP data, cookies, files, raw HTML, WebView state and diagnostic payloads never sync.
- macOS, Windows and iOS may independently sign in to school systems and reconcile through the same export.

## Subscriptions

Yearly reference data comes from a public GitHub repository (to be created), subscribed in Settings:

- Academic calendar: holidays, no-class days, replacement days, quarter dates.
- Departments and their recommended courses, parsed from the official admission-cohort study guide (`guide_{admissionYear}`) tables. They do not switch when the academic calendar advances.

Once a year an automated agent in that repository parses the new official documents into JSON and opens a pull request for human review. The app validates each file against its schema and applies it through the versioned-data update.

## Agent access

Other AI agents can use the app's data through a local MCP server:

- Read: today's classes, assignments, courses and syllabus data, catalog search, registration, credits, GPA/GPT and requirement progress.
- Write: only the person's own data — plans, hidden items, personal events.
- Never: anything that writes to a school system, credentials or session data.

## Language

- UI locales are English (`en`) and Japanese (`ja`). Japanese system locale defaults to Japanese; other locales default to English.
- Fetch both language variants when an official source supports them and retain them side by side with provenance.
- Switching app language changes cached content immediately without changing the WebView site's own language.
- When only one source language exists, show the original. Do not synthesize an authoritative translation.

## Advanced

- Desktop: app CSS, per-site CSS/JavaScript and bounded hooks.
- Scheduled tasks: hooks and built-in jobs (school refresh, export) run on a schedule set in Settings.
- iOS: per-site CSS at most; no user JavaScript or hooks.
- Login pages reject injection by default.
- Site scripts and hooks cannot access credentials, authentication fields, cookies or the mutable internal store.

## Visual thesis

A premium but simple student-life app: warm neutral surfaces, precise typography, restrained Science Tokyo blue, and muted course colours used for recognition rather than decoration.

- Moderate desktop density and calm single-column mobile flows.
- Layout and spacing before cards, borders or shadows.
- Cards only when the object itself is an interaction, especially next class or urgent work.
- Short utility labels. Trust the user; avoid tutorial copy, promotional sentences and repeated explanations.
- Motion only for route changes, WebView transitions and meaningful state updates.

## Deferred

- Exam-site subscription
- grade analytics beyond the GPA/GPT/credit summary
- focus timer and streaks
- AI study coach
- family or school administration features
- Web-to-App interception
- packaged WebView platform QA beyond the defined lifecycle contract
