# Task 9 — Plans (one-off and repeating)

Product spec: `docs/product-definition.md` → User-created content. Depends on task 1.

## Build

- Table `plans` (task 1). A plan has a title, optional course, room and reminder, a start (date, or date + time), a duration, and optionally an `rrule`.
- Repetition is an iCalendar RRULE with a **minimum unit of one day**: allowed `FREQ` values are `DAILY`, `WEEKLY`, `MONTHLY`, `YEARLY`; reject `HOURLY`/`MINUTELY`/`SECONDLY` and `BYHOUR`/`BYMINUTE`/`BYSECOND`. Support `INTERVAL`, `BYDAY`, `COUNT`, `UNTIL`, and `EXDATE`.
- Editor: one-off by default; presets `Every day`, `Weekdays`, `Weekly on …`, `Every 2 weeks`, `Until …`; an advanced field accepts a raw RRULE and shows the next 5 occurrences.
- Use a maintained RRULE library on the Rust side (occurrence expansion for Today/Schedule/notifications) and the same rules in ICS export (task 7).
- Plans appear in Today, Schedule and ICS; they never affect school data.

## Acceptance

- `FREQ=HOURLY` and `BYHOUR` are rejected with a clear message.
- `FREQ=WEEKLY;BYDAY=TU,FR;UNTIL=…` expands correctly across a DST-free Tokyo calendar and respects EXDATE.
- Round trip: plan → ICS → parse gives the same occurrences.
