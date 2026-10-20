# Course data model (course selection)

Status: data-layer contract, 2026-10-06. Cloud sync is intentionally out of
scope; this document distinguishes rebuildable school data from local user
data and overlays.

## What each source gives

| Source            | Identity                                                                                                                     | Language                                                                                                                 | Notes                                                                                                                                                                                                                                                                                                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Syllabus (public) | `externalId` e.g. `202600907`, URL `/courses/{year}/{dept}/{path}/{id}`                                                      | Separate `?hl=ja` / `?hl=en` pages                                                                                       | The 12-field metadata block is identical in structure for every offering (100/100 sampled, both languages). Section `key`s, meeting slots, code, credits are language-neutral; title, instructor, **rooms**, labels, body text differ (`Ishikawadai Bldg. 2 318` / `石川台2号館318`). Language is display text (`日本語` / `Japanese`), credits are a split (`2 0 0`). |
| LMS (Moodle)      | `course.id` (e.g. 11663), **`idnumber` = syllabus id**, `shortname` = `CODE-CLASS[year]`                                     | `fullname` is bilingual in one string: `社会モデリングＢ / Social Modeling B`; UI strings follow the user's LMS language | Read-only AJAX (`core_course_get_enrolled_courses_by_timeline_classification`, `core_calendar_get_action_events_by_timesort`) returns ids and Unix timestamps; paginated by 50. HTML dates are localized text, so do not parse them. Categories encode year/quarter (2025: 2–5, 2026: 8–11, 他: 6/12). Every course has a `シラバス URL` module.                       |
| 教務Web (kyomu)   | Registration cells carry **`data-jwc` = syllabus id** and link to `syllabus…/courses?jwc={id}`; grades have only code + year | Japanese (English mode is a server-side toggle; not used)                                                                | Registration list with requirement letter and mark (`A(○)`, `B(○)`, `C(○)`, `L(選択)`, `R(◎)`), credit split `[0-1-0]`, schedule `3Q：月3-4`, intensive `集中講義等`, whole-list state (一時保存 / 要エラーチェック / 本登録可能 / 承認 / 本登録不可 / 本登録). Grade list and graduation-requirement tables by 科目区分.                                              |

## How each source is read

### Course-code lookup

Course code is the common lookup field for courses, their assignments, and
published exams. An explicitly linked LMS course keeps its full shortname code
without the year, including the numeric class suffix; syllabus base codes stay
search aliases and do not overwrite that suffix. Manual courses stay user-owned.
An exact association uses
`(academic year, quarter, normalized full code)` and returns every candidate;
only a unique enrolled course is linked automatically. An explicit syllabus
offering ID can resolve the same identity directly. Base-code and fuzzy matches
are search candidates, never automatic exam links.

The Core `lectureSearch` query serves Lecture Desk and Today's course finder.
It searches course codes, both title languages, instructors, assignment titles,
and published exam labels/rooms. It normalizes full-width input and punctuation,
accepts reordered search words, and tolerates a single typo in longer Latin
words. Lecture Desk searches its selected quarter; Today's course finder searches
every stored quarter. The course-selection catalog continues using 教務Web.

Persistent course IDs and personal overrides retain their existing provider
identities. Code is a lookup key, rather than a replacement storage ID.

LMS owns the weekly timetable grid. Syllabus details supply course metadata and
rooms for those grid weekdays; an LMS English title alone does not mean the
details have been fetched. Subscription exams retain their source code, dates,
times, rooms and notes, including instructions to confirm an individual room in
LMS when a published room cell lists a shared candidate pool.

Checked against the live sites on 2026-09-28 (read-only calls only).

| Source             | Data                                                                                                               | Access                                                                                 | Notes                                                                                                                                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Syllabus           | Search results                                                                                                     | **HTML** (`GET /search`, `GET /courses/{year}/{scope}/search`)                         | Server-rendered table, at most 100 rows per query, no pagination, no JSON endpoint.                                                                                                                       |
| Syllabus           | Offering detail and content                                                                                        | **HTML** (`GET /courses/…/{id}?hl=en`, and the same URL **without `hl`** for Japanese) | `?hl=ja` answers 302 to the URL without `hl`; follow same-host redirects only. `dl.c-dl-2col` metadata, `h3.c-h3` sections. Fetched once per language.                                                    |
| LMS                | Enrolled courses (id, `idnumber` = syllabus id, `shortname`, bilingual `fullname`, category, dates)                | **API** `core_course_get_enrolled_courses_by_timeline_classification`                  | Paginate with `offset` until `nextoffset` stops advancing.                                                                                                                                                |
| LMS                | Recently visited courses                                                                                           | **API** `core_course_get_recent_courses`                                               | Optional.                                                                                                                                                                                                 |
| LMS                | Course structure: sections and every activity (id, name, module type, URL, visibility)                             | **API** `core_courseformat_get_state`                                                  | Returns a JSON string; replaces HTML scraping of course pages.                                                                                                                                            |
| LMS                | Open work with deadlines (Unix `timesort`, overdue flag)                                                           | **API** `core_calendar_get_action_events_by_timesort`, `…_by_course`                   | Lists only work still to do; an assignment that drops out has been submitted or closed.                                                                                                                   |
| LMS                | All dated events, including submitted work                                                                         | **API** `core_calendar_get_calendar_upcoming_view`, `…_monthly_view`                   | Deadlines of finished work.                                                                                                                                                                               |
| LMS                | Submission detail, assignment description, folder files                                                            | **HTML** (`mod/assign/view.php`, `mod/folder/view.php`)                                | `mod_assign_*`, `core_course_get_contents` and the other web-service functions return `servicenotavailable`. Only fetched on demand.                                                                      |
| LMS                | Grades                                                                                                             | **Not available**                                                                      | `grade/report/user` → 無効なユーザです, `grade/report/overview` → 評点を表示できません. Official grades come from 教務Web.                                                                                |
| LMS                | Dashboard timetable block                                                                                          | HTML, read from the loaded dashboard                                                   | Read all offered year/quarter grids through the dashboard query form; preserve failed quarters and keep non-weekly membership separate from missing data.                                                 |
| 教務Web            | Registration list and state (`PID1_0.aspx`), grades (`PID2_0.aspx`), requirement progress (`PID2_6.aspx`), notices | **HTML only** (`GET`)                                                                  | ASP.NET WebForms with `__doPostBack` and ViewState, no JSON. Its only `$.ajax` is a keep-alive that re-posts the whole form; the app must never call it. Japanese only (English is a server-side toggle). |
| Portal (GetAccess) | Sign-in and service links                                                                                          | HTML                                                                                   | Sign-in only; no data imported.                                                                                                                                                                           |

API calls run inside the signed-in LMS page as `POST /2025/lib/ajax/service.php?sesskey=…` with `M.cfg.sesskey`, the same way the dashboard itself calls them. Only the functions listed above are allowed; the extractor refuses any other method name, and never calls anything that writes.

Join keys, in order of reliability: `syllabusId` (LMS `idnumber` = syllabus `externalId` = 教務Web `data-jwc`), then `year + code + class` (grades, LMS `shortname`). Titles are never used to join.

## Timetable sources

LMS owns the timetable course list and each quarter's weekday/period grid.
First and manual full refreshes read every year and quarter offered by the LMS
filter, including its non-weekly block. Only the dashboard's read-only query
POST (`year`, `quota`, `search`) is used. A successful grid records present
courses, explicit non-weekly courses and absences separately; failed or
unavailable grids keep the previous cache. Ordinary page/quarter changes read
cache; timetable refresh pulls LMS only.

Syllabus supplies course metadata, credits and rooms. Its weekly slots do not
create or replace LMS timetable clocks. 教務Web supplies search, official
registration, final grades and the academic profile; it does not project
calendar membership or clocks. School-side changes appear after LMS synchronizes.
Local courses, personal clocks, overlays and placement tombstones keep priority.
Cross-quarter credits are divided equally in quarter read models while the
original offering total is retained. The switch requires no cache deletion or
user-table rewrite.

## Where data lives

**One SQLite database for structured application data** (`school.db` in the app data dir, owned by the Rust backend). It replaces `workspace.json`, the localStorage caches and `school-cache.json`. The UI reads and writes only through narrow Tauri commands; it never parses HTML and never opens the database itself. Downloaded material bytes and user-authored note files are the explicit filesystem exception; their types, TTL and paths are defined in [course-file-storage.md](course-file-storage.md).

Table groups:

| Group                      | Tables                                                                                                                                          | Ownership and storage                                                                               |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| User records               | `manual_courses`, `manual_memberships`, `plan_items`, `personal_events`, `calendar_exceptions`, `intensive_schedules`, `plans`, `subscriptions` | SQLite; stable UUIDv7, `updatedAt`, soft deletion; included in local backup                         |
| User overlays              | `course_overrides`, `assignment_marks`                                                                                                          | SQLite; reference a school `sourceKey`; independent revision and lifetime; included in local backup |
| Local notes                | Markdown under the course `notes` / `deleted-notes` directories                                                                                 | Local user data; included in local and cloud workspace backups, excluded from cache cleanup         |
| School records and catalog | `timetable_courses`, `memberships`, `registrations`, `grades`, `lms_courses`, `lms_items`, `assignments`, `offerings`, reference tables         | Rebuildable SQLite cache; `lastSeenAt` / `missingSince`; never exported                             |
| Derived caches             | `offering_content`, `catalog_search_items`, `offerings_fts`, `dataset_state`, `sync_runs`, downloaded attachments                              | Rebuildable; never exported                                                                         |

Only user records and overlays use tombstones. School rows use `missingSince`
and automatically become visible again if the source returns them.

Export writes a version 6 user-data backup containing user tables, overlays,
user preferences/profile and Markdown notes. Restore is an explicit replacement.
Cloud storage and secret boundaries are defined in
[data-ownership.md](data-ownership.md).

**Schema** is version 6. Startup migration and development rebuild behavior are
implemented in `crates/isct-core/src/data/migrations.rs`.

## Departments (系)

The person picks a department and admission year in Settings. The official cohort guide (`https://www.titech.ac.jp/guide/guide_{admissionYear}/gakubu1/`) lists 17 undergraduate departments, each with its curriculum PDF (`pdf/27-NN.pdf`, containing the 付表 of recommended courses) and its syllabus scope. Recommendations always use the student's admission-year guide, not the current academic year's guide.

| Department       | Syllabus scope       | Guide PDF |
| ---------------- | -------------------- | --------- |
| 数学系           | `1/0-901-311100-0-0` | `27-01`   |
| 物理学系         | `1/0-901-311200-0-0` | `27-02`   |
| 化学系           | `1/0-901-311300-0-0` | `27-03`   |
| 地球惑星科学系   | `1/0-901-311400-0-0` | `27-04`   |
| 機械系           | `2/0-902-321500-0-0` | `27-05`   |
| システム制御系   | `2/0-902-321600-0-0` | `27-06`   |
| 電気電子系       | `2/0-902-321700-0-0` | `27-07`   |
| 情報通信系       | `2/0-902-321800-0-0` | `27-08`   |
| 経営工学系       | `2/0-902-321900-0-0` | `27-09`   |
| 材料系           | `3/0-903-332000-0-0` | `27-10`   |
| 応用化学系       | `3/0-903-332100-0-0` | `27-11`   |
| 数理・計算科学系 | `4/0-904-342200-0-0` | `27-12`   |
| 情報工学系       | `4/0-904-342300-0-0` | `27-13`   |
| 生命理工学系     | `5/0-905-352400-0-0` | `27-14`   |
| 建築学系         | `6/0-906-362500-0-0` | `27-15`   |
| 土木・環境工学系 | `6/0-906-362600-0-0` | `27-16`   |
| 融合理工学系     | `6/0-906-362700-0-0` | `27-17`   |

The scope paths shown are from the 2026-admission guide; the list is cohort-versioned subscription data (see product definition), not hard-coded forever. Stored department and recommendation identities include the admission cohort so multiple cohorts can coexist.

**Recommended courses** come from the 付表 in each PDF: 科目区分, 番台, 科目コード **with group suffix** (`CSC.T241.A` → code `CSC.T241`, group `A`), mark (◎ required / ○ recommended), 科目名, 単位 (`2-0-0`), 身に付ける力, 学修内容, 備考 (`A群`, `アントレ対応科目`, …). The group letter matches 教務Web's `A(○)` / `R(◎)` category. Parsed once a year into the subscription repository.

## Bilingual rule

Every text that comes from the school is stored as

```ts
type Lang = "ja" | "en";
type Localized = Partial<Record<Lang, string>>;
```

- Both languages are fetched for everything that is stored (syllabus list and detail with `hl=ja` and `hl=en`; LMS `fullname` split at `/`; kyomu text is `ja`, and its `en` comes from the syllabus join).
- Display picks the school content language (`contentLanguage`, or the app locale when it is `auto`), falls back to the other language (`zh` reads `en` first, as today), never machine-translates.
- Language-neutral facts are stored once, never per language: codes, ids, credits, meeting days/periods, quarters, dates, categories, flags. Room names are `Localized`.
- Enumerations are stored as codes, and the app owns the labels: instruction language `ja | en | ja-en | other`, class format, requirement mark.
- Text the person typed (overrides, notes, personal events) is a plain string and applies to every locale.
- Teacher-written LMS items (assignment and file names) are single-language by nature: stored as a plain string with an optional detected `lang`.

## Records

### Catalog (`school.db`)

```ts
type Quarter = "1Q" | "2Q" | "3Q" | "4Q";
type Credits = { lecture: number; exercise: number; lab: number }; // "2-0-0", "[0-1-0]"; halves occur ("0.5-0.5-0")

/** One part of the syllabus "Day of week/Period (Classrooms)" field. */
type ScheduleEntry =
  | {
      kind: "weekly";
      day: 1 | 2 | 3 | 4 | 5 | 6 | 7;
      periods: [number, number];
      room?: Localized;
    }
  | {
      kind:
        | "intensive"
        | "seminar"
        | "internship"
        | "on-demand"
        | "tba"
        | "registration-only";
      room?: Localized;
    };

/** Person-set dates for intensive courses (集中講義): a range with the same periods, then per-day overrides. */
type IntensiveSchedule = {
  from: string;
  to: string; // ISO dates, inclusive
  periods: [number, number];
  days?: { date: string; periods: [number, number] | null }[]; // null = no class that day
  room?: string;
};

/** One syllabus offering. id = syllabus externalId = LMS idnumber. */
type Offering = {
  id: string; // "202600907"
  year: number;
  quarters: Quarter[]; // "3～4Q" → ["3Q","4Q"]
  code: string; // "LAH.T209"
  classCode?: string; // "53" from LAE.E213-53, 【1】 → "1"
  unitCode?: string; // 6-digit 開講元/科目区分 code from the syllabus path ("0-907-0-110100-0" → 110100 = 文系教養科目; "0-904-342300-0-0" → 342300 = 情報工学系). 教務Web uses the same category codes (resultUnit_{code}).
  title: Localized;
  instructors: Localized[];
  instructionLanguage: "ja" | "en" | "ja-en" | "other";
  format: {
    kinds: ("lecture" | "exercise" | "experiment")[]; // 講義 / 演習 / 実験等
    delivery?: "face-to-face" | "livestream" | "blended" | "other"; // 対面型 / ライブ型 / ブレンド型 / その他
  };
  media: Localized; // "-" in every sampled page; kept raw
  classLabel?: string; // クラス: "E", "A2", "C11", … ("-" → absent)
  credits: Credits;
  schedule: ScheduleEntry[]; // [] when "-"
  academicUnit: Localized; // 開講元
  intensive: boolean; // 集中講義等
  syllabusUrl: string; // canonical, without hl
  sourceUpdated?: string; // ISO date parsed from "Mar 5, 2026" / "2026/03/05"
  fetchedAt: Partial<Record<Lang, string>>;
};

/** Full syllabus text, one row per language. Sections share `key` across languages. */
type OfferingContent = {
  offeringId: string;
  lang: Lang;
  content: SyllabusContent;
};
```

`SyllabusContent` (existing) stays per language. Only opened, planned or registered offerings get content; list rows come from search.

### LMS (`school.db`)

```ts
type LmsCourse = {
  lmsId: number; // 11663
  offeringId?: string; // from idnumber
  shortname: string; // "LAH.T209[2026]"
  title: Localized; // split fullname
  year: number;
  quarters: Quarter[]; // from category
  url: string;
  startsAt?: string;
  endsAt?: string;
};
```

Assignments keep their current workspace shape, with `dueAt` taken from the event's `timesort` (Unix) and a stable `source.externalId` = module id (`mod/assign/view.php?id=…`). Material metadata keeps the module id in `school.db`; attachment bytes are downloaded on demand under the separate [course-file storage contract](course-file-storage.md).

### Registration, grades, requirements (`school.db`, rebuildable school cache)

```ts
type RequirementMark =
  "required" /* ◎ */ | "recommended" /* ○ */ | "elective" /* 選択 */ | "other";

type Registration = {
  year: number;
  quarters: Quarter[];
  code: string;
  classCode?: string;
  offeringId?: string;
  category: string; // letter as shown: "A" | "B" | "C" | "L" | "R" | …
  mark: RequirementMark;
  credits: Credits;
  declaredOnly: boolean; // "申告専用"
  fetchedAt: string;
};
type RegistrationState = {
  year: number;
  term: "1-2Q" | "3-4Q";
  state:
    | "saved"
    | "needs-check"
    | "registrable"
    | "needs-approval"
    | "rejected"
    | "registered";
  savedAt?: string;
  fetchedAt: string;
};

type Grade = {
  year: number;
  quarter?: Quarter;
  code: string;
  offeringId?: string;
  title: Localized;
  credits: number;
  result: string; // score or pass/fail as shown
  category: Localized; // the h3 heading that precedes the grid (文系教養科目, 英語科目, 専門科目, …)
};
type RequirementProgress = {
  group: "liberal-arts" | "major";
  category: Localized; // 文系教養, 英語, 理工系教養, … (ja from kyomu; en from app labels)
  required?: number;
  earned: number;
  fetchedAt: string;
};
```

### Person's own tables (replacing the old `Workspace` arrays)

- `timetable_courses` (was `Course`): `offering_id?`, `names` (`title`/`instructor` as `Localized`, a snapshot so the timetable renders without the catalog), plain `title`/`instructor` for manual courses, `instruction_language` code, `credits` total plus `credit_split`, `meetings`, `color`. `course-language.ts` and `syllabusSource` go away.
- `course_overrides`: single-string `title`/`instructor`/`color`, winning in every locale.
- `memberships`: `status` is evidence-based: `declared` (教務Web saved/checked), `registered` (本登録), `lms-member`, `completed` (grade exists), `withdrawn`; plus the evidence source.
- `plan_items` (was `DraftSelection`): `term`, `offering_id` or `course_id`, `state: candidate | planned`. Registration is not a plan state; it comes from 教務Web.
- `settings`: key/value rows for the current `Settings` object; `academic_profile` likewise.

## Refresh rules

- Syllabus list: per year/quarter search in both languages; merge by id; 7-day freshness. Content: on open/plan, both languages, 7-day freshness.
- LMS: at launch and on LMS visit, through the APIs above; enrolled-course list paginated to the end before any tombstoning. HTML pages only on demand (submission detail, grades, folder files).
- 教務Web: registration on visit to Settings/Plan during 履修申告 periods; grades/requirements on demand. Read-only GETs; the app never presses a registration button.
- Any source failure keeps the last good rows (existing staging rules in `docs/data-architecture.md`).
