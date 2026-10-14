# Task 2 — Syllabus catalog, both languages

## Goal

Fill `offerings`, `offering_content` and `offerings_fts` (task 1) from the public syllabus in **both** languages, parsed with CSS selectors, and move the course catalog UI onto `catalog_search` / `catalog_offering`.

## Today

`crates/isct-core/src/syllabus.rs` fetches one language per call (`hl=en` for search), parses search rows with `scraper` but syllabus content with regexes. `src/cache/syllabus-*.ts` and `src/core/syllabus-content.ts` cache results in localStorage (30 entries) and `src/cache/syllabus-languages.ts` fetches the other language in the background. Research notes: `docs/syllabus-connector-research.md`.

## Transport (verified 2026-09-28)

- Host `https://syllabus.s.isct.ac.jp`, `GET` only, HTML, no JSON API.
- English: `?hl=en`. Japanese: **the URL without `hl`**. `?hl=ja` answers `302` to the URL without `hl`; follow redirects only when the target stays on the same host and path prefix.
- Search: `GET /search?q=&year=&teacher=&name=&code=&group=100…600` (global) or `GET /courses/{year}/{scope}/search?…` (department scope). At most 100 rows per response, no pagination. Detail: `GET /courses/{year}/{dept}/{path}/{id}`.
- Keep the existing request discipline (bounded body size and time, low concurrency, no cookies).

## Verified structure (2026-09-29)

`scripts/research/syllabus_sample.py` sampled 100 random 2026 offerings from 2,157 candidates, both languages (200 pages). Re-run it after any site change.

- Metadata block: all 12 `dt` labels present, in the same order, on 100/100 pages in both languages.
- Credits: 3 spans on every page; values can be halves (`0.5 / 0.5 / 0`).
- Schedule: 200/200 parse with the grammar below; en and ja give identical structure (only room names differ by language).
- Format: 14 distinct values, all covered by the mapping below. Media: always `-`.
- Sections: 79/100 have the 12 standard sections; 21 (seminar, internship and some project courses) have none. Optional sections pair 1:1 across languages. One page had `その他` in Japanese only, so sections stay per language.
- Competencies: `ul.c-check-list` with 5 or 6 items, or absent.

## Selectors

### Search results

| Field | Selector | Notes |
|---|---|---|
| Result table | `table.c-table.is-table-style1` | Header row `thead th`: code, title, instructors, academic unit, offered (quarter), updated. |
| Row | `table.c-table tbody tr` | |
| Code | `td:nth-child(1)` | `CSC.T354` |
| Title + link | `td:nth-child(2) a[href*="/courses/"]` | Identity from `href`: `/courses/{year}/…/{id}`; strip `hl`. |
| Instructors | `td:nth-child(3)` | Names separated by ` / `; split and trim. |
| Academic unit | `td:nth-child(4)` | Localized text. |
| Offered | `td:nth-child(5)` | `2Q`, `1-2Q`, `3-4Q` (regex leaf). |
| Updated | `td:nth-child(6)` | `Apr 14, 2026` / `2026年4月14日`. |
| Count | `p.c-item-amount__number` | `100 items / out of 117` / `100 件 / 全 110 件` → `{returned, total, truncated}`. |

Run each search in both languages and merge rows by id; a row missing in one language keeps the language it has.

### Detail

| Field | Selector | Notes |
|---|---|---|
| Title | `h1.c-h1` | Trim. |
| Metadata items | `dl.c-dl-2col > div.c-dl-2col__item` → `dt`, `dd` | Map `dt` text through the label table below; keep unknown labels as `metadata` rows. |
| Credits | `dd.is-dl-number-of-credits > span` | Three spans → `{lecture, exercise, lab}`; numbers may be decimal (`0.5`); never concatenate (copy-paste shows `210` for `2 1 0`). |
| Sections | `h3.c-h3` | The section body is every following sibling until the next `h3.c-h3` or `h2`. |
| Paragraph | `p.c-p` | `<br>` → newline. |
| Lists | `ul.c-ul > li`, `ul.c-summary-group li` (related courses, links) | |
| Competencies | `ul.c-check-list > li` | Selected when `li.is-check-list-checked`. |
| Lecture plan | `table#lecture_plans tbody tr` | `td:nth-child(1)` label (`Class 1` / `第1回`), `td:nth-child(2)` topic, `td:nth-child(3)` objective. |
| Links | `a[href]` inside a section | Keep `http(s)` only, no credentials (existing rule). |

Metadata and section headings have no ids or classes that identify them, so these two are matched by label (the one text-based exception). Use this dictionary; unknown headings get `key = "section-{n}"` and keep their heading text.

| Key | English `dt` | Japanese `dt` |
|---|---|---|
| `academicUnit` | Academic unit or major | 開講元 |
| `instructors` | Instructor(s) | 担当教員 |
| `format` | Class Format | 授業形態 |
| `media` | Media-enhanced courses | メディア利用科目 |
| `schedule` | Day of week/Period (Classrooms) | 曜日・時限 (講義室) |
| `class` | Class | クラス |
| `code` | Course Code | 科目コード |
| `credits` | Number of credits | 単位数 |
| `year` | Course offered | 開講時期 |
| `quarter` | Offered quarter | 開講クォーター |
| `updated` | Syllabus updated | シラバス更新日 |
| `language` | Language | 使用言語 |

| Section key | English `h3` | Japanese `h3` |
|---|---|---|
| `overview` | Course overview and goals | 授業の目的（ねらい）、概要 |
| `aims` | Course description and aims | 到達目標 |
| `practicalExperience` | Student learning outcomes | 実務経験のある教員等による授業科目等 |
| `keywords` | Keywords | キーワード |
| `competencies` | Competencies | 学生が身につける力 |
| `classFlow` | Class flow | 授業の進め方 |
| `lecturePlan` | Course schedule/Objectives | 授業計画・課題 |
| `studyAdvice` | Study advice (preparation and review) | 準備学修(事前学修・復習)等についての指示 |
| `textbooks` | Textbook(s) | 教科書 |
| `references` | Reference books, course materials, etc. | 参考書、講義資料等 |
| `assessment` | Evaluation methods and criteria | 成績評価の方法及び基準 |
| `relatedCourses` | Related courses | 関連する科目 |
| `prerequisites` | Prerequisites | 履修の条件・注意事項 |
| `contact` | Contact information (e-mail and phone) … (prefix) | 連絡先 (メール、電話番号) … (prefix) |
| `officeHours` | Office hours | オフィスアワー |
| `other` | Other | その他 |

Normalize whitespace (including `<br>` inside `dt`) before matching; `contact` matches by prefix. All labels above were seen on the sampled pages. The English label "Student learning outcomes" pairs with the Japanese 実務経験のある教員等による授業科目等 (confirmed position by position on the same offerings); the key name follows the Japanese meaning.

### Leaf values (regex allowed)

**Schedule** (`dd` of 曜日・時限 / Day of week/Period): split on ` / `. For each part, the room is the **outermost** trailing `(…)` group (rooms can contain parentheses: `(W8E-101 (計算機室 E207))`); the rest is either a weekly slot or a fixed kind:

| Part | English | Japanese | Result |
|---|---|---|---|
| Weekly | `3-4 Tue` | `火3-4` | `{kind:"weekly", day, periods:[3,4], room}` |
| Intensive | `Intensive` | `集中講義等` | `intensive` |
| Seminar / project | `Seminar` | `特課PJ` | `seminar` |
| Internship | `Internship` | `インターンシップ` | `internship` |
| On-demand | `On-demand` | `オンデマンド` | `on-demand` |
| To be announced | `To be announced` | `未定` | `tba` |
| Registration only | `Only Registration` | `申告専用` | `registration-only` |

`-` means no schedule. Anything else fails the whole field (store raw, flag the offering) rather than guessing. Clock times come from the existing period table.

**Format** (`授業形態` / Class Format): `kinds` from `講義`/`Lecture`, `演習`/`Exercise`, `実験等`/`Experiment` joined by `/`; optional delivery in parentheses: `対面型`/`Face-to-face`, `ライブ型`/`Livestream`, `ブレンド型`/`Blended`, `その他`/`Others`; none when absent.

**Others:** quarter `2Q`, `1-2Q`, `3～4Q` → quarters; year `2026` / `2026年度`; updated `Apr 14, 2026` / `2026年4月14日` / `-`; language `Japanese`/`日本語` → `ja`, `English`/`英語` → `en` (only these two seen); class `-` → absent, else the raw label.

## Build

1. Rewrite the parsers in `crates/isct-core/src/syllabus.rs` with the selectors above; remove the regex document parsing.
2. `sync_offering(id)` fetches detail in both languages, merges into one `Offering` (neutral fields must agree; if they disagree keep the English page's value and record a warning) and two `offering_content` rows, then updates FTS.
3. `catalog_search` first answers from the database; a live search runs both languages, upserts the rows (without content), and returns merged results with `{returned, total, truncated}`.
5. Frontend: the course catalog and course drawer read `catalog_*` commands and render `Localized` with `pickLocalized`. Delete `src/cache/syllabus-catalog.ts`, `src/cache/syllabus-languages.ts` and the localStorage part of `src/core/syllabus-content.ts`.
5. Content freshness 7 days; list rows refresh on search.

## Department sync (for the course planner)

Search results have no day/period, so slot search needs every offering's detail. `sync_department(year, scopePath, quarter?)`:

1. List the department scope page `GET /courses/{year}/{scope}?hl=en` (and the Japanese page) — it lists the department's offerings; if the list reports truncation, split by `group` 100–600.
2. Also sync the liberal-arts units the planner searches without a department filter (codes `LAH`, `LAE`, `LAS`, `LAW`, `LAL`, `ENT`, `XCO`; search by `code` per series).
3. Fetch details for offerings not yet stored or older than 7 days, both languages, concurrency 2, ~0.4 s spacing; progress is reported to the UI; the planner works on what is already stored.
4. Unchanged pages (same hash, task 1) do not rewrite rows.

`catalog_search` then supports `{year, quarter, day, period, departmentScope?, codePrefix?}` from stored `schedule` entries.

## Fixtures

Save the public pages as fixtures (`backend/tests/fixtures/syllabus/`): one search result page and one detail page per language, e.g. `…/courses/2026/4/0-904-342300-0-0/202602440` with and without `?hl=en`. Public pages contain instructor names already published by the school; keep only these few pages.

## Acceptance

- Parser tests for both languages produce the same keys, credits `{1,1,0}`, two meetings (Tue and Fri 3-4, W9-323), 12 metadata fields, the competency flags, and lecture plan rows.
- Merge test: one `Offering` with `title.ja = 並列プログラミング`, `title.en = Parallel Programming`.
- An unknown heading survives as `section-{n}`; a login or error page is rejected, not stored.
- FTS finds the offering by `並列` and by `Parallel`.
