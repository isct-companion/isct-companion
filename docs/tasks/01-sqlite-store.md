# Task 1 — SQLite store, export/import

## Goal

Replace every local store with one SQLite database owned by the Rust backend, and make cloud sync an export/import of that database. Schema version 3, created from scratch; old data is discarded.

## Today

- `src/storage/repository.ts`: `TauriStoreRepository` keeps the whole `Workspace` (`src/core/model.ts`, `schemaVersion: 2`) in `workspace.json` via `tauri-plugin-store`; `LocalRepository` (localStorage) serves the browser preview/PWA.
- `src/store/workspace.ts`: the UI edits a whole `Workspace` draft (`workspaceStore.update(fn)`) and saves it through the repository.
- Caches: `src/cache/*` (localStorage, 30 entries), `src/core/syllabus-content.ts`, and `school-cache.json` written by `backend/src/school_extract.rs`.
- Cloud: `backend/src/cloud.rs` + `src/connectors/providers/transport.ts` provide OAuth and provider transport. Task 15 replaces the old encrypted two-file draft with one plaintext JSON file.

## Build

1. **Database.** Add `rusqlite` with the `bundled` feature. Open `school.db` in `app_data_dir` at startup (WAL, foreign keys on). A `migrations` module creates schema v3 in one transaction; if a different version is found, drop everything and recreate (development reset policy). Delete `workspace.json` and `school-cache.json` on first v3 launch.
2. **Tables** (column names snake_case; JSON columns hold small nested values such as `Localized`, meetings, credit splits):

   | Table | Key | Columns |
   |---|---|---|
   | `timetable_courses` | `id` | `term`, `offering_id?`, `names` (JSON: `{title: Localized, instructor: Localized}`), `title?`, `instructor?` (manual courses), `code`, `instruction_language`, `credits`, `credit_split` (JSON), `meetings` (JSON), `color`, `url?`, `source` (JSON) |
   | `course_overrides` | `course_id` | `title?`, `instructor?`, `color?` |
   | `plan_items` | `id` | `term`, `offering_id?`, `course_id?`, `state` (`candidate`/`planned`) |
   | `memberships` | `id` | `term`, `course_id`, `status` (`declared`/`registered`/`lms-member`/`completed`/`withdrawn`), `evidence` (JSON) |
   | `personal_events`, `calendar_exceptions` | `id` | fields of today's `PersonalEvent` / `CalendarException` |
   | `assignments` | `id` | today's `Assignment` fields plus `lms_cmid?`, `lms_course_id?`, `submission` (`unknown`/`open`/`submitted`/`graded`) |
   | `assignment_marks` | `assignment_id` | `hidden`, `completed_at?` |
   | `settings` | `key` | `value` (JSON) — today's `Settings`, `academicProfile`, `selectedTerm`, `terms` |
   | `offerings` | `id` | see `Offering` in the model doc; `title`, `instructors` as JSON `Localized` |
   | `offering_content` | `offering_id, lang` | `content` (JSON `SyllabusContent`), `fetched_at` |
   | `offerings_fts` | FTS5 | `offering_id`, `code`, `title_ja`, `title_en`, `instructors_ja`, `instructors_en` |
   | `lms_courses`, `lms_items` | `lms_id` / `cmid` | see task 3 |
   | `registrations`, `registration_states`, `grades`, `requirement_progress` | see task 4 | |
   | `import_runs` | `id` | `provider`, `scope`, `state`, `started_at`, `finished_at`, `message` |
   | `intensive_schedules` | `course_id` | `IntensiveSchedule` (model doc): range, periods, per-day overrides, room |
   | `plans` | `id` | `title`, `course_id?`, `room?`, `start` (date or date-time), `duration`, `rrule?` (iCalendar, `FREQ` ≥ `DAILY`), `exdates` (JSON), `reminder?` |
   | `gpa_summary`, `gpa_by_term` | `scope` / `year, term` | see task 4 |
   | `subscriptions` | `id` | `url`, `kind` (`calendar` / `departments`), `enabled`, `hash`, `fetched_at`, `last_error?` |
   | `academic_calendar` | `date` | `type` (`holiday` / `no-class` / `replacement` / `quarter-start` / `quarter-end`), `replacement_day?`, `label` (`Localized`), `source_hash` |
   | `departments`, `recommended_courses` | `id` / `department_id, code, group` | from the departments subscription (model doc "Departments") |
   | `scheduled_tasks` | `id` | `name`, `action` (`school-refresh` / `export` / `hook`), `hook_id?`, `schedule` (`launch` / RRULE), `enabled`, `last_run_at?`, `last_result?` |

   Every row except `offering_content`, `offerings_fts` and `import_runs` carries the versioned columns from step 3 plus `deleted_at?`. The person's own tables are `timetable_courses`, `course_overrides`, `plan_items`, `memberships`, `personal_events`, `calendar_exceptions`, `assignment_marks`, `intensive_schedules`, `plans`, `subscriptions`, `scheduled_tasks`, `settings`.
3. **Versioned data.** One Rust module (mirrored as TS types) used by every table, import and subscription:

   ```ts
   type Versioned<T> = { value: T; hash: string; updatedAt: string; updatedBy: string; rev: number; source?: string };
   diff(a: Versioned<T>, b: Versioned<T>): FieldChange[];            // field-level, stable key order
   update(current, incoming, method: "use-latest" | "use-current"): { result; changed: boolean; conflict?: FieldChange[] };
   ```

   - `hash` = SHA-256 of canonical JSON (sorted keys, no whitespace, numbers normalized) of `value` only.
   - School imports: same hash → no write (no `rev` bump, no change notification); different hash → write and record the change (e.g. "syllabus updated").
   - Person's edits bump `rev`. Import/merge: equal hash → nothing; otherwise `use-latest` compares `updatedAt`, `use-current` keeps local; `resolve-manually` (existing setting) returns the diff for the UI.
   - Stored as columns (`hash`, `updated_at`, `updated_by`, `rev`, `source`) on every row, not as a wrapper blob.
4. **Read model for the UI.** Keep the UI's whole-workspace editing for now: `load_workspace` assembles the v3 `Workspace` from the person's tables (plus referenced offerings for display); `save_workspace(workspace)` diffs against the database and upserts only rows whose `rev` changed, tombstoning removed ones. Replace `TauriStoreRepository` with a `SqliteRepository` calling these commands. `LocalRepository` stays for the browser preview.
5. **Model v3** (`src/core/model.ts`): apply "Person's own tables" from the model doc — `Course.offeringId`, `Course.names`, `instructionLanguage`, `creditSplit`; evidence-based `Membership.status`; `DraftSelection` → `PlanItem` with `state`; drop `syllabusSource`. Delete `src/core/course-language.ts` and render names with a `pickLocalized(value, locale)` helper (`zh` → `en` first, then the other language). Rewrite `seedWorkspace` for v3.
6. **Export/import.** The current `export_data()` document uses schema version 6, matching the SQLite schema generation, and contains the person's tables, the personal school tables, and only the `offerings` rows they reference. Caches (`offering_content`, `offerings_fts`, `import_runs`) are excluded. `import_data(json)` validates the whole document first, then merges row by row in one transaction through `update()` above, using the conflict setting (`use-current` / `use-latest` / `resolve-manually`). Point the cloud transport at these two commands. Task 15 defines the current plaintext cloud file format. The PWA imports the same document read-only into `LocalRepository`.
7. **Catalog commands** used by later tasks: `catalog_upsert_offerings`, `catalog_search(query, filters)` (FTS over both languages plus day/period/quarter filters), `catalog_offering(id)`.

## Acceptance

- Rust tests: canonical hash stability (key order, whitespace), `diff`, both `update` methods, schema creation, version mismatch → rebuild, `save_workspace` diff (only changed rows written, tombstones), export excludes caches and unreferenced offerings, import merge rules (higher rev, conflicts, tombstones), FTS finds a course by Japanese and by English title.
- TS tests: v3 model helpers, `pickLocalized` fallback order, repository contract.
- Existing tests updated to v3; `tsc -b`, `vitest run`, `cargo test` pass.
