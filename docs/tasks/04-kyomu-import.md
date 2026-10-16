# Task 4 — 教務Web registration, grades, requirements

## Goal

Import the registration list and state, grades, and graduation-requirement progress from 教務Webシステム into `registrations`, `registration_states`, `grades`, `requirement_progress` (task 1), and update `memberships` (`declared` / `registered` / `completed`). Read-only; Japanese only.

## Transport (verified 2026-09-28)

- ASP.NET WebForms on `https://kyomu0.gakumu.titech.ac.jp/Titech/`. No JSON endpoints; `__doPostBack` + ViewState. HTML only.
- Entry: the session is established by loading `/Titech/Default.aspx` (reached from the signed-in Tokyo Tech portal; it redirects to `/Titech/Student/`). Opening a sub-page first bounces to the portal resource list.
- Pages (plain `GET` after entry; paths are URL-encoded Japanese):
  - Registration: `/Titech/Student/科目申告/PID1_0.aspx`
  - Grades: `/Titech/Student/成績閲覧/PID2_0.aspx`
  - Requirements: `/Titech/Student/成績閲覧/PID2_6.aspx`
  - GPA・GPT: `/Titech/Student/成績閲覧/PID2_7.aspx` (see below; Today shows GPA, GPT and earned credits from it).
- **Never** click, submit, or trigger: delete buttons (`a.glyphicon-remove`, `btnDelete_*`), 一時保存 / 申告登録確認, the quarter radio `ctl00$ContentPlaceHolder1$rdoQ` (postback), the `English` link (postback that switches the account language), or the `$.ajax` keep-alive in `/Titech/Scripts/JScript.js` (it re-posts the whole form).
- Pages are Japanese. English titles come from the syllabus (task 2) through the syllabus id or code + year.

Run an app-owned extractor in the school webview on these three paths only (same relay/staging pattern as task 3), after the person opens 教務Web or asks for a refresh. Inject it only when the loaded URL is one of these three paths on `kyomu0.gakumu.titech.ac.jp`; it never runs on other pages.

## Registration — `PID1_0.aspx`

### Whole-list state

| Field     | Selector                                      | Value                                                                                                                                                                                                                                  |
| --------- | --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| State     | `#ctl00_ContentPlaceHolder1_ctl00_lblStatus2` | Text: 一時保存 → `saved`, 要エラーチェック → `needs-check`, 本登録可能 → `registrable`, 承認待ち / 要承認申請 → `needs-approval`, 本登録不可 → `rejected`, 本登録 / 本登録済 → `registered`. Unknown text → keep raw, state `unknown`. |
| Saved at  | `#ctl00_ContentPlaceHolder1_ctl00_lblStatus3` | `(2026/09/28 17:08:44)` → ISO, Asia/Tokyo.                                                                                                                                                                                             |
| Container | `#divSinkokuStatus`                           | Presence check.                                                                                                                                                                                                                        |

### Timetable courses

| Field                  | Selector                                                                                                                                                                                                                                                                                                                           | Notes                                                                                                            |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Course block           | `table#timetable [data-jwc]`                                                                                                                                                                                                                                                                                                       | **`data-jwc` = syllabus id.** The same course appears in several slots; dedupe by `data-jwc`.                    |
| Quarter                | block class `tdQuarter3` / `tdQuarter4` (also `ttQuarter`)                                                                                                                                                                                                                                                                         | One class per quarter the course runs.                                                                           |
| Title                  | `.ttKamokuName a`                                                                                                                                                                                                                                                                                                                  | `href` = `https://syllabus.s.isct.ac.jp/courses?jwc={id}` (cross-check with `data-jwc`).                         |
| Code                   | `.ttSinkokuNo`                                                                                                                                                                                                                                                                                                                     | `CSC.T273`, with class suffix when present (`LAE.E213-53`).                                                      |
| Schedule               | `.ttYoubi`, `.ttYoubi2`                                                                                                                                                                                                                                                                                                            | One line per meeting, `3Q：月3-4` → `{quarter, day, periods}` (regex leaf).                                      |
| Requirement            | `.ttSuisho`                                                                                                                                                                                                                                                                                                                        | `A(○)`, `C(○)`, `L(選択)`, `R(◎)` → `category` = letter, `mark`: `◎` required, `○` recommended, `選択` elective. |
| Instructor             | `.ttKyoin`                                                                                                                                                                                                                                                                                                                         | Text.                                                                                                            |
| Per-course check state | Legend classes on the block or its `td`: `timetableUnchecked` (要エラーチェック), `timetableBunkei` (本登録可能, 文系予約), `timetableAlert22` (本登録可能・承認待ち), `timetableAlert3` (本登録可能・要承認申請), `timetableAlert2` (承認待ち), `timetableAlert1` (要承認申請), `timetableError` (本登録不可); none → 本登録可能. | Only the no-class case was observed live; check both the block and its `td.tdSinkokuCell`.                       |
| Slot                   | Row = period (`#timetable tr`, first cell number), column = weekday (header row `月 火 水 木 金`).                                                                                                                                                                                                                                 | Prefer `.ttYoubi` text; slot position is a cross-check.                                                          |

### Declared-only and intensive lists

| Field         | Selector                                                                                                                   | Notes                                                          |
| ------------- | -------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| Table         | `table[id^="kamokuUnit_"][id$="_gvKamoku"]`                                                                                | Several; a single-cell row 対象データはありません means empty. |
| Row           | `tr` after the header                                                                                                      |                                                                |
| Quarter       | `td.tdQ` (class `tdQuarter{n}`)                                                                                            |                                                                |
| Schedule      | `td.tdYoubi`                                                                                                               | `申告専用` (declaration only) or schedule text.                |
| Title         | `a.kamokuGridKamokuName`                                                                                                   | `href` `…/courses?jwc={id}` → syllabus id.                     |
| Code, credits | `span.resultUnitKamokuCodeSP`, `span.resultUnitKamokuTaniSP`                                                               | `LAE.E213-53`, `[0-1-0]` → `{lecture, exercise, lab}`.         |
| Other columns | Resolve the column index from header `th` text: `番台`, `単位`, `授業担当教員`, `備考` (the requirement mark, e.g. `R(◎)`) | Header labels are the only structure (text exception).         |

`declaredOnly = true` when the schedule is `申告専用`.

## Grades — `PID2_0.aspx`

The observed undergraduate page and the graduate structure documented in the
official student manual are separate supported shapes. The graduate fixture is
derived from the public 2017 manual still linked by the university; it is a
structural acceptance sample, not a claim that the current graduate DOM was
verified with an account.

| Field            | Selector                                                                                                         | Notes                                                                                                                                                                                                                                                               |
| ---------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Groups           | Walk `h3[id$="_h3"], table[id$="_grid"]` **in document order**; each grid belongs to the nearest preceding `h3`. | The ids repeat (every heading/grid uses `…resultUnit_110100_h3/_grid`), so never select a group by id. `h3` text is the category (文系教養科目, 英語科目, 理工系教養科目, 第二外国語科目, アントレプレナーシップ科目, 広域教養科目, 専門科目, 横断科目（専門）, …). |
| Category code    | not on this page                                                                                                 | Every heading and grid here reuses `resultUnit_110100` (a page bug). Take codes from `PID2_7.aspx`, where each `h3` id is distinct, matching by heading text.                                                                                                       |
| Recommended mark | `td.resultUnitSuisho`                                                                                            |                                                                                                                                                                                                                                                                     |
| Code             | the `td.mediaPC` right after `td.resultUnitSuisho`                                                               | Also `span.resultUnitKamokuCodeSP` inside `td.resultUnitKamokuNameSP`.                                                                                                                                                                                              |
| Title            | `td.resultUnitKamokuName`                                                                                        | Ignore the duplicate mobile cell `td.resultUnitKamokuNameSP`.                                                                                                                                                                                                       |
| Credits          | `td.resultUnitTani`                                                                                              |                                                                                                                                                                                                                                                                     |
| Result           | `td.resultUnitSeiseki`                                                                                           | Score or pass/fail as shown; store the raw string.                                                                                                                                                                                                                  |
| Quarter          | cell with class `tdQuarter{n}`                                                                                   |                                                                                                                                                                                                                                                                     |
| Completed        | `td.resultUnitYG span.mediaPC`                                                                                   | Year (and quarter) of completion; leaf regex.                                                                                                                                                                                                                       |

Summary tables on the same page (no ids): the tables whose header is `科目区分 | 修得済単位` give earned credits per category; `table.heikinTbl` gives averages (教養科目群, 専門科目群, 全体). Match rows by the first-cell label.

### Graduate flat grade shape

The graduate page has two labelled flat tables, `大学院科目` and `学部科目`.
Both resolve columns by their visible headers: `授業科目名`, `授業担当教員`,
`単位`, `成績`, `修得時期`. The shared table reader expands `rowspan` and
`colspan` and combines multi-row headers before semantic lookup. Both labelled
sections must be present, including an explicit no-data row when a section is
empty; a missing label, partial header or malformed row rejects the complete
page. Course codes are not inferred from names or numbering.

## GPA・GPT — `PID2_7.aspx`

| Field                | Selector                                                                                                                      | Notes                                                                                                                                                                                                                                                                 |
| -------------------- | ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Totals               | `table.tblGPASummary.mediaPC` under `h3` 通算                                                                                 | Row 1 `th`: 全ての科目 / 教養科目群 / 専門科目群. Row 2 `th`: GPA, GPT, 修得済単位, 合格率% for 全ての科目, then GPA, 修得済単位, 合格率% for each group. Row 3 `td.tani` values in the same order; GPT cell is `td[id$="_tdGPT"]`. Ignore the `.mediaSP` duplicates. |
| Academic coefficient | the `table.tblGPASummary` whose first header cell is `th.keisuu` (h3 【参考】学業成績係数)                                    | 通算, 直前N学期通算, 前年度.                                                                                                                                                                                                                                          |
| Per category         | tables after h3 教養科目群 / 専門科目群; rows link `a[id*="rptZengaku1"]`, `a[id*="rptSenmon"]` with `href="#{categoryCode}"` | Columns 科目区分, GPA, 修得済単位, 合格率% (`td.tani`).                                                                                                                                                                                                               |
| Per term             | `table.tblQ` (h3 修得時期ごと)                                                                                                | Two header rows; data rows: `td.tdY` year (only on the first row of a year, carry it down), term cell (`1Q`…`4Q`, 前学期, 後学期, 通年), then GPA / 修得済単位 / 合格率% for all, 教養, 専門 (`td.tani`, `-` = none), last cell 算出月 (`#/#`).                       |
| Categories           | `h3[id$="_h3"]`                                                                                                               | **Distinct ids here:** `resultUnit_{code}_h3` with 110100 文系教養, 110200 英語, 110800 理工系教養, 110300 第二外国語, 110610 アントレプレナーシップ, 110700 広域教養, 120900 専門, 112200 横断（専門）.                                                              |
| Course rows          | `table.tblResultUnit` after each category `h3`                                                                                | Same cells as `PID2_0.aspx` plus `td.resultUnitGP`. Grid ids still repeat, so keep document order. `推奨` can be `*` (no mark).                                                                                                                                       |

Store as `gpa_summary` (overall, per group, GPT, coefficient) and `gpa_by_term` rows; include both in the export.

## Requirements — `PID2_6.aspx`

One or more semantic `table.tblYouken` elements inside `div.tableSet02Tbl`.
The currently documented undergraduate page contains two:

1. **Liberal arts.** Header cells = categories (文系教養, 英語, 理工系教養, 第二外国語, 日本語・日本文化, アントレ, 広域教養, 横断科目, 教養科目群計). For each column take the row labelled `卒業要件` (required) and `修得済単位` (earned). Other rows (`100`, `◎ 立志PJ`, `H`, `特課研所属要件`) are stored as extra requirement rows with their label.
2. **Major.** Columns `付表`, `①必要修得単位`, `専門科目群 計`, `②総修得単位`, `備考`; rows `専門科目`, `◎`, `R※`, `特課研所属要件`, `卒業要件`, `修得済単位`.

Row and column labels are text (the only structure); normalize whitespace and full-width characters before matching. Cells use classes `tdSum`, `tdSum2`, `tdSum4`, `tdSum22` for totals.

Requirement progress is stored exactly as 教務Web reports it. The app does not
calculate graduate completion locally and does not derive requirements from
GPA, grade summaries, course numbers or LMS activity.

## Mapping

- `registrations` rows from the timetable and lists, with `offering_id` from `data-jwc` / `jwc=`; `registration_states` one row per year/term with the whole-list state.
- `memberships`: `declared` for registered courses while the state is not `registered`; `registered` once it is; `completed` for courses that appear in grades; link through `offering_id`, else `year + code + class`.
- `grades` with `category` (`{ja: h3 text}`; `en` from an app label table) and `offering_id` when a syllabus offering with the same code and year exists.
- `requirement_progress` only from the authoritative `PID2_6.aspx` result.
- All of these are included in the cloud export (task 1).

## Fixtures

Synthetic HTML per page reproduces the ids and classes above: two timetable
blocks with `data-jwc`, one declared-only list row, one scoped empty list, three
undergraduate grade groups in `h3`/grid order with repeated ids, and requirement
tables. A redacted manual-derived fixture covers the two graduate flat tables,
multi-row headers, `rowspan` and `colspan`; all names, grades and dates are
invented.

## Acceptance

- Registration: courses deduplicated by `data-jwc`, schedule `3Q：月3-4` parsed, `A(○)` → `{category: "A", mark: "recommended"}`, `[0-1-0]` → `{0,1,0}`, state `一時保存` → `saved` with timestamp.
- Grades: rows land in the category of the preceding `h3` despite repeated ids.
- Requirements: required/earned per category from both tables.
- A portal redirect with an explicit login form is `reconnect-required`; an
  unknown host is an error. An explicit not-offered message is `unavailable`.
  Both preserve existing rows, while `unavailable` records the page capability
  and satisfies refresh freshness.
- Missing collections, an unknown DOM shape, a partial row, or an unscoped
  no-data message is an error and never marks older rows missing.
- A test asserts the extractor contains no `__doPostBack`, `submit(`, `.click(` or `$.ajax` call.
