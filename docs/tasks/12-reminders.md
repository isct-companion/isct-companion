# Task 12 — Reminders and auto refresh

Depends on tasks 1, 4, 6, 10. Follow the rules in [README](README.md).

Today the Notifications toggles (`settings.notifications.classes`, `.assignments`) are saved but nothing reads them; the only call to the native `notify` command is the test message after granting permission. The `background-sync` capability is hard-coded off (`background_schedule: false` in `backend/src/lib.rs`) and its hint points to an Advanced control that does not exist.

## Build

- **Class reminders:** a local notification N minutes before each class occurrence of the selected term. Occurrences come from the same timetable resolution Today uses (academic calendar exceptions, replacement days, breaks, intensive dates). Default 10 minutes; the lead time is a select on the Notifications page (5 / 10 / 15 / 30).
- **Assignment reminders:** one notification 24 h before and one 1 h before `dueAt` for assignments that are not submitted, not hidden and not deleted. Lead times are fixed for now.
- **Scheduling:** runs while the app (or its tray process, if present) is running. Recompute the next due notifications when the workspace changes and on wake from sleep; never notify twice for the same occurrence (persist the last fired key per item, e.g. `class:{courseId}:{date}:{period}`). Missed reminders older than 5 minutes are skipped, not delivered late.
- **Delivery:** desktop through the existing `notify` command; browser preview through the Web Notification API; mobile through the platform notification plugin when available. Honour `localPermission`; when it is not `granted`, schedule nothing.
- **Auto refresh:** add a Sync setting `Auto refresh: Off / On launch / Hourly` that creates or updates the built-in `school-refresh` scheduled task (it already runs in `createScheduledTaskRuntime`). Report the `background-sync` capability as ready when that task is enabled; remove the "Choose … in Advanced" reason text.
- **LMS background refresh:** install a separate `LMS background refresh` task, disabled by default, on a four-hour RRULE. It may run only on Wi-Fi while the app is active. Course-level Refresh in Today must pass the Moodle course ID and fetch deep content for that course only; a scheduled/manual background run remains the explicit full refresh.
- Remove the `remote-push` capability row from Notifications (keep the capability id for Diagnostics).

## Out of scope

Styling of the Notifications and Sync pages (done separately). Push from a server.

## Acceptance

- With class reminders on and permission granted, a class starting in 11 minutes produces exactly one notification at the 10-minute mark; a cancelled day (calendar exception) produces none.
- Toggling assignment reminders off cancels pending ones without a restart.
- Unit tests for occurrence → reminder computation (including replacement days and quarter boundaries) and de-duplication.
- `Auto refresh: Hourly` runs `school-refresh` once per hour while the app is open and records the result in Scheduled tasks.
