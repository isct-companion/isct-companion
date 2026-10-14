# Tasks

What to build: [docs/product-definition.md](../product-definition.md). Data contract: [docs/course-data-model.md](../course-data-model.md). Read both first.

## Order

| #   | Task                                                                        | Depends on  | Area        |
| --- | --------------------------------------------------------------------------- | ----------- | ----------- |
| 1   | [SQLite store, versioned data, export/import](01-sqlite-store.md)           | —           | Data        |
| 2   | [Syllabus catalog, both languages, department sync](02-syllabus-catalog.md) | 1           | Data        |
| 3   | [LMS sync through Moodle AJAX](03-lms-sync.md)                              | 1           | Data        |
| 4   | [教務Web registration, grades, GPA, requirements](04-kyomu-import.md)       | 1           | Data        |
| 5   | [Subscriptions: academic calendar, departments](05-subscriptions.md)        | 1           | Data        |
| 6   | [Today (with tasks)](06-today.md)                                           | 1, 3, 4, 5  | UI          |
| 7   | [Courses: list, detail, intensive dates, ICS](07-courses.md)                | 1–4         | UI          |
| 8   | [Course planner](08-course-planner.md)                                      | 1, 2, 4, 5  | UI          |
| 9   | [Plans (one-off and repeating)](09-plans.md)                                | 1           | UI          |
| 10  | [Settings](10-settings.md)                                                  | 1, 5, 6     | UI          |
| 11  | [Agent access (MCP server)](11-mcp-server.md)                               | 1–5, 9      | Integration |
| 12  | [Reminders and auto refresh](12-reminders.md)                               | 1, 4, 6, 10 | Feature     |
| 13  | [Web toolbar: forward, CSS/JS extensions](13-web-extensions.md)             | 10          | Feature     |
| 14  | [Subscriptions: important dates, exams](14-important-dates-exams.md)        | 5           | Data        |
| 15  | [Native cloud workspace and credential sync](15-cloud-sync.md)              | 1, 10       | Integration |
| 16  | [LMS files through an app URI scheme](16-lms-file-scheme.md)                | 3           | Data        |

Task 1 first. Then 2–5 in parallel. Then 6–10 (7 and 9 share ICS export: do 9 before finishing 7's export). 11 and 15 build on those foundations.

## Rules for every task

- **Read-only toward the school.** Only `GET` page loads, plus the Moodle AJAX functions listed in task 3. Never submit a form, never call `__doPostBack`, never press a registration, delete, language or filter control. 教務Web's `$.ajax` keep-alive (it re-posts the whole form) must never be called.
- **No HTML in the UI.** Parsers live in Rust (`scraper` crate, CSS selectors) or in fixed app-owned extractor scripts evaluated in the school webview; they return typed records. Regex only for leaf values (dates, periods, credit splits), never to find document structure.
- **Match on markup, not text.** Selectors use ids, classes, `data-*` and `href` patterns. Visible text is matched only where a page has no other structure; those places are listed explicitly in the task.
- **Only reliable structure.** Show structured fields that parse on every page; free text (e.g. evaluation methods) is displayed as written, never mined for numbers.
- **Both languages.** Every school text is `Localized = { ja?: string; en?: string }`; see "Bilingual rule" in the model doc.
- **Versioned writes.** Every stored record goes through task 1's `Versioned` update; unchanged content (same hash) is not rewritten.
- **Failure keeps data.** A login page, a changed layout or an empty result never deletes rows; follow "Validation and reconciliation" in `docs/data-architecture.md`.
- **No personal data in fixtures or logs.** Fixtures are synthetic; ids and course codes are fine, names and grades must be invented.
- **UI:** Mantine components, pages fit the window without page scroll, minimal text (see `docs/product-definition.md` → Visual thesis).
- **Login automation is off limits:** do not change `crates/isct-core/src/school_browser/assets/school-login.js` or the sign-in logic in `crates/isct-core/src/school_browser/engine.rs` (login scripts, session decisions, relay handling, auto sign-in). Adding extractor injection to its `page_loaded` hook is expected.

## Tools

- `scripts/research/syllabus_sample.py` re-checks the syllabus structure on 100 random offerings (both languages); run it after any syllabus site change and before changing task-2 parsers.
- `docs/tasks/data/` holds hand-transcribed reference data.

## Verify

```bash
cargo test --manifest-path backend/Cargo.toml --locked
./node_modules/.bin/tsc -b
./node_modules/.bin/vitest run
```

(On the original machine the `node`/`yarn` shell functions are broken; call binaries by path as above.)
